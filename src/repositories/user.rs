//! 用户数据访问：users 表的参数化 SQL 读写。
//!
//! 唯一性约束：`UNIQUE (provider, external_user_id)`。

use rusqlite::{Connection, OptionalExtension, Row, params};

use crate::domain::user::User;
use crate::error::AppError;

/// 插入新用户。
pub fn insert(conn: &Connection, user: &User) -> Result<(), AppError> {
    conn.execute(
        "INSERT INTO users (id, provider, external_user_id, created_at) VALUES (?1, ?2, ?3, ?4)",
        params![
            user.id,
            user.provider,
            user.external_user_id,
            user.created_at
        ],
    )?;
    Ok(())
}

/// 按 PAA 用户 ID 查询。
pub fn get(conn: &Connection, id: &str) -> Result<Option<User>, AppError> {
    let user = conn
        .query_row(
            "SELECT id, provider, external_user_id, created_at FROM users WHERE id = ?1",
            params![id],
            row_to_user,
        )
        .optional()?;
    Ok(user)
}

/// 按（平台, 外部用户 ID）查询。
pub fn find_by_identity(
    conn: &Connection,
    provider: &str,
    external_user_id: &str,
) -> Result<Option<User>, AppError> {
    let user = conn
        .query_row(
            "SELECT id, provider, external_user_id, created_at FROM users \
             WHERE provider = ?1 AND external_user_id = ?2",
            params![provider, external_user_id],
            row_to_user,
        )
        .optional()?;
    Ok(user)
}

fn row_to_user(row: &Row<'_>) -> rusqlite::Result<User> {
    Ok(User {
        id: row.get(0)?,
        provider: row.get(1)?,
        external_user_id: row.get(2)?,
        created_at: row.get(3)?,
    })
}
