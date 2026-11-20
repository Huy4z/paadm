//! reminder 数据访问：参数化 SQL 读写与数据库行 → 领域对象。
//!
//! 关键点（见 docs/paadm-impl-notes.md 第 2 节）：
//! - 所有查询/更新/删除都必须同时按 `user_id` 过滤，不允许只按 id 操作；
//! - 时间只保存一份 ISO 8601 字符串（无 `*_utc` 列）；范围比较/排序用
//!   `julianday(col)` 处理时区偏移；
//! - 列表固定排序 `julianday(remind_at) ASC, id ASC`；
//! - 关联校验显式查询 `items` 表并限定当前用户，不依赖外键报错。

use rusqlite::{Connection, OptionalExtension, Row, params};

use crate::domain::reminder::{Reminder, ReminderStatus};
use crate::error::AppError;

/// 列表查询条件。
#[derive(Debug, Clone, Default)]
pub struct ReminderFilter {
    pub status: Option<ReminderStatus>,
    /// 范围起点（原始时间串，SQL 中用 julianday 比较，含边界）。
    pub from: Option<String>,
    /// 范围终点（原始时间串，SQL 中用 julianday 比较，含边界）。
    pub to: Option<String>,
    pub item_id: Option<String>,
}

/// 查询列（与 [`row_to_reminder`] 的字段保持一致）。
const SELECT_COLUMNS: &str =
    "id, user_id, item_id, content, remind_at, status, created_at, updated_at";

/// 插入一条提醒记录。
pub fn insert(conn: &Connection, reminder: &Reminder) -> Result<(), AppError> {
    conn.execute(
        "INSERT INTO reminders (
            id, user_id, item_id, content, remind_at, status, created_at, updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            reminder.id,
            reminder.user_id,
            reminder.item_id,
            reminder.content,
            reminder.remind_at,
            reminder.status.as_str(),
            reminder.created_at,
            reminder.updated_at,
        ],
    )?;
    Ok(())
}

/// 按 (user_id, id) 查询提醒；不存在（或属于其他用户）返回 `Ok(None)`。
pub fn get(conn: &Connection, user_id: &str, id: &str) -> Result<Option<Reminder>, AppError> {
    let sql = format!("SELECT {SELECT_COLUMNS} FROM reminders WHERE user_id = ?1 AND id = ?2");
    let reminder = conn
        .query_row(&sql, params![user_id, id], row_to_reminder)
        .optional()?;
    Ok(reminder)
}

/// 查询提醒列表（静态参数化 SQL，各条件为 `NULL` 时视为不筛选）。
pub fn list(
    conn: &Connection,
    user_id: &str,
    filter: &ReminderFilter,
) -> Result<Vec<Reminder>, AppError> {
    let sql = format!(
        "SELECT {SELECT_COLUMNS} FROM reminders
         WHERE user_id = ?1
           AND (?2 IS NULL OR status = ?2)
           AND (?3 IS NULL OR julianday(remind_at) >= julianday(?3))
           AND (?4 IS NULL OR julianday(remind_at) <= julianday(?4))
           AND (?5 IS NULL OR item_id = ?5)
         ORDER BY julianday(remind_at) ASC, id ASC"
    );
    let status = filter.status.map(ReminderStatus::as_str);
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(
        params![
            user_id,
            status,
            filter.from.as_deref(),
            filter.to.as_deref(),
            filter.item_id.as_deref(),
        ],
        row_to_reminder,
    )?;

    let mut reminders = Vec::new();
    for row in rows {
        reminders.push(row?);
    }
    Ok(reminders)
}

/// 用给定实体整体覆盖一条已有记录（调用方负责先确认存在并完成校验）。
///
/// 返回受影响的行数，便于上层区分“记录不存在”。
pub fn update(conn: &Connection, reminder: &Reminder) -> Result<usize, AppError> {
    let affected = conn.execute(
        "UPDATE reminders
         SET item_id = ?3,
             content = ?4,
             remind_at = ?5,
             status = ?6,
             updated_at = ?7
         WHERE user_id = ?1 AND id = ?2",
        params![
            reminder.user_id,
            reminder.id,
            reminder.item_id,
            reminder.content,
            reminder.remind_at,
            reminder.status.as_str(),
            reminder.updated_at,
        ],
    )?;
    Ok(affected)
}

/// 删除一条提醒记录，返回受影响的行数。
pub fn delete(conn: &Connection, user_id: &str, id: &str) -> Result<usize, AppError> {
    let affected = conn.execute(
        "DELETE FROM reminders WHERE user_id = ?1 AND id = ?2",
        params![user_id, id],
    )?;
    Ok(affected)
}

/// 事项是否存在且属于指定用户。
///
/// 关联校验必须同时校验归属（复合外键 (user_id, item_id) 的语义），
/// 不存在或属于他人时统一返回 `false`，由服务层转成 ITEM_NOT_FOUND。
pub fn item_exists(conn: &Connection, user_id: &str, item_id: &str) -> Result<bool, AppError> {
    let found: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM items WHERE user_id = ?1 AND id = ?2",
            params![user_id, item_id],
            |row| row.get(0),
        )
        .optional()?;
    Ok(found.is_some())
}

/// 数据库行 → 领域对象。
fn row_to_reminder(row: &Row<'_>) -> rusqlite::Result<Reminder> {
    let status_raw: String = row.get("status")?;
    let status = ReminderStatus::from_db_str(&status_raw).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "reminders.status contains an unexpected value",
            )),
        )
    })?;

    Ok(Reminder {
        id: row.get("id")?,
        user_id: row.get("user_id")?,
        item_id: row.get("item_id")?,
        content: row.get("content")?,
        remind_at: row.get("remind_at")?,
        status,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
    })
}
