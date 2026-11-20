//! Reminder（提醒记录）CLI 命令定义与实现。
//!
//! `item_id` 为空表示独立提醒；非空表示关联某个待办/日程（`items.id`）。
//! paadm 只管理提醒数据，不负责到点触发/推送；`trigger` 子命令供 Scheduler 使用。
//!
//! 输出约定（见 docs/paadm-impl-notes.md 第 4 节）：
//! - 单条记录：`data` 直接是记录对象；
//! - 列表：`data = {"items": [...], "count": N}`；
//! - 删除：`data = {"id": "...", "deleted": true}`；
//! - `--input-json` 从文件或 stdin(-) 读取完整参数，字段名与 CLI 长参数一致；
//! - 业务命令统一通过 `crate::cli::require_user(&db, cli)?` 取得当前用户。

use clap::{Args, Subcommand};
use serde::Deserialize;

use crate::cli::{Cli, CliOutput, load_input_json, require_user};
use crate::domain::reminder::{
    CreateReminderInput, ListRemindersInput, Reminder, UpdateReminderInput,
};
use crate::error::AppError;
use crate::services::reminder as reminder_service;

#[derive(Subcommand, Debug)]
pub enum ReminderCommand {
    /// 创建提醒记录
    Create(CreateArgs),
    /// 按 ID 查询提醒
    Get(IdArgs),
    /// 查询提醒列表
    List(ListArgs),
    /// 修改提醒
    Update(UpdateArgs),
    /// 标记提醒已触发（pending → triggered，由 Scheduler 调用）
    Trigger(IdArgs),
    /// 取消提醒（保留记录，只改状态）
    Cancel(IdArgs),
    /// 删除提醒
    Delete(IdArgs),
}

/// 创建提醒参数。
#[derive(Args, Debug, Deserialize, Default)]
#[serde(default)]
pub struct CreateArgs {
    /// 提醒内容（必填）
    #[arg(long, required_unless_present = "input_json")]
    pub content: Option<String>,
    /// 提醒时间（带时区偏移的 ISO 8601，必填）
    #[arg(long, required_unless_present = "input_json")]
    pub remind_at: Option<String>,
    /// 关联的待办/日程 ID（可选，必须是当前用户已存在的 items.id）
    #[arg(long)]
    pub item_id: Option<String>,
    /// 从 JSON 文件或 stdin(-) 读取完整输入；提供后忽略其他业务参数
    #[arg(long, value_name = "PATH|-")]
    pub input_json: Option<String>,
}

/// 查询单条参数。
#[derive(Args, Debug, Deserialize)]
pub struct IdArgs {
    /// 提醒 ID
    #[arg(long)]
    pub id: String,
}

/// 列表查询参数。
#[derive(Args, Debug, Deserialize, Default)]
#[serde(default)]
pub struct ListArgs {
    /// 按状态筛选：pending / triggered / cancelled
    #[arg(long)]
    pub status: Option<String>,
    /// 提醒时间范围起点（含）
    #[arg(long)]
    pub from: Option<String>,
    /// 提醒时间范围终点（含）
    #[arg(long)]
    pub to: Option<String>,
    /// 按关联事项筛选
    #[arg(long)]
    pub item_id: Option<String>,
}

/// 修改参数。
#[derive(Args, Debug, Deserialize, Default)]
#[serde(default)]
pub struct UpdateArgs {
    /// 提醒 ID（必填）
    #[arg(long, required_unless_present = "input_json")]
    pub id: Option<String>,
    /// 新内容
    #[arg(long)]
    pub content: Option<String>,
    /// 新提醒时间
    #[arg(long)]
    pub remind_at: Option<String>,
    /// 新的关联事项 ID
    #[arg(long)]
    pub item_id: Option<String>,
    /// 显式解除事项关联
    #[arg(long)]
    pub clear_item_id: bool,
    /// 从 JSON 文件或 stdin(-) 读取修改内容；提供后忽略其他业务参数
    #[arg(long, value_name = "PATH|-")]
    pub input_json: Option<String>,
}

/// 提醒命令入口。
pub fn run(cli: &Cli, command: &ReminderCommand) -> Result<CliOutput, AppError> {
    let db = cli.open_db()?;
    let user_id = require_user(&db, cli)?;
    let conn = db.connection();

    match command {
        ReminderCommand::Create(args) => run_create(conn, &user_id, args),
        ReminderCommand::Get(args) => run_get(conn, &user_id, args),
        ReminderCommand::List(args) => run_list(conn, &user_id, args),
        ReminderCommand::Update(args) => run_update(conn, &user_id, args),
        ReminderCommand::Trigger(args) => run_trigger(conn, &user_id, args),
        ReminderCommand::Cancel(args) => run_cancel(conn, &user_id, args),
        ReminderCommand::Delete(args) => run_delete(conn, &user_id, args),
    }
}

