//! note 数据访问：参数化 SQL 读写与数据库行 → 领域对象。
//!
//! 约定（见 `docs/design/paadm数据表结构设计.md`）：
//! - 所有用户输入都通过参数绑定传入，绝不拼接进 SQL 字符串；
//! - **所有查询 / 更新 / 删除都必须同时按 `user_id` 过滤**，跨用户访问表现为“不存在”；
//! - 列表排序固定为 `created_at ASC, id ASC`；
//! - 关键词按 `title` / `content` 的包含关系匹配，`%`、`_` 按字面量处理；
//! - 创建时间范围用 `julianday(created_at)` 比较，正确处理时区偏移。

use rusqlite::{Connection, Row, params, params_from_iter, types::Value};

use crate::domain::note::{Note, NoteQuery};
use crate::error::AppError;

/// 查询列（顺序与 [`row_to_note`] 一致）。
const COLUMNS: &str = "id, user_id, title, content, created_at, updated_at";

/// 插入一条笔记。
pub fn insert(conn: &Connection, note: &Note) -> Result<(), AppError> {
    conn.execute(
        "INSERT INTO notes (id, user_id, title, content, created_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            note.id,
            note.user_id,
            note.title,
            note.content,
            note.created_at,
            note.updated_at
        ],
    )?;
    Ok(())
}

/// 按用户 + ID 查询；不存在或不属于该用户时返回 `None`。
pub fn get_by_id(conn: &Connection, user_id: &str, id: &str) -> Result<Option<Note>, AppError> {
    let sql = format!("SELECT {COLUMNS} FROM notes WHERE user_id = ?1 AND id = ?2");
    let mut stmt = conn.prepare(&sql)?;
    let mut rows = stmt.query(params![user_id, id])?;
    match rows.next()? {
        Some(row) => Ok(Some(row_to_note(row)?)),
        None => Ok(None),
    }
}

/// 按用户 + 条件查询笔记列表（稳定排序：created_at ASC, id ASC）。
pub fn list(conn: &Connection, user_id: &str, query: &NoteQuery) -> Result<Vec<Note>, AppError> {
    let mut sql = format!("SELECT {COLUMNS} FROM notes WHERE user_id = ?");
    let mut args: Vec<Value> = vec![Value::Text(user_id.to_string())];

    if let Some(keyword) = &query.keyword {
        // 同一个模式绑定两次：title 与 content 各一次。
        let pattern = like_pattern(keyword);
        sql.push_str(" AND (title LIKE ? ESCAPE '\\' OR content LIKE ? ESCAPE '\\')");
        args.push(Value::Text(pattern.clone()));
        args.push(Value::Text(pattern));
    }

    if let Some(from) = &query.from {
        sql.push_str(" AND julianday(created_at) >= julianday(?)");
        args.push(Value::Text(from.clone()));
    }

    if let Some(to) = &query.to {
        sql.push_str(" AND julianday(created_at) <= julianday(?)");
        args.push(Value::Text(to.clone()));
    }

    sql.push_str(" ORDER BY created_at ASC, id ASC");

    let mut stmt = conn.prepare(&sql)?;
    let mut rows = stmt.query(params_from_iter(args.iter()))?;

    let mut notes = Vec::new();
    while let Some(row) = rows.next()? {
        notes.push(row_to_note(row)?);
    }
    Ok(notes)
}

/// 用给定实体整体覆盖一条已有记录（按 user_id + id 定位）；返回是否命中。
pub fn update(conn: &Connection, note: &Note) -> Result<bool, AppError> {
    let affected = conn.execute(
        "UPDATE notes
         SET title = ?1, content = ?2, updated_at = ?3
         WHERE user_id = ?4 AND id = ?5",
        params![
            note.title,
            note.content,
            note.updated_at,
            note.user_id,
            note.id
        ],
    )?;
    Ok(affected > 0)
}

/// 按用户 + ID 删除；返回是否命中。
pub fn delete(conn: &Connection, user_id: &str, id: &str) -> Result<bool, AppError> {
    let affected = conn.execute(
        "DELETE FROM notes WHERE user_id = ?1 AND id = ?2",
        params![user_id, id],
    )?;
    Ok(affected > 0)
}

/// 构造 LIKE 包含匹配模式，并把 `%` / `_` / `\` 转义为字面量。
fn like_pattern(keyword: &str) -> String {
    let mut pattern = String::with_capacity(keyword.len() + 2);
    pattern.push('%');
    for ch in keyword.chars() {
        if matches!(ch, '\\' | '%' | '_') {
            pattern.push('\\');
        }
        pattern.push(ch);
    }
    pattern.push('%');
    pattern
}

/// 数据库行 → 领域对象。
fn row_to_note(row: &Row<'_>) -> rusqlite::Result<Note> {
    Ok(Note {
        id: row.get(0)?,
        user_id: row.get(1)?,
        title: row.get(2)?,
        content: row.get(3)?,
        created_at: row.get(4)?,
        updated_at: row.get(5)?,
    })
}
