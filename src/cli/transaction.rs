//! Transaction（收支记录）CLI 命令定义与实现。
//!
//! 第一版只使用人民币：`--amount` 为整数，单位人民币分（¥18.50 → 1850）。
//! 所有业务命令都必须带全局 `--user-id`，记录按用户隔离。

use clap::{Args, Subcommand};
use serde::Deserialize;

use crate::cli::{Cli, CliOutput, load_input_json, require_user};
use crate::domain::transaction::{
    CreateTransactionInput, Summary, TransactionFilter, TransactionType, UpdateTransactionInput,
};
use crate::error::AppError;
use crate::services::transaction as service;

#[derive(Subcommand, Debug)]
pub enum TransactionCommand {
    /// 创建收支记录
    Create(CreateArgs),
    /// 按 ID 查询记录
    Get(IdArgs),
    /// 查询记录列表
    List(ListArgs),
    /// 修改记录
    Update(UpdateArgs),
    /// 删除记录
    Delete(IdArgs),
    /// 基础汇总（收入、支出、净额与按类别统计）
    Summary(SummaryArgs),
}

/// 创建记录参数。
#[derive(Args, Debug, Deserialize, Default)]
#[serde(default)]
pub struct CreateArgs {
    /// 类型：income / expense（必填）
    #[arg(long = "type", required_unless_present = "input_json")]
    pub r#type: Option<String>,
    /// 金额，整数人民币分（必填，必须大于 0）
    #[arg(
        long,
        allow_hyphen_values = true,
        required_unless_present = "input_json"
    )]
    pub amount: Option<i64>,
    /// 一级分类，例如 food
    #[arg(long)]
    pub category: Option<String>,
    /// 备注
    #[arg(long)]
    pub description: Option<String>,
    /// 发生时间（带时区偏移的 ISO 8601，必填）
    #[arg(long, required_unless_present = "input_json")]
    pub occurred_at: Option<String>,
    /// 从 JSON 文件或 stdin(-) 读取完整输入；提供后忽略其他业务参数
    #[arg(long, value_name = "PATH|-")]
    pub input_json: Option<String>,
}

/// 查询单条参数。
#[derive(Args, Debug, Deserialize)]
pub struct IdArgs {
    /// 记录 ID
    #[arg(long)]
    pub id: String,
}

/// 列表查询参数。
#[derive(Args, Debug, Deserialize, Default)]
#[serde(default)]
pub struct ListArgs {
    /// 按类型筛选：income / expense
    #[arg(long = "type")]
    pub r#type: Option<String>,
    /// 按分类筛选
    #[arg(long)]
    pub category: Option<String>,
    /// 发生时间范围起点（含）
    #[arg(long)]
    pub from: Option<String>,
    /// 发生时间范围终点（含）
    #[arg(long)]
    pub to: Option<String>,
}

/// 修改参数。
#[derive(Args, Debug, Deserialize, Default)]
#[serde(default)]
pub struct UpdateArgs {
    /// 记录 ID（必填）
    #[arg(long, required_unless_present = "input_json")]
    pub id: Option<String>,
    /// 新类型
    #[arg(long = "type")]
    pub r#type: Option<String>,
    /// 新金额（整数人民币分）
    #[arg(long, allow_hyphen_values = true)]
    pub amount: Option<i64>,
    /// 新分类
    #[arg(long)]
    pub category: Option<String>,
    /// 新备注
    #[arg(long)]
    pub description: Option<String>,
    /// 新发生时间
    #[arg(long)]
    pub occurred_at: Option<String>,
    /// 显式清空分类
    #[arg(long)]
    pub clear_category: bool,
    /// 显式清空备注
    #[arg(long)]
    pub clear_description: bool,
    /// 从 JSON 文件或 stdin(-) 读取修改内容；提供后忽略其他业务参数
    #[arg(long, value_name = "PATH|-")]
    pub input_json: Option<String>,
}

/// 汇总参数。
#[derive(Args, Debug, Deserialize, Default)]
#[serde(default)]
pub struct SummaryArgs {
    /// 只统计指定类型
    #[arg(long = "type")]
    pub r#type: Option<String>,
    /// 只统计指定分类
    #[arg(long)]
    pub category: Option<String>,
    /// 发生时间范围起点（含）
    #[arg(long)]
    pub from: Option<String>,
    /// 发生时间范围终点（含）
    #[arg(long)]
    pub to: Option<String>,
}