/// `reminder create`
fn run_create(
    conn: &rusqlite::Connection,
    user_id: &str,
    args: &CreateArgs,
) -> Result<CliOutput, AppError> {
    let args = resolve_create_args(args)?;
    let reminder = reminder_service::create(
        conn,
        user_id,
        CreateReminderInput {
            content: args.content,
            remind_at: args.remind_at,
            item_id: args.item_id,
        },
    )?;
    record_output(reminder, "已创建提醒")
}

/// `reminder get`
fn run_get(
    conn: &rusqlite::Connection,
    user_id: &str,
    args: &IdArgs,
) -> Result<CliOutput, AppError> {
    let reminder = reminder_service::get(conn, user_id, &args.id)?;
    record_output(reminder, "提醒详情")
}

/// `reminder list`
fn run_list(
    conn: &rusqlite::Connection,
    user_id: &str,
    args: &ListArgs,
) -> Result<CliOutput, AppError> {
    let items = reminder_service::list(
        conn,
        user_id,
        ListRemindersInput {
            status: args.status.clone(),
            from: args.from.clone(),
            to: args.to.clone(),
            item_id: args.item_id.clone(),
        },
    )?;
    let count = items.len();
    let data = serde_json::json!({ "items": items, "count": count });
    Ok(CliOutput::new(data, format!("共 {count} 条提醒")))
}

/// `reminder update`
fn run_update(
    conn: &rusqlite::Connection,
    user_id: &str,
    args: &UpdateArgs,
) -> Result<CliOutput, AppError> {
    let args = resolve_update_args(args)?;
    let id = args
        .id
        .ok_or_else(|| AppError::invalid_argument("id is required"))?;
    let reminder = reminder_service::update(
        conn,
        user_id,
        UpdateReminderInput {
            id,
            content: args.content,
            remind_at: args.remind_at,
            item_id: args.item_id,
            clear_item_id: args.clear_item_id,
        },
    )?;
    record_output(reminder, "已更新提醒")
}

/// `reminder trigger`
fn run_trigger(
    conn: &rusqlite::Connection,
    user_id: &str,
    args: &IdArgs,
) -> Result<CliOutput, AppError> {
    let reminder = reminder_service::trigger(conn, user_id, &args.id)?;
    record_output(reminder, "提醒已标记为已触发")
}

/// `reminder cancel`
fn run_cancel(
    conn: &rusqlite::Connection,
    user_id: &str,
    args: &IdArgs,
) -> Result<CliOutput, AppError> {
    let reminder = reminder_service::cancel(conn, user_id, &args.id)?;
    record_output(reminder, "已取消提醒")
}

/// `reminder delete`
fn run_delete(
    conn: &rusqlite::Connection,
    user_id: &str,
    args: &IdArgs,
) -> Result<CliOutput, AppError> {
    let id = reminder_service::delete(conn, user_id, &args.id)?;
    let data = serde_json::json!({ "id": id, "deleted": true });
    Ok(CliOutput::new(data, format!("已删除提醒 {id}")))
}

/// 单条记录的 JSON/人类可读输出。
fn record_output(reminder: Reminder, action: &str) -> Result<CliOutput, AppError> {
    let data = serde_json::to_value(&reminder)
        .map_err(|_| AppError::internal("failed to serialize reminder"))?;
    let human = format!(
        "{action}：{}（{}，提醒时间 {}）",
        reminder.id, reminder.content, reminder.remind_at
    );
    Ok(CliOutput::new(data, human))
}

/// 解析创建参数：`--input-json` 优先，否则使用命令行参数。
fn resolve_create_args(args: &CreateArgs) -> Result<CreateArgs, AppError> {
    match args.input_json.as_deref() {
        Some(source) => load_input_json::<CreateArgs>(source),
        None => Ok(CreateArgs {
            content: args.content.clone(),
            remind_at: args.remind_at.clone(),
            item_id: args.item_id.clone(),
            input_json: None,
        }),
    }
}

/// 解析修改参数：`--input-json` 优先，否则使用命令行参数。
fn resolve_update_args(args: &UpdateArgs) -> Result<UpdateArgs, AppError> {
    match args.input_json.as_deref() {
        Some(source) => load_input_json::<UpdateArgs>(source),
        None => Ok(UpdateArgs {
            id: args.id.clone(),
            content: args.content.clone(),
            remind_at: args.remind_at.clone(),
            item_id: args.item_id.clone(),
            clear_item_id: args.clear_item_id,
            input_json: None,
        }),
    }
}
