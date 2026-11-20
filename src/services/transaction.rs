//! transaction 业务服务：输入校验、业务规则，协调 Repository。
//!
//! 业务规则（见数据表结构设计 2.5 与测试用例 TX 系列）：
//! - `type` ∈ {income, expense}；`amount` 必须是大于零的整数（人民币分），
//!   收入/支出由 type 区分，金额符号不与 type 混用；
//! - `occurred_at` 必填且必须带时区偏移；
//! - `category` / `description` 可选；
//! - 所有操作都按 `user_id` 隔离，跨用户访问表现为“不存在”；
//! - 修改未提供的字段不覆盖，`clear_*` 显式清空可选字段，任何修改刷新 `updated_at`；
//! - 汇总不区分币种（第一版只有人民币），不同类别分别统计。

use rusqlite::Connection;
use uuid::Uuid;

use crate::domain::common;
use crate::domain::transaction::{
    CreateTransactionInput, Summary, Transaction, TransactionFilter, TransactionType,
    UpdateTransactionInput,
};
use crate::error::AppError;
use crate::repositories::transaction as repository;

/// 创建一条收支记录。
pub fn create(
    conn: &Connection,
    user_id: &str,
    input: CreateTransactionInput,
) -> Result<Transaction, AppError> {
    let user_id = common::validate_id("user-id", user_id)?;

    let raw_type = required_text("type", input.r#type)?;
    let r#type = TransactionType::parse(&raw_type)?;

    let amount = input
        .amount
        .ok_or_else(|| AppError::invalid_argument("amount is required"))?;
    let amount = common::validate_amount(amount)?;

    let category = common::validate_optional_text("category", input.category.as_deref())?;
    let description = common::validate_optional_text("description", input.description.as_deref())?;

    let raw_occurred_at = required_text("occurred-at", input.occurred_at)?;
    let occurred_at = common::validate_datetime("occurred-at", &raw_occurred_at)?;

    let now = common::now_iso();
    let transaction = Transaction {
        id: Uuid::new_v4().to_string(),
        user_id,
        r#type,
        amount,
        category,
        description,
        occurred_at,
        created_at: now.clone(),
        updated_at: now,
    };

    repository::insert(conn, &transaction)?;
    Ok(transaction)
}

/// 按 ID 查询（同时按用户隔离）；不存在返回 TRANSACTION_NOT_FOUND。
pub fn get(conn: &Connection, user_id: &str, id: &str) -> Result<Transaction, AppError> {
    let user_id = common::validate_id("user-id", user_id)?;
    let id = common::validate_id("id", id)?;
    repository::get_by_id(conn, &user_id, &id)?.ok_or_else(|| AppError::transaction_not_found(&id))
}

/// 按筛选条件查询列表。
pub fn list(
    conn: &Connection,
    user_id: &str,
    filter: TransactionFilter,
) -> Result<Vec<Transaction>, AppError> {
    let user_id = common::validate_id("user-id", user_id)?;
    repository::list(conn, &user_id, &filter)
}

/// 修改记录：未提供字段不覆盖，`clear_*` 显式清空；返回修改后的完整记录。
pub fn update(
    conn: &Connection,
    user_id: &str,
    id: &str,
    input: UpdateTransactionInput,
) -> Result<Transaction, AppError> {
    let user_id = common::validate_id("user-id", user_id)?;
    let id = common::validate_id("id", id)?;
    let mut transaction = repository::get_by_id(conn, &user_id, &id)?
        .ok_or_else(|| AppError::transaction_not_found(&id))?;

    if let Some(raw_type) = input.r#type.as_deref() {
        transaction.r#type = TransactionType::parse(raw_type)?;
    }
    if let Some(amount) = input.amount {
        transaction.amount = common::validate_amount(amount)?;
    }
    if input.clear_category {
        transaction.category = None;
    } else if let Some(category) = input.category.as_deref() {
        transaction.category = common::validate_optional_text("category", Some(category))?;
    }
    if input.clear_description {
        transaction.description = None;
    } else if let Some(description) = input.description.as_deref() {
        transaction.description = common::validate_optional_text("description", Some(description))?;
    }
    if let Some(raw_occurred_at) = input.occurred_at.as_deref() {
        transaction.occurred_at = common::validate_datetime("occurred-at", raw_occurred_at)?;
    }

    // 任何修改（包括空修改）都刷新 updated_at。
    transaction.updated_at = common::now_iso();
    if !repository::update(conn, &transaction)? {
        // 读取与写入之间记录被删除：按不存在处理，不假装成功。
        return Err(AppError::transaction_not_found(&id));
    }
    Ok(transaction)
}

/// 删除记录；返回被删除的 ID。
pub fn delete(conn: &Connection, user_id: &str, id: &str) -> Result<String, AppError> {
    let user_id = common::validate_id("user-id", user_id)?;
    let id = common::validate_id("id", id)?;
    if !repository::delete(conn, &user_id, &id)? {
        return Err(AppError::transaction_not_found(&id));
    }
    Ok(id)
}

/// 基础汇总（人民币，不区分币种）。
pub fn summary(
    conn: &Connection,
    user_id: &str,
    filter: TransactionFilter,
) -> Result<Summary, AppError> {
    let user_id = common::validate_id("user-id", user_id)?;
    repository::summarize(conn, &user_id, &filter)
}

/// 必填文本：缺失或空白都返回 INVALID_ARGUMENT。
fn required_text(field: &str, value: Option<String>) -> Result<String, AppError> {
    match value {
        Some(value) if !value.trim().is_empty() => Ok(value),
        _ => Err(AppError::invalid_argument(format!("{field} is required"))),
    }
}
