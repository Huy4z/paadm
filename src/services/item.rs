//! item 业务服务：待办/日程统一业务规则、状态流转、事务删除。
//!
//! 业务规则（见权威数据表设计与 impl-notes 第 4 节）：
//! - task 命令组只操作 `type='task'`，event 命令组只操作 `type='event'`；
//!   按 ID 取/改/删时同时校验类型，不匹配分别返回 TASK_NOT_FOUND / SCHEDULE_NOT_FOUND；
//! - 所有操作按 `user_id` 隔离；
//! - task：`title` 必填、默认 `pending`；`complete` 设置 `completed_at`（重复完成幂等）；
//!   `cancel` 设置 `cancelled` 并清空 `completed_at`；update 只覆盖提供的字段；
//! - event：`start_at` 必填；`end_at` 可选且必须**严格晚于** start_at（按时间点比较）；
//!   `all_day` 只允许 0/1；update 后同样要重新校验起止时间一致性；
//! - 任何成功修改刷新 `updated_at`；删除事项时在同一事务中先解除提醒关联。

use rusqlite::Connection;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use uuid::Uuid;

use crate::domain::common;
use crate::domain::item::{
    CreateEventInput, CreateTaskInput, Item, ItemStatus, ItemType, ListEventsInput, ListTasksInput,
    UpdateEventInput, UpdateTaskInput,
};
use crate::error::AppError;
use crate::repositories::item as item_repo;
use crate::repositories::item::{EventFilter, TaskFilter};

// ---------------------------------------------------------------------------
// 待办（task）
// ---------------------------------------------------------------------------

/// 创建待办（`type='task'`，默认 `status=pending`）。
pub fn create_task(
    conn: &Connection,
    user_id: &str,
    input: CreateTaskInput,
) -> Result<Item, AppError> {
    let user_id = common::validate_id("user-id", user_id)?;
    let title = match input.title.as_deref() {
        Some(value) => common::validate_required_text("title", value)?,
        None => return Err(AppError::invalid_argument("title is required")),
    };
    let description = common::validate_optional_text("description", input.description.as_deref())?;
    let due_at = optional_datetime("due_at", input.due_at.as_deref())?;

    let now = common::now_iso();
    let item = Item {
        id: Uuid::new_v4().to_string(),
        user_id,
        title,
        description,
        item_type: ItemType::Task,
        status: ItemStatus::Pending,
        priority: input.priority,
        start_at: None,
        end_at: None,
        due_at,
        all_day: false,
        location: None,
        created_at: now.clone(),
        updated_at: now,
        completed_at: None,
    };
    item_repo::insert(conn, &item)?;
    Ok(item)
}

/// 按 ID 查询待办；不存在或类型不是 task → `TASK_NOT_FOUND`。
pub fn get_task(conn: &Connection, user_id: &str, id: &str) -> Result<Item, AppError> {
    let user_id = common::validate_id("user-id", user_id)?;
    let id = common::validate_id("id", id)?;
    item_repo::get(conn, &user_id, &id, ItemType::Task)?
        .ok_or_else(|| AppError::task_not_found(&id))
}

/// 按条件查询待办列表（空结果是成功）。
pub fn list_tasks(
    conn: &Connection,
    user_id: &str,
    input: &ListTasksInput,
) -> Result<Vec<Item>, AppError> {
    let user_id = common::validate_id("user-id", user_id)?;
    let status = match input.status.as_deref().map(str::trim) {
        Some(value) if !value.is_empty() => Some(ItemStatus::parse("status", value)?),
        _ => None,
    };
    let due_from = optional_datetime("due-from", input.due_from.as_deref())?;
    let due_to = optional_datetime("due-to", input.due_to.as_deref())?;
    let keyword = trimmed_non_empty(input.keyword.as_deref());

    let filter = TaskFilter {
        status,
        priority: input.priority,
        due_from,
        due_to,
        keyword,
    };
    item_repo::list_tasks(conn, &user_id, &filter)
}

/// 修改待办：只覆盖提供的字段，`clear_*` 显式清空可选字段。
pub fn update_task(
    conn: &Connection,
    user_id: &str,
    input: UpdateTaskInput,
) -> Result<Item, AppError> {
    let user_id = common::validate_id("user-id", user_id)?;
    let id = common::validate_id("id", &input.id)?;
    let mut item = item_repo::get(conn, &user_id, &id, ItemType::Task)?
        .ok_or_else(|| AppError::task_not_found(&id))?;

    if let Some(title) = input.title.as_deref() {
        item.title = common::validate_required_text("title", title)?;
    }

    if input.clear_description {
        item.description = None;
    } else if let Some(description) = input.description.as_deref() {
        item.description = common::validate_optional_text("description", Some(description))?;
    }

    if input.clear_priority {
        item.priority = None;
    } else if input.priority.is_some() {
        item.priority = input.priority;
    }

    if input.clear_due_at {
        item.due_at = None;
    } else if input.due_at.is_some() {
        item.due_at = optional_datetime("due_at", input.due_at.as_deref())?;
    }

    item.updated_at = common::now_iso();
    persist(conn, &item)?;
    Ok(item)
}

