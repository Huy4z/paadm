//! Note 业务服务：执行领域校验、协调 repository，并按用户隔离数据。
//!
//! 表结构以 `docs/design/paadm数据表结构设计.md` 为唯一权威来源：
//! 所有操作都同时带上 `user_id`，跨用户访问统一表现为 `NOTE_NOT_FOUND`。
//!
//! 业务规则：
//! - 创建：正文必填，标题可选；
//! - 查询 / 修改 / 删除：记录不存在或不属于当前用户返回 `NOTE_NOT_FOUND`；
//! - 修改：只覆盖提供的字段，刷新 `updated_at`。

use rusqlite::Connection;
use uuid::Uuid;

use crate::domain::common;
use crate::domain::note::{NewNote, Note, NoteChanges, NoteQuery};
use crate::error::AppError;
use crate::repositories::note as repo;

/// 创建笔记，记录归属 `user_id`。
pub fn create(
    conn: &Connection,
    user_id: &str,
    title: Option<&str>,
    content: Option<&str>,
) -> Result<Note, AppError> {
    let input = NewNote::parse(title, content)?;
    let now = common::now_iso();

    let note = Note {
        id: Uuid::new_v4().to_string(),
        user_id: user_id.to_string(),
        title: input.title,
        content: input.content,
        created_at: now.clone(),
        updated_at: now,
    };

    repo::insert(conn, &note)?;
    Ok(note)
}

/// 按 ID 查询当前用户的笔记；不存在或不属于该用户返回 `NOTE_NOT_FOUND`。
pub fn get(conn: &Connection, user_id: &str, id: &str) -> Result<Note, AppError> {
    let id = common::validate_id("id", id)?;
    repo::get_by_id(conn, user_id, &id)?.ok_or_else(|| AppError::note_not_found(&id))
}

/// 按关键词 / 创建时间范围查询当前用户的笔记列表（排序 `created_at ASC, id ASC`）。
pub fn list(
    conn: &Connection,
    user_id: &str,
    keyword: Option<&str>,
    from: Option<&str>,
    to: Option<&str>,
) -> Result<Vec<Note>, AppError> {
    let query = NoteQuery::parse(keyword, from, to)?;
    repo::list(conn, user_id, &query)
}

/// 修改当前用户的笔记：未提供的字段保持不变，`clear_title` 显式清空，刷新 `updated_at`。
pub fn update(
    conn: &Connection,
    user_id: &str,
    id: &str,
    title: Option<&str>,
    content: Option<&str>,
    clear_title: bool,
) -> Result<Note, AppError> {
    let id = common::validate_id("id", id)?;
    let changes = NoteChanges::parse(title, content, clear_title)?;

    let current =
        repo::get_by_id(conn, user_id, &id)?.ok_or_else(|| AppError::note_not_found(&id))?;
    let updated = changes.apply(&current, common::now_iso());
    repo::update(conn, &updated)?;
    Ok(updated)
}

/// 删除当前用户的笔记；不存在或不属于该用户返回 `NOTE_NOT_FOUND`。
pub fn delete(conn: &Connection, user_id: &str, id: &str) -> Result<(), AppError> {
    let id = common::validate_id("id", id)?;
    if !repo::delete(conn, user_id, &id)? {
        return Err(AppError::note_not_found(&id));
    }
    Ok(())
}
