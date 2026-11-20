//! 用户业务服务：创建、解析（查找或创建）、查询与存在性校验。

use rusqlite::Connection;
use uuid::Uuid;

use crate::domain::common;
use crate::domain::user::User;
use crate::error::AppError;
use crate::repositories::user as user_repo;

/// 创建用户。
///
/// - `provider` 统一转小写，例如 `QQ` → `qq`；
/// - 同一（provider, external_user_id）已存在时返回 `DUPLICATE_USER`。
pub fn create(conn: &Connection, provider: &str, external_user_id: &str) -> Result<User, AppError> {
    let provider = common::validate_provider(provider)?;
    let external_user_id = common::validate_external_user_id(external_user_id)?;

    if user_repo::find_by_identity(conn, &provider, &external_user_id)?.is_some() {
        return Err(AppError::duplicate_user(&provider, &external_user_id));
    }

    let user = User {
        id: Uuid::new_v4().to_string(),
        provider,
        external_user_id,
        created_at: common::now_iso(),
    };
    user_repo::insert(conn, &user)?;
    Ok(user)
}

/// 按外部身份解析用户：已存在则返回（`created=false`），否则创建（`created=true`）。
///
/// Agent 在首次接触某个 QQ/微信用户时可用它取得 PAA 用户 ID。
pub fn resolve(
    conn: &Connection,
    provider: &str,
    external_user_id: &str,
) -> Result<(User, bool), AppError> {
    let provider = common::validate_provider(provider)?;
    let external_user_id = common::validate_external_user_id(external_user_id)?;

    if let Some(user) = user_repo::find_by_identity(conn, &provider, &external_user_id)? {
        return Ok((user, false));
    }

    let user = common::now_iso();
    let user = User {
        id: Uuid::new_v4().to_string(),
        provider,
        external_user_id,
        created_at: user,
    };
    user_repo::insert(conn, &user)?;
    Ok((user, true))
}

/// 按 ID 查询用户；不存在 → `USER_NOT_FOUND`。
pub fn get(conn: &Connection, id: &str) -> Result<User, AppError> {
    let id = common::validate_id("id", id)?;
    user_repo::get(conn, &id)?.ok_or_else(|| AppError::user_not_found(&id))
}

/// 校验用户存在；不存在 → `USER_NOT_FOUND`。
pub fn ensure_exists(conn: &Connection, id: &str) -> Result<(), AppError> {
    get(conn, id).map(|_| ())
}