/// 标记待办完成：`status=completed` 并设置 `completed_at`；重复完成幂等。
pub fn complete_task(conn: &Connection, user_id: &str, id: &str) -> Result<Item, AppError> {
    let user_id = common::validate_id("user-id", user_id)?;
    let id = common::validate_id("id", id)?;
    let mut item = item_repo::get(conn, &user_id, &id, ItemType::Task)?
        .ok_or_else(|| AppError::task_not_found(&id))?;

    if item.status == ItemStatus::Completed {
        return Ok(item);
    }

    let now = common::now_iso();
    item.status = ItemStatus::Completed;
    item.completed_at = Some(now.clone());
    item.updated_at = now;
    persist(conn, &item)?;
    Ok(item)
}

/// 取消待办：`status=cancelled`，取消不是完成，会清空 `completed_at`；重复取消幂等。
pub fn cancel_task(conn: &Connection, user_id: &str, id: &str) -> Result<Item, AppError> {
    let user_id = common::validate_id("user-id", user_id)?;
    let id = common::validate_id("id", id)?;
    let mut item = item_repo::get(conn, &user_id, &id, ItemType::Task)?
        .ok_or_else(|| AppError::task_not_found(&id))?;

    if item.status == ItemStatus::Cancelled {
        return Ok(item);
    }

    let now = common::now_iso();
    item.status = ItemStatus::Cancelled;
    item.completed_at = None;
    item.updated_at = now;
    persist(conn, &item)?;
    Ok(item)
}