/// 收支命令入口。
pub fn run(cli: &Cli, command: &TransactionCommand) -> Result<CliOutput, AppError> {
    let db = cli.open_db()?;
    let user_id = require_user(&db, cli)?;
    let conn = db.connection();

    match command {
        TransactionCommand::Create(args) => {
            let input = build_create_input(args)?;
            let transaction = service::create(conn, &user_id, input)?;
            let human = format!(
                "已创建{}记录 {}（{} 分）",
                type_label(transaction.r#type),
                transaction.id,
                transaction.amount
            );
            Ok(CliOutput::new(serde_json::to_value(&transaction)?, human))
        }
        TransactionCommand::Get(args) => {
            let transaction = service::get(conn, &user_id, &args.id)?;
            let human = format!(
                "{}记录 {}（{} 分）发生于 {}",
                type_label(transaction.r#type),
                transaction.id,
                transaction.amount,
                transaction.occurred_at
            );
            Ok(CliOutput::new(serde_json::to_value(&transaction)?, human))
        }
        TransactionCommand::List(args) => {
            let filter = TransactionFilter::build(
                args.r#type.as_deref(),
                args.category.as_deref(),
                args.from.as_deref(),
                args.to.as_deref(),
            )?;
            let items = service::list(conn, &user_id, filter)?;
            let count = items.len();
            let data = serde_json::json!({ "items": items, "count": count });
            Ok(CliOutput::new(data, format!("共 {count} 条收支记录")))
        }
        TransactionCommand::Update(args) => {
            let (id, input) = build_update_input(args)?;
            let transaction = service::update(conn, &user_id, &id, input)?;
            let human = format!(
                "已更新{}记录 {}",
                type_label(transaction.r#type),
                transaction.id
            );
            Ok(CliOutput::new(serde_json::to_value(&transaction)?, human))
        }
        TransactionCommand::Delete(args) => {
            let id = service::delete(conn, &user_id, &args.id)?;
            let human = format!("已删除收支记录 {id}");
            let data = serde_json::json!({ "id": id, "deleted": true });
            Ok(CliOutput::new(data, human))
        }
        TransactionCommand::Summary(args) => {
            let filter = TransactionFilter::build(
                args.r#type.as_deref(),
                args.category.as_deref(),
                args.from.as_deref(),
                args.to.as_deref(),
            )?;
            let summary = service::summary(conn, &user_id, filter)?;
            let human = summary_human(&summary);
            Ok(CliOutput::new(serde_json::to_value(&summary)?, human))
        }
    }
}

/// 把创建参数（含 `--input-json`）转换成领域输入。
fn build_create_input(args: &CreateArgs) -> Result<CreateTransactionInput, AppError> {
    let args = match args.input_json.as_deref() {
        Some(source) => load_input_json::<CreateArgs>(source)?,
        None => CreateArgs {
            r#type: args.r#type.clone(),
            amount: args.amount,
            category: args.category.clone(),
            description: args.description.clone(),
            occurred_at: args.occurred_at.clone(),
            input_json: None,
        },
    };
    Ok(CreateTransactionInput {
        r#type: args.r#type,
        amount: args.amount,
        category: args.category,
        description: args.description,
        occurred_at: args.occurred_at,
    })
}

/// 把修改参数（含 `--input-json`）转换成 (ID, 领域输入)。
fn build_update_input(args: &UpdateArgs) -> Result<(String, UpdateTransactionInput), AppError> {
    let args = match args.input_json.as_deref() {
        Some(source) => load_input_json::<UpdateArgs>(source)?,
        None => UpdateArgs {
            id: args.id.clone(),
            r#type: args.r#type.clone(),
            amount: args.amount,
            category: args.category.clone(),
            description: args.description.clone(),
            occurred_at: args.occurred_at.clone(),
            clear_category: args.clear_category,
            clear_description: args.clear_description,
            input_json: None,
        },
    };
    let id = args
        .id
        .ok_or_else(|| AppError::invalid_argument("id is required"))?;
    Ok((
        id,
        UpdateTransactionInput {
            r#type: args.r#type,
            amount: args.amount,
            category: args.category,
            description: args.description,
            occurred_at: args.occurred_at,
            clear_category: args.clear_category,
            clear_description: args.clear_description,
        },
    ))
}

/// 类型的中文标签（用于人类可读输出）。
fn type_label(r#type: TransactionType) -> &'static str {
    match r#type {
        TransactionType::Income => "收入",
        TransactionType::Expense => "支出",
    }
}

/// 人类可读的汇总摘要。
fn summary_human(summary: &Summary) -> String {
    format!(
        "收支汇总：收入 {} 分，支出 {} 分，净额 {} 分",
        summary.income_total, summary.expense_total, summary.net_total
    )
}
