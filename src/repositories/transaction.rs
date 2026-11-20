//! transaction 数据访问：参数化 SQL 读写与数据库行 → 领域对象。
//!
//! 约定（见数据表结构设计第 4.1 节）：
//! - 所有读取/修改/删除都必须同时按 `user_id` 过滤，跨用户访问表现为“不存在”；
//! - 用户输入全部通过参数绑定，禁止拼接进 SQL；
//! - 时间只有一列 ISO 8601 字符串：范围比较与排序都用 SQLite
//!   的 `julianday()`，跨时区偏移也能正确比较时间点；
//! - 列表固定排序 `julianday(occurred_at) ASC, id ASC`；
//! - 汇总按 `type` / `category` 聚合，不区分币种（第一版只有人民币）。

use std::collections::BTreeMap;

use rusqlite::{Connection, OptionalExtension, Row, params};

use crate::domain::transaction::{
    CategoryTotal, Summary, Transaction, TransactionFilter, TransactionType,
};
use crate::error::AppError;

/// 查询列（顺序与 [`row_to_transaction`] 保持一致）。
const SELECT_COLUMNS: &str =
    "id, user_id, type, amount, category, description, occurred_at, created_at, updated_at";

/// 插入一条收支记录。
pub fn insert(conn: &Connection, transaction: &Transaction) -> Result<(), AppError> {
    conn.execute(
        "INSERT INTO transactions (
            id, user_id, type, amount, category, description,
            occurred_at, created_at, updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            transaction.id,
            transaction.user_id,
            transaction.r#type.as_str(),
            transaction.amount,
            transaction.category,
            transaction.description,
            transaction.occurred_at,
            transaction.created_at,
            transaction.updated_at,
        ],
    )?;
    Ok(())
}

/// 按（user_id, id）查询记录；不存在或不属于该用户都返回 `None`。
pub fn get_by_id(
    conn: &Connection,
    user_id: &str,
    id: &str,
) -> Result<Option<Transaction>, AppError> {
    let sql = format!("SELECT {SELECT_COLUMNS} FROM transactions WHERE user_id = ?1 AND id = ?2");
    let transaction = conn
        .query_row(&sql, params![user_id, id], row_to_transaction)
        .optional()?;
    Ok(transaction)
}

/// 按筛选条件查询列表（严格按 user_id 隔离）。
pub fn list(
    conn: &Connection,
    user_id: &str,
    filter: &TransactionFilter,
) -> Result<Vec<Transaction>, AppError> {
    let sql = format!(
        "SELECT {SELECT_COLUMNS} FROM transactions
         WHERE user_id = ?1
           AND (?2 IS NULL OR type = ?2)
           AND (?3 IS NULL OR category = ?3)
           AND (?4 IS NULL OR julianday(occurred_at) >= julianday(?4))
           AND (?5 IS NULL OR julianday(occurred_at) <= julianday(?5))
         ORDER BY julianday(occurred_at) ASC, id ASC"
    );
    let type_value = filter.r#type.map(TransactionType::as_str);
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(
        params![
            user_id,
            type_value,
            filter.category.as_deref(),
            filter.from.as_deref(),
            filter.to.as_deref(),
        ],
        row_to_transaction,
    )?;

    let mut transactions = Vec::new();
    for row in rows {
        transactions.push(row?);
    }
    Ok(transactions)
}

/// 用给定实体整体覆盖一条已有记录；返回是否命中（调用方负责先确认存在）。
pub fn update(conn: &Connection, transaction: &Transaction) -> Result<bool, AppError> {
    let affected = conn.execute(
        "UPDATE transactions
         SET type = ?1, amount = ?2, category = ?3, description = ?4,
             occurred_at = ?5, updated_at = ?6
         WHERE user_id = ?7 AND id = ?8",
        params![
            transaction.r#type.as_str(),
            transaction.amount,
            transaction.category,
            transaction.description,
            transaction.occurred_at,
            transaction.updated_at,
            transaction.user_id,
            transaction.id,
        ],
    )?;
    Ok(affected > 0)
}

/// 按（user_id, id）删除；返回是否命中。
pub fn delete(conn: &Connection, user_id: &str, id: &str) -> Result<bool, AppError> {
    let affected = conn.execute(
        "DELETE FROM transactions WHERE user_id = ?1 AND id = ?2",
        params![user_id, id],
    )?;
    Ok(affected > 0)
}

/// 基础汇总：收入/支出/净额 + 按一级分类统计（不区分币种）。
///
/// `category` 为 NULL 的记录只计入总额，不生成 `by_category` 行；
/// `by_category` 按 category 升序（BTreeMap 保证稳定输出）。
pub fn summarize(
    conn: &Connection,
    user_id: &str,
    filter: &TransactionFilter,
) -> Result<Summary, AppError> {
    let sql = "SELECT type, category, SUM(amount) FROM transactions
               WHERE user_id = ?1
                 AND (?2 IS NULL OR type = ?2)
                 AND (?3 IS NULL OR category = ?3)
                 AND (?4 IS NULL OR julianday(occurred_at) >= julianday(?4))
                 AND (?5 IS NULL OR julianday(occurred_at) <= julianday(?5))
               GROUP BY type, category";
    let type_value = filter.r#type.map(TransactionType::as_str);
    let mut stmt = conn.prepare(sql)?;
    let mut rows = stmt.query(params![
        user_id,
        type_value,
        filter.category.as_deref(),
        filter.from.as_deref(),
        filter.to.as_deref(),
    ])?;

    let mut income_total: i64 = 0;
    let mut expense_total: i64 = 0;
    let mut by_category: BTreeMap<String, (i64, i64)> = BTreeMap::new();

    while let Some(row) = rows.next()? {
        let raw_type: String = row.get(0)?;
        let category: Option<String> = row.get(1)?;
        let amount: i64 = row.get(2)?;

        let r#type = TransactionType::parse(&raw_type)
            .map_err(|_| AppError::internal("invalid transaction type stored in database"))?;
        match r#type {
            TransactionType::Income => income_total += amount,
            TransactionType::Expense => expense_total += amount,
        }
        if let Some(category) = category {
            let entry = by_category.entry(category).or_insert((0, 0));
            match r#type {
                TransactionType::Income => entry.0 += amount,
                TransactionType::Expense => entry.1 += amount,
            }
        }
    }

    let by_category = by_category
        .into_iter()
        .map(|(category, (income, expense))| CategoryTotal {
            category,
            income,
            expense,
        })
        .collect();

    Ok(Summary {
        income_total,
        expense_total,
        net_total: income_total - expense_total,
        by_category,
    })
}

/// 数据库行 → 领域对象。
fn row_to_transaction(row: &Row<'_>) -> rusqlite::Result<Transaction> {
    let raw_type: String = row.get(2)?;
    let r#type = TransactionType::parse(&raw_type).map_err(|_| {
        rusqlite::Error::InvalidColumnType(2, "type".to_string(), rusqlite::types::Type::Text)
    })?;
    Ok(Transaction {
        id: row.get(0)?,
        user_id: row.get(1)?,
        r#type,
        amount: row.get(3)?,
        category: row.get(4)?,
        description: row.get(5)?,
        occurred_at: row.get(6)?,
        created_at: row.get(7)?,
        updated_at: row.get(8)?,
    })
}
