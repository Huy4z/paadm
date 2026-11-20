//! 各领域共享的输入校验与时间处理工具。
//!
//! 表结构约定以 `docs/design/paadm数据表结构设计.md` 为准：
//! - 时间统一使用 ISO 8601 的 TEXT（例如 `2026-11-17T15:00:00+08:00`），
//!   不再额外保存 UTC 归一化列；时间范围比较统一在 SQL 中通过
//!   SQLite 的 `julianday()` 处理时区偏移；
//! - 金额统一使用整数（人民币分），不保存币种；
//! - 所有业务数据都必须带 `user_id`，查询必须按 `user_id` 过滤。

use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use crate::error::AppError;

/// 单个文本字段的最大字符数（防止超长输入，见测试用例 AGENT-009）。
pub const MAX_TEXT_LENGTH: usize = 100_000;

/// 校验并返回带时区偏移的 ISO 8601 时间字符串（保持用户原始写法）。
///
/// 新代码应优先使用本函数；时间范围比较由 SQL 的 `julianday()` 完成。
pub fn validate_datetime(field: &str, value: &str) -> Result<String, AppError> {
    let trimmed = value.trim();
    OffsetDateTime::parse(trimmed, &Rfc3339).map_err(|_| {
        AppError::invalid_argument(format!(
            "{field} must be an ISO 8601 datetime with a timezone offset, e.g. 2026-11-17T15:00:00+08:00"
        ))
    })?;
    Ok(trimmed.to_string())
}

/// 校验必填文本：去首尾空白后不能为空，且不能超过长度上限。
pub fn validate_required_text(field: &str, value: &str) -> Result<String, AppError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(AppError::invalid_argument(format!(
            "{field} must not be empty"
        )));
    }
    validate_text_length(field, trimmed)?;
    Ok(trimmed.to_string())
}

/// 校验可选文本：去首尾空白；空字符串按“未提供”处理。
pub fn validate_optional_text(
    field: &str,
    value: Option<&str>,
) -> Result<Option<String>, AppError> {
    match value {
        None => Ok(None),
        Some(raw) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                return Ok(None);
            }
            validate_text_length(field, trimmed)?;
            Ok(Some(trimmed.to_string()))
        }
    }
}

/// 校验文本长度上限（按字符数计算）。
pub fn validate_text_length(field: &str, value: &str) -> Result<(), AppError> {
    if value.chars().count() > MAX_TEXT_LENGTH {
        return Err(AppError::invalid_argument(format!(
            "{field} exceeds the maximum length of {MAX_TEXT_LENGTH} characters"
        )));
    }
    Ok(())
}

/// 校验 ID 非空。
pub fn validate_id(field: &str, value: &str) -> Result<String, AppError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(AppError::invalid_argument(format!(
            "{field} must not be empty"
        )));
    }
    Ok(trimmed.to_string())
}

/// 校验金额：必须是正整数（人民币分，正负号由 type 表达）。
pub fn validate_amount(amount: i64) -> Result<i64, AppError> {
    if amount <= 0 {
        return Err(AppError::invalid_argument(
            "amount must be a positive integer in RMB cents",
        ));
    }
    Ok(amount)
}

/// 校验 `all_day` 布尔字段（数据库用 INTEGER 0/1 存储）。
pub fn validate_all_day(value: i64) -> Result<bool, AppError> {
    match value {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(AppError::invalid_argument("all-day must be 0 or 1")),
    }
}

/// 校验外部平台标识（如 qq、wechat），统一转小写。
pub fn validate_provider(value: &str) -> Result<String, AppError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(AppError::invalid_argument("provider must not be empty"));
    }
    if trimmed.chars().any(|c| c.is_whitespace()) {
        return Err(AppError::invalid_argument(
            "provider must not contain whitespace",
        ));
    }
    validate_text_length("provider", trimmed)?;
    Ok(trimmed.to_ascii_lowercase())
}

/// 校验外部平台用户 ID。
pub fn validate_external_user_id(value: &str) -> Result<String, AppError> {
    validate_required_text("external-user-id", value)
}

/// 当前时间（UTC，RFC 3339），用于 created_at / updated_at / completed_at。
pub fn now_iso() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}
