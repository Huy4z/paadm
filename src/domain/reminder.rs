//! reminder 领域模型：实体、状态枚举、输入结构与领域校验。
//!
//! 表结构以 `docs/design/paadm数据表结构设计.md` 为唯一权威来源：
//! `reminders(id, user_id, item_id, content, remind_at, status, created_at, updated_at)`。
//!
//! 边界（见数据表设计 2.4）：
//! - 提醒独立于事项存在；`item_id` 为空表示独立提醒，非空表示关联某个待办/日程。
//! - 第一版只保存一次性提醒；`repeat_rule` 属于已废弃设计，不在表结构中。
//! - paadm 只管理提醒数据，不负责到点触发/推送；`triggered` 状态由 Scheduler 调用
//!   `reminder trigger` 写入。
//! - 所有记录都属于某个用户，可见性由 `user_id` 决定。
//!
//! 时间约定：`remind_at` 保存用户输入的原始 ISO 8601 串（保留时区偏移语义），
//! 范围比较与排序在 SQL 中通过 `julianday()` 完成。

use serde::Serialize;

use crate::domain::common;
use crate::error::AppError;

/// 提醒状态：pending（待触发）/ triggered（已触发）/ cancelled（已取消）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ReminderStatus {
    Pending,
    Triggered,
    Cancelled,
}

impl ReminderStatus {
    /// 数据库/JSON 中使用的稳定小写字符串。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Triggered => "triggered",
            Self::Cancelled => "cancelled",
        }
    }

    /// 从用户输入解析状态；非法值返回 INVALID_ARGUMENT。
    pub fn parse(value: &str) -> Result<Self, AppError> {
        match value.trim() {
            "pending" => Ok(Self::Pending),
            "triggered" => Ok(Self::Triggered),
            "cancelled" => Ok(Self::Cancelled),
            other => Err(AppError::invalid_argument(format!(
                "status must be 'pending', 'triggered' or 'cancelled', got '{other}'"
            ))),
        }
    }

    /// 从数据库列解析；供 Repository 映射行使用。
    pub fn from_db_str(value: &str) -> Option<Self> {
        match value {
            "pending" => Some(Self::Pending),
            "triggered" => Some(Self::Triggered),
            "cancelled" => Some(Self::Cancelled),
            _ => None,
        }
    }
}

/// 一条提醒记录。
#[derive(Debug, Clone, Serialize)]
pub struct Reminder {
    pub id: String,
    pub user_id: String,
    /// 关联的事项（items.id）；`None` 表示独立提醒。
    pub item_id: Option<String>,
    pub content: String,
    /// 用户输入的原始提醒时间（保留时区偏移语义）。
    pub remind_at: String,
    pub status: ReminderStatus,
    pub created_at: String,
    pub updated_at: String,
}

/// 创建提醒的输入（`None` 表示未提供，由领域校验给出明确错误）。
#[derive(Debug, Clone, Default)]
pub struct CreateReminderInput {
    pub content: Option<String>,
    pub remind_at: Option<String>,
    pub item_id: Option<String>,
}

/// 修改提醒的输入。
///
/// 约定：
/// - `None` 表示“本次不修改该字段”，不会被覆盖；
/// - `clear_item_id` 显式解除事项关联，优先级高于 `item_id`。
#[derive(Debug, Clone, Default)]
pub struct UpdateReminderInput {
    pub id: String,
    pub content: Option<String>,
    pub remind_at: Option<String>,
    pub item_id: Option<String>,
    pub clear_item_id: bool,
}

/// 提醒列表查询条件（时间范围为闭区间，按真实时刻比较）。
#[derive(Debug, Clone, Default)]
pub struct ListRemindersInput {
    pub status: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub item_id: Option<String>,
}

/// 校验必填的提醒内容（trim 后非空）。
pub fn validate_content(value: Option<&str>) -> Result<String, AppError> {
    match value {
        Some(raw) => common::validate_required_text("content", raw),
        None => Err(AppError::invalid_argument("content must not be empty")),
    }
}

/// 校验必填的提醒时间（必须是带时区偏移的 ISO 8601），返回 trim 后的原串。
pub fn validate_remind_at(value: Option<&str>) -> Result<String, AppError> {
    match value {
        Some(raw) => common::validate_datetime("remind_at", raw),
        None => Err(AppError::invalid_argument(
            "remind_at is required and must be an ISO 8601 datetime with a timezone offset",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_parse_is_strict() {
        assert_eq!(
            ReminderStatus::parse("pending").unwrap(),
            ReminderStatus::Pending
        );
        assert_eq!(
            ReminderStatus::parse("triggered").unwrap(),
            ReminderStatus::Triggered
        );
        assert_eq!(
            ReminderStatus::parse("cancelled").unwrap(),
            ReminderStatus::Cancelled
        );
        // 旧状态 active 以及大小写变体都必须拒绝。
        for invalid in ["active", "PENDING", "done", ""] {
            assert!(ReminderStatus::parse(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn status_roundtrip() {
        for status in [
            ReminderStatus::Pending,
            ReminderStatus::Triggered,
            ReminderStatus::Cancelled,
        ] {
            assert_eq!(ReminderStatus::from_db_str(status.as_str()), Some(status));
            assert_eq!(serde_json::to_value(status).unwrap(), status.as_str());
        }
    }

    #[test]
    fn content_and_remind_at_are_required() {
        assert!(validate_content(None).is_err());
        assert!(validate_content(Some("   ")).is_err());
        assert_eq!(validate_content(Some(" 你好 ")).unwrap(), "你好");

        assert!(validate_remind_at(None).is_err());
        assert!(validate_remind_at(Some("2026-11-17")).is_err());
        assert_eq!(
            validate_remind_at(Some(" 2026-11-17T15:00:00+08:00 ")).unwrap(),
            "2026-11-17T15:00:00+08:00"
        );
    }
}
