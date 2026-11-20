//! transaction 领域模型：实体、枚举、输入结构、筛选条件与汇总结构。
//!
//! 权威表结构见 `docs/design/paadm数据表结构设计.md`：
//! `transactions(id, user_id, type, amount, category, description,
//! occurred_at, created_at, updated_at)`。
//!
//! 约定：
//! - 第一版只使用人民币，`amount` 为整数人民币分（¥18.50 → 1850），不保存币种；
//! - `amount` 始终为正数，收入/支出由 `type` 区分；
//! - 所有记录都属于某个 `user_id`；
//! - 时间只保存一份 ISO 8601 字符串（保留时区偏移），范围比较交给 SQL 的 `julianday()`。

use serde::{Deserialize, Serialize};

use crate::domain::common;
use crate::error::AppError;

/// 收支类型：只允许 income / expense。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TransactionType {
    /// 收入
    Income,
    /// 支出
    Expense,
}

impl TransactionType {
    /// 稳定的字符串表示（与数据库 CHECK 约束一致）。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Income => "income",
            Self::Expense => "expense",
        }
    }

    /// 解析用户输入的 type：只接受小写 `income` / `expense`（忽略首尾空白），
    /// 其它取值（含大写变体）一律返回 INVALID_ARGUMENT。
    pub fn parse(value: &str) -> Result<Self, AppError> {
        match value.trim() {
            "income" => Ok(Self::Income),
            "expense" => Ok(Self::Expense),
            _ => Err(AppError::invalid_argument(
                "type must be either 'income' or 'expense'",
            )),
        }
    }
}

/// 一条收支记录（对外 JSON 结构与数据库行一一对应）。
#[derive(Debug, Clone, Serialize)]
pub struct Transaction {
    pub id: String,
    pub user_id: String,
    pub r#type: TransactionType,
    /// 金额，单位人民币分（始终 > 0）。
    pub amount: i64,
    pub category: Option<String>,
    pub description: Option<String>,
    pub occurred_at: String,
    pub created_at: String,
    pub updated_at: String,
}

/// 创建收支记录的原始输入（CLI 参数或 `--input-json` 反序列化结果）。
#[derive(Debug, Clone, Default)]
pub struct CreateTransactionInput {
    pub r#type: Option<String>,
    pub amount: Option<i64>,
    pub category: Option<String>,
    pub description: Option<String>,
    pub occurred_at: Option<String>,
}

/// 修改收支记录的原始输入。
///
/// 规则：未提供的字段不覆盖；`clear_category` / `clear_description`
/// 显式清空可选字段。
#[derive(Debug, Clone, Default)]
pub struct UpdateTransactionInput {
    pub r#type: Option<String>,
    pub amount: Option<i64>,
    pub category: Option<String>,
    pub description: Option<String>,
    pub occurred_at: Option<String>,
    pub clear_category: bool,
    pub clear_description: bool,
}

/// 列表 / 汇总共用的筛选条件（已校验；时间保留原串，由 SQL 用 julianday 比较）。
#[derive(Debug, Clone, Default)]
pub struct TransactionFilter {
    /// 按类型筛选。
    pub r#type: Option<TransactionType>,
    /// 按一级分类精确筛选。
    pub category: Option<String>,
    /// 发生时间下界（含）。
    pub from: Option<String>,
    /// 发生时间上界（含）。
    pub to: Option<String>,
}

impl TransactionFilter {
    /// 从 CLI/JSON 原始参数构造筛选条件，统一做校验。
    pub fn build(
        r#type: Option<&str>,
        category: Option<&str>,
        from: Option<&str>,
        to: Option<&str>,
    ) -> Result<Self, AppError> {
        let r#type = match r#type {
            Some(value) => Some(TransactionType::parse(value)?),
            None => None,
        };
        let category = common::validate_optional_text("category", category)?;
        let from = match from {
            Some(value) => Some(common::validate_datetime("from", value)?),
            None => None,
        };
        let to = match to {
            Some(value) => Some(common::validate_datetime("to", value)?),
            None => None,
        };
        Ok(Self {
            r#type,
            category,
            from,
            to,
        })
    }
}

/// 汇总中某个一级分类的小计。
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CategoryTotal {
    pub category: String,
    pub income: i64,
    pub expense: i64,
}

/// 收支基础汇总（人民币，不按币种分组）。
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Summary {
    pub income_total: i64,
    pub expense_total: i64,
    pub net_total: i64,
    pub by_category: Vec<CategoryTotal>,
}
