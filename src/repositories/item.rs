//! item 数据访问：`items` 表参数化 SQL、数据库记录 → 领域对象。
//!
//! 关键约定（见权威数据表设计与 impl-notes）：
//! - **所有查询/更新/删除都必须同时按 `user_id` 过滤**，跨用户访问表现为“不存在”；
//! - 时间范围与排序统一用 SQLite 的 `julianday()` 处理时区偏移；
//! - 删除事项必须在**同一事务**中先解除 `reminders.item_id` 关联、再删除 `items`
//!   （复合外键是 `ON DELETE RESTRICT`；不要依赖 `SET NULL`，SQLite 复合外键置空会
//!   连 `user_id` 一起置空）。

use rusqlite::types::Value;
use rusqlite::{Connection, OptionalExtension, Row, params, params_from_iter};

use crate::domain::item::{Item, ItemStatus, ItemType};
use crate::error::AppError;

/// 待办列表过滤条件（时间范围为原始 ISO 8601 字符串，由 SQL 用 julianday 比较）。
#[derive(Debug, Clone, Default)]
pub struct TaskFilter {
    pub status: Option<ItemStatus>,
    pub priority: Option<i64>,
    pub due_from: Option<String>,
    pub due_to: Option<String>,
    pub keyword: Option<String>,
}

/// 日程列表过滤条件；区间相交规则固定为 `[from, to)`。
#[derive(Debug, Clone, Default)]
pub struct EventFilter {
    pub from: Option<String>,
    pub to: Option<String>,
    pub keyword: Option<String>,
}

/// 查询列顺序必须与 [`RawItem::from_row`] 一致。
const SELECT_COLUMNS: &str = "id, user_id, title, description, type, status, priority, \
     start_at, end_at, due_at, all_day, location, created_at, updated_at, completed_at";

/// 插入一条事项。
pub fn insert(conn: &Connection, item: &Item) -> Result<(), AppError> {
    conn.execute(
        "INSERT INTO items (
            id, user_id, title, description, type, status, priority,
            start_at, end_at, due_at, all_day, location,
            created_at, updated_at, completed_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
        params![
            item.id,
            item.user_id,
            item.title,
            item.description,
            item.item_type.as_str(),
            item.status.as_str(),
            item.priority,
            item.start_at,
            item.end_at,
            item.due_at,
            all_day_as_int(item.all_day),
            item.location,
            item.created_at,
            item.updated_at,
            item.completed_at,
        ],
    )
    .map_err(AppError::from_sqlite)?;
    Ok(())
}

/// 按（用户, ID, 类型）查询单条事项；不存在或类型不匹配返回 `None`。
pub fn get(
    conn: &Connection,
    user_id: &str,
    id: &str,
    item_type: ItemType,
) -> Result<Option<Item>, AppError> {
    let sql =
        format!("SELECT {SELECT_COLUMNS} FROM items WHERE user_id = ?1 AND id = ?2 AND type = ?3");
    let raw = conn
        .query_row(
            &sql,
            params![user_id, id, item_type.as_str()],
            RawItem::from_row,
        )
        .optional()
        .map_err(AppError::from_sqlite)?;
    raw.map(RawItem::into_item).transpose()
}

/// 全字段更新（调用方已合并出最终实体）；返回是否命中了记录。
pub fn update(conn: &Connection, item: &Item) -> Result<bool, AppError> {
    let changed = conn
        .execute(
            "UPDATE items SET
                title = ?4, description = ?5, status = ?6, priority = ?7,
                start_at = ?8, end_at = ?9, due_at = ?10, all_day = ?11, location = ?12,
                created_at = ?13, updated_at = ?14, completed_at = ?15
             WHERE user_id = ?1 AND id = ?2 AND type = ?3",
            params![
                item.user_id,
                item.id,
                item.item_type.as_str(),
                item.title,
                item.description,
                item.status.as_str(),
                item.priority,
                item.start_at,
                item.end_at,
                item.due_at,
                all_day_as_int(item.all_day),
                item.location,
                item.created_at,
                item.updated_at,
                item.completed_at,
            ],
        )
        .map_err(AppError::from_sqlite)?;
    Ok(changed == 1)
}

/// 按条件列出待办（`type='task'`），排序 `created_at ASC, id ASC`。
pub fn list_tasks(
    conn: &Connection,
    user_id: &str,
    filter: &TaskFilter,
) -> Result<Vec<Item>, AppError> {
    let mut sql = format!("SELECT {SELECT_COLUMNS} FROM items WHERE user_id = ? AND type = 'task'");
    let mut values: Vec<Value> = vec![Value::Text(user_id.to_string())];

    if let Some(status) = filter.status {
        sql.push_str(" AND status = ?");
        values.push(Value::Text(status.as_str().to_string()));
    }
    if let Some(priority) = filter.priority {
        sql.push_str(" AND priority = ?");
        values.push(Value::Integer(priority));
    }
    if let Some(due_from) = &filter.due_from {
        sql.push_str(" AND due_at IS NOT NULL AND julianday(due_at) >= julianday(?)");
        values.push(Value::Text(due_from.clone()));
    }
    if let Some(due_to) = &filter.due_to {
        sql.push_str(" AND due_at IS NOT NULL AND julianday(due_at) <= julianday(?)");
        values.push(Value::Text(due_to.clone()));
    }
    if let Some(keyword) = &filter.keyword {
        // 包含匹配，只匹配标题；lower() 让 ASCII 关键词大小写不敏感。
        sql.push_str(" AND instr(lower(title), lower(?)) > 0");
        values.push(Value::Text(keyword.clone()));
    }

    sql.push_str(" ORDER BY created_at ASC, id ASC");
    query_items(conn, &sql, &values)
}

