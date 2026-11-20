//! reminder 业务服务：校验、用户隔离、item 关联检查、状态流转与 Repository 调用。
//!
//! 关键业务规则（见 docs/design/paadm数据表结构设计.md 与 docs/paadm-impl-notes.md）：
//! - content/remind_at 必填；item_id 可选，提供时必须属于当前用户且存在（ITEM_NOT_FOUND）；
//! - status 默认 pending；trigger 把 pending 改为 triggered（triggered 幂等，
//!   cancelled 触发按固定规则返回 INVALID_ARGUMENT）；
//! - cancel 把状态改为 cancelled 并保留记录（重复取消幂等）；
//! - update 只覆盖提供字段，`clear_item_id` 显式解除关联，任何成功修改刷新 updated_at；
//! - 所有读写都按 user_id 隔离，跨用户访问表现为“不存在”；
//! - 删除提醒是物理删除，不存在返回 REMINDER_NOT_FOUND。

use rusqlite::Connection;

use crate::domain::common;
use crate::domain::reminder::{
    self, CreateReminderInput, ListRemindersInput, Reminder, ReminderStatus, UpdateReminderInput,
};
use crate::error::AppError;
use crate::repositories::reminder as reminder_repo;
use crate::repositories::reminder::ReminderFilter;

/// 创建提醒记录（默认 status=pending）。
pub fn create(
    conn: &Connection,
    user_id: &str,
    input: CreateReminderInput,
) -> Result<Reminder, AppError> {
    let user_id = common::validate_id("user_id", user_id)?;
    let content = reminder::validate_content(input.content.as_deref())?;
    let remind_at = reminder::validate_remind_at(input.remind_at.as_deref())?;
    // 关联校验先于写入：不存在/不属于当前用户的 item 不会留下孤立关联。
    let item_id = resolve_item_id(conn, &user_id, input.item_id.as_deref())?;

    let now = common::now_iso();
    let created = Reminder {
        id: uuid::Uuid::new_v4().to_string(),
        user_id,
        item_id,
        content,
        remind_at,
        status: ReminderStatus::Pending,
        created_at: now.clone(),
        updated_at: now,
    };

    reminder_repo::insert(conn, &created)?;
    Ok(created)
}

/// 按 ID 查询当前用户的提醒；不存在返回 REMINDER_NOT_FOUND。
pub fn get(conn: &Connection, user_id: &str, id: &str) -> Result<Reminder, AppError> {
    let user_id = common::validate_id("user_id", user_id)?;
    let id = common::validate_id("id", id)?;
    find_owned(conn, &user_id, &id)
}

/// 查询当前用户的提醒列表。
pub fn list(
    conn: &Connection,
    user_id: &str,
    input: ListRemindersInput,
) -> Result<Vec<Reminder>, AppError> {
    let user_id = common::validate_id("user_id", user_id)?;
    let status = match input.status.as_deref() {
        Some(raw) => Some(ReminderStatus::parse(raw)?),
        None => None,
    };
    let from = match input.from.as_deref() {
        Some(raw) => Some(common::validate_datetime("from", raw)?),
        None => None,
    };
    let to = match input.to.as_deref() {
        Some(raw) => Some(common::validate_datetime("to", raw)?),
        None => None,
    };
    let item_id = match input.item_id.as_deref() {
        Some(raw) => Some(common::validate_id("item_id", raw)?),
        None => None,
    };

    reminder_repo::list(
        conn,
        &user_id,
        &ReminderFilter {
            status,
            from,
            to,
            item_id,
        },
    )
}

/// 修改提醒：未提供字段不覆盖，`clear_item_id` 显式解除关联。
pub fn update(
    conn: &Connection,
    user_id: &str,
    input: UpdateReminderInput,
) -> Result<Reminder, AppError> {
    let user_id = common::validate_id("user_id", user_id)?;
    let id = common::validate_id("id", &input.id)?;
    let mut updated = find_owned(conn, &user_id, &id)?;

    if let Some(raw) = input.content.as_deref() {
        updated.content = common::validate_required_text("content", raw)?;
    }
    if let Some(raw) = input.remind_at.as_deref() {
        updated.remind_at = common::validate_datetime("remind_at", raw)?;
    }
    if input.clear_item_id {
        updated.item_id = None;
    } else if input.item_id.is_some() {
        updated.item_id = resolve_item_id(conn, &user_id, input.item_id.as_deref())?;
    }

    updated.updated_at = common::now_iso();
    let affected = reminder_repo::update(conn, &updated)?;
    if affected == 0 {
        // 读取与写入之间记录被删除：按不存在处理，不假装成功。
        return Err(AppError::reminder_not_found(&id));
    }
    Ok(updated)
}

/// 标记提醒已触发。
///
/// 固定规则：
/// - pending → triggered；
/// - triggered → triggered（幂等，不报错、不重复改写）；
/// - cancelled → INVALID_ARGUMENT（已取消的提醒不允许再触发）。
pub fn trigger(conn: &Connection, user_id: &str, id: &str) -> Result<Reminder, AppError> {
    let user_id = common::validate_id("user_id", user_id)?;
    let id = common::validate_id("id", id)?;
    let mut reminder = find_owned(conn, &user_id, &id)?;

    match reminder.status {
        ReminderStatus::Pending => {
            reminder.status = ReminderStatus::Triggered;
            reminder.updated_at = common::now_iso();
            reminder_repo::update(conn, &reminder)?;
            Ok(reminder)
        }
        ReminderStatus::Triggered => Ok(reminder),
        ReminderStatus::Cancelled => Err(AppError::invalid_argument(
            "a cancelled reminder cannot be triggered",
        )),
    }
}

/// 取消提醒：保留记录，只把状态改为 cancelled（重复取消幂等）。
pub fn cancel(conn: &Connection, user_id: &str, id: &str) -> Result<Reminder, AppError> {
    let user_id = common::validate_id("user_id", user_id)?;
    let id = common::validate_id("id", id)?;
    let mut reminder = find_owned(conn, &user_id, &id)?;

    if reminder.status == ReminderStatus::Cancelled {
        return Ok(reminder);
    }
    reminder.status = ReminderStatus::Cancelled;
    reminder.updated_at = common::now_iso();
    reminder_repo::update(conn, &reminder)?;
    Ok(reminder)
}

/// 删除提醒（物理删除）；不存在返回 REMINDER_NOT_FOUND。
pub fn delete(conn: &Connection, user_id: &str, id: &str) -> Result<String, AppError> {
    let user_id = common::validate_id("user_id", user_id)?;
    let id = common::validate_id("id", id)?;
    let affected = reminder_repo::delete(conn, &user_id, &id)?;
    if affected == 0 {
        return Err(AppError::reminder_not_found(&id));
    }
    Ok(id)
}

/// 读取当前用户拥有的提醒，不存在（或不属于该用户）返回 REMINDER_NOT_FOUND。
fn find_owned(conn: &Connection, user_id: &str, id: &str) -> Result<Reminder, AppError> {
    reminder_repo::get(conn, user_id, id)?.ok_or_else(|| AppError::reminder_not_found(id))
}

/// 校验并解析可选关联事项 ID（必须属于当前用户且存在）。
fn resolve_item_id(
    conn: &Connection,
    user_id: &str,
    item_id: Option<&str>,
) -> Result<Option<String>, AppError> {
    match item_id {
        None => Ok(None),
        Some(raw) => {
            let id = common::validate_id("item_id", raw)?;
            if !reminder_repo::item_exists(conn, user_id, &id)? {
                return Err(AppError::item_not_found(&id));
            }
            Ok(Some(id))
        }
    }
}
