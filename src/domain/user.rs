//! 用户领域模型：PAA 内部用户与外部平台身份的映射。
//!
//! 每个外部平台身份对应一个独立 PAA 用户；同一平台的外部用户 ID 不允许重复。

use serde::Serialize;

/// 一个 PAA 用户。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct User {
    /// PAA 内部用户 ID（UUID）。
    pub id: String,
    /// 外部平台标识，例如 `qq`、`wechat`（小写）。
    pub provider: String,
    /// 外部平台提供的稳定用户标识。
    pub external_user_id: String,
    /// 创建时间（ISO 8601）。
    pub created_at: String,
}