/// 按条件列出日程（`type='event'`），排序 `julianday(start_at) ASC, id ASC`。
///
/// 区间规则固定为 `[from, to)` 相交：
/// `julianday(start_at) < julianday(to)` 且
/// `julianday(COALESCE(end_at, start_at)) > julianday(from)`
/// （没有 end_at 的日程视为开始时刻的瞬时事项）。
pub fn list_events(
    conn: &Connection,
    user_id: &str,
    filter: &EventFilter,
) -> Result<Vec<Item>, AppError> {
    let mut sql =
        format!("SELECT {SELECT_COLUMNS} FROM items WHERE user_id = ? AND type = 'event'");
    let mut values: Vec<Value> = vec![Value::Text(user_id.to_string())];

    if let Some(to) = &filter.to {
        sql.push_str(" AND julianday(start_at) < julianday(?)");
        values.push(Value::Text(to.clone()));
    }
    if let Some(from) = &filter.from {
        sql.push_str(" AND julianday(COALESCE(end_at, start_at)) > julianday(?)");
        values.push(Value::Text(from.clone()));
    }
    if let Some(keyword) = &filter.keyword {
        sql.push_str(" AND instr(lower(title), lower(?)) > 0");
        values.push(Value::Text(keyword.clone()));
    }

    sql.push_str(" ORDER BY julianday(start_at) ASC, id ASC");
    query_items(conn, &sql, &values)
}

/// 删除事项：同一事务内先解除提醒关联，再删除记录；返回是否命中了记录。
///
/// 记录不存在（或类型/用户不匹配）时回滚事务并返回 `false`，由 Service 报
/// TASK_NOT_FOUND / SCHEDULE_NOT_FOUND，且不会误改提醒关联。
pub fn delete(
    conn: &Connection,
    user_id: &str,
    id: &str,
    item_type: ItemType,
) -> Result<bool, AppError> {
    // `Database::connection()` 只有 `&Connection`，使用 unchecked_transaction 开启事务
    // （本进程内没有嵌套事务，安全性等价于 Connection::transaction）。
    let tx = conn
        .unchecked_transaction()
        .map_err(AppError::from_sqlite)?;

    // 复合外键 (user_id, item_id) → items(user_id, id) 是 ON DELETE RESTRICT，
    // 必须先解除关联；这里只把 item_id 置空，保留提醒本身。
    tx.execute(
        "UPDATE reminders SET item_id = NULL WHERE user_id = ?1 AND item_id = ?2",
        params![user_id, id],
    )
    .map_err(AppError::from_sqlite)?;

    let deleted = tx
        .execute(
            "DELETE FROM items WHERE user_id = ?1 AND id = ?2 AND type = ?3",
            params![user_id, id, item_type.as_str()],
        )
        .map_err(AppError::from_sqlite)?;

    if deleted == 0 {
        // 目标不存在/类型不匹配：回滚提醒关联的修改，保持数据一致。
        tx.rollback().map_err(AppError::from_sqlite)?;
        return Ok(false);
    }

    tx.commit().map_err(AppError::from_sqlite)?;
    Ok(true)
}

/// 执行查询并把数据库行转换为领域对象。
fn query_items(conn: &Connection, sql: &str, values: &[Value]) -> Result<Vec<Item>, AppError> {
    let mut statement = conn.prepare(sql).map_err(AppError::from_sqlite)?;
    let rows = statement
        .query_map(params_from_iter(values.iter()), RawItem::from_row)
        .map_err(AppError::from_sqlite)?;

    let mut items = Vec::new();
    for row in rows {
        let raw = row.map_err(AppError::from_sqlite)?;
        items.push(raw.into_item()?);
    }
    Ok(items)
}

/// `all_day` 在数据库中用 INTEGER 0/1 存储。
fn all_day_as_int(all_day: bool) -> i64 {
    i64::from(all_day)
}

/// 数据库行的中间表示：枚举列与布尔列先按原始类型取出，再转换为领域值。
struct RawItem {
    id: String,
    user_id: String,
    title: String,
    description: Option<String>,
    item_type: String,
    status: String,
    priority: Option<i64>,
    start_at: Option<String>,
    end_at: Option<String>,
    due_at: Option<String>,
    all_day: i64,
    location: Option<String>,
    created_at: String,
    updated_at: String,
    completed_at: Option<String>,
}

impl RawItem {
    fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            user_id: row.get(1)?,
            title: row.get(2)?,
            description: row.get(3)?,
            item_type: row.get(4)?,
            status: row.get(5)?,
            priority: row.get(6)?,
            start_at: row.get(7)?,
            end_at: row.get(8)?,
            due_at: row.get(9)?,
            all_day: row.get(10)?,
            location: row.get(11)?,
            created_at: row.get(12)?,
            updated_at: row.get(13)?,
            completed_at: row.get(14)?,
        })
    }

    fn into_item(self) -> Result<Item, AppError> {
        let item_type = ItemType::from_stored(&self.item_type)?;
        let status = ItemStatus::from_stored(&self.status)?;
        let all_day = match self.all_day {
            0 => false,
            1 => true,
            _ => return Err(AppError::internal("stored all_day is invalid")),
        };
        Ok(Item {
            id: self.id,
            user_id: self.user_id,
            title: self.title,
            description: self.description,
            item_type,
            status,
            priority: self.priority,
            start_at: self.start_at,
            end_at: self.end_at,
            due_at: self.due_at,
            all_day,
            location: self.location,
            created_at: self.created_at,
            updated_at: self.updated_at,
            completed_at: self.completed_at,
        })
    }
}