/// 删除待办（同一事务中先解除提醒关联）；不存在 → `TASK_NOT_FOUND`。
pub fn delete_task(conn: &Connection, user_id: &str, id: &str) -> Result<(), AppError> {
    let user_id = common::validate_id("user-id", user_id)?;
    let id = common::validate_id("id", id)?;
    let deleted = item_repo::delete(conn, &user_id, &id, ItemType::Task)?;
    if !deleted {
        return Err(AppError::task_not_found(&id));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 日程（event）
// ---------------------------------------------------------------------------

/// 创建日程（`type='event'`）；`start_at` 必填，`end_at` 必须严格晚于开始时间。
pub fn create_event(
    conn: &Connection,
    user_id: &str,
    input: CreateEventInput,
) -> Result<Item, AppError> {
    let user_id = common::validate_id("user-id", user_id)?;
    let title = match input.title.as_deref() {
        Some(value) => common::validate_required_text("title", value)?,
        None => return Err(AppError::invalid_argument("title is required")),
    };
    let description = common::validate_optional_text("description", input.description.as_deref())?;
    let start_at = match input.start_at.as_deref() {
        Some(value) if !value.trim().is_empty() => common::validate_datetime("start_at", value)?,
        _ => return Err(AppError::invalid_argument("start_at is required")),
    };
    let end_at = optional_datetime("end_at", input.end_at.as_deref())?;
    ensure_end_after_start(&start_at, end_at.as_deref())?;
    let location = common::validate_optional_text("location", input.location.as_deref())?;
    let all_day = optional_all_day(input.all_day)?;

    let now = common::now_iso();
    let item = Item {
        id: Uuid::new_v4().to_string(),
        user_id,
        title,
        description,
        item_type: ItemType::Event,
        status: ItemStatus::Pending,
        priority: None,
        start_at: Some(start_at),
        end_at,
        due_at: None,
        all_day,
        location,
        created_at: now.clone(),
        updated_at: now,
        completed_at: None,
    };
    item_repo::insert(conn, &item)?;
    Ok(item)
}

/// 按 ID 查询日程；不存在或类型不是 event → `SCHEDULE_NOT_FOUND`。
pub fn get_event(conn: &Connection, user_id: &str, id: &str) -> Result<Item, AppError> {
    let user_id = common::validate_id("user-id", user_id)?;
    let id = common::validate_id("id", id)?;
    item_repo::get(conn, &user_id, &id, ItemType::Event)?
        .ok_or_else(|| AppError::schedule_not_found(&id))
}

/// 按条件查询日程列表（`[from, to)` 相交；空结果是成功）。
pub fn list_events(
    conn: &Connection,
    user_id: &str,
    input: &ListEventsInput,
) -> Result<Vec<Item>, AppError> {
    let user_id = common::validate_id("user-id", user_id)?;
    let from = optional_datetime("from", input.from.as_deref())?;
    let to = optional_datetime("to", input.to.as_deref())?;
    let keyword = trimmed_non_empty(input.keyword.as_deref());

    let filter = EventFilter { from, to, keyword };
    item_repo::list_events(conn, &user_id, &filter)
}

/// 修改日程：只覆盖提供的字段，`clear_*` 显式清空；修改后重新校验起止时间一致性。
pub fn update_event(
    conn: &Connection,
    user_id: &str,
    input: UpdateEventInput,
) -> Result<Item, AppError> {
    let user_id = common::validate_id("user-id", user_id)?;
    let id = common::validate_id("id", &input.id)?;
    let mut item = item_repo::get(conn, &user_id, &id, ItemType::Event)?
        .ok_or_else(|| AppError::schedule_not_found(&id))?;

    if let Some(title) = input.title.as_deref() {
        item.title = common::validate_required_text("title", title)?;
    }

    if input.clear_description {
        item.description = None;
    } else if let Some(description) = input.description.as_deref() {
        item.description = common::validate_optional_text("description", Some(description))?;
    }

    if let Some(start_at) = input.start_at.as_deref() {
        item.start_at = Some(common::validate_datetime("start_at", start_at)?);
    }

    if input.clear_end_at {
        item.end_at = None;
    } else if input.end_at.is_some() {
        item.end_at = optional_datetime("end_at", input.end_at.as_deref())?;
    }

    if input.clear_location {
        item.location = None;
    } else if let Some(location) = input.location.as_deref() {
        item.location = common::validate_optional_text("location", Some(location))?;
    }

    if let Some(all_day) = input.all_day {
        item.all_day = common::validate_all_day(all_day)?;
    }

    // 任一或两个时间变更后，都必须按时间点重新校验 end_at > start_at。
    ensure_end_after_start(
        item.start_at.as_deref().unwrap_or_default(),
        item.end_at.as_deref(),
    )?;

    item.updated_at = common::now_iso();
    persist(conn, &item)?;
    Ok(item)
}

/// 删除日程（同一事务中先解除提醒关联）；不存在 → `SCHEDULE_NOT_FOUND`。
pub fn delete_event(conn: &Connection, user_id: &str, id: &str) -> Result<(), AppError> {
    let user_id = common::validate_id("user-id", user_id)?;
    let id = common::validate_id("id", id)?;
    let deleted = item_repo::delete(conn, &user_id, &id, ItemType::Event)?;
    if !deleted {
        return Err(AppError::schedule_not_found(&id));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 内部辅助
// ---------------------------------------------------------------------------

/// 校验可选时间：未提供 / 空串按“无时间”处理；提供时必须带时区偏移。
fn optional_datetime(field: &str, raw: Option<&str>) -> Result<Option<String>, AppError> {
    match raw.map(str::trim) {
        Some(value) if !value.is_empty() => Ok(Some(common::validate_datetime(field, value)?)),
        _ => Ok(None),
    }
}

/// 校验可选 `all_day`：未提供按 false（0）处理。
fn optional_all_day(raw: Option<i64>) -> Result<bool, AppError> {
    match raw {
        Some(value) => common::validate_all_day(value),
        None => Ok(false),
    }
}

/// 去除首尾空白；空串视为未提供。
fn trimmed_non_empty(raw: Option<&str>) -> Option<String> {
    raw.map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

/// 起止时间一致性：`end_at` 若存在必须**严格晚于** `start_at`（按时间点比较）。
fn ensure_end_after_start(start_at: &str, end_at: Option<&str>) -> Result<(), AppError> {
    let Some(end_at) = end_at else {
        return Ok(());
    };
    let start = parse_instant("start_at", start_at)?;
    let end = parse_instant("end_at", end_at)?;
    if end <= start {
        return Err(AppError::invalid_argument(
            "end_at must be later than start_at",
        ));
    }
    Ok(())
}

/// 解析已经过 `validate_datetime` 校验的时间字符串为时间点。
fn parse_instant(field: &str, value: &str) -> Result<OffsetDateTime, AppError> {
    OffsetDateTime::parse(value.trim(), &Rfc3339).map_err(|_| {
        AppError::invalid_argument(format!(
            "{field} must be an ISO 8601 datetime with a timezone offset"
        ))
    })
}

/// 把最终实体写回数据库；记录已不存在时按类型返回对应 NOT_FOUND。
fn persist(conn: &Connection, item: &Item) -> Result<(), AppError> {
    let updated = item_repo::update(conn, item)?;
    if !updated {
        // 读取后记录被并发删除：按“不存在”处理，不报告成功。
        return Err(match item.item_type {
            ItemType::Task => AppError::task_not_found(&item.id),
            ItemType::Event => AppError::schedule_not_found(&item.id),
        });
    }
    Ok(())
}
