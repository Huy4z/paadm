//! Task（待办）CLI 命令：把参数转换成 Service 调用，并组织 JSON data / 人类可读输出。
//!
//! 待办与日程统一存于 `items` 表：本命令组的记录 `type='task'`。
//! 输出契约（见 impl-notes 第 4 节）：
//! - 单条记录：`data` 直接是 item 对象（含 `id`）；
//! - 列表：`data = {"items": [...], "count": N}`，空列表也是成功；
//! - 删除：`data = {"id": "...", "deleted": true}`；
//! - `--json` 的打印由 `cli::mod` 统一处理，本文件只返回 `CliOutput`。

use clap::{Args, Subcommand};
use serde::Deserialize;

use crate::cli::{Cli, CliOutput};
use crate::domain::item::{CreateTaskInput, Item, ListTasksInput, UpdateTaskInput};
use crate::error::AppError;
use crate::services::item as item_service;

#[derive(Subcommand, Debug)]
pub enum TaskCommand {
    /// 创建待办
    Create(CreateArgs),
    /// 按 ID 查询单条待办
    Get(IdArgs),
    /// 查询待办列表
    List(ListArgs),
    /// 修改待办
    Update(UpdateArgs),
    /// 标记待办完成
    Complete(IdArgs),
    /// 取消待办（与完成的语义不同）
    Cancel(IdArgs),
    /// 删除待办
    Delete(IdArgs),
}

/// 创建待办参数。
#[derive(Args, Debug, Deserialize, Default)]
#[serde(default)]
pub struct CreateArgs {
    /// 待办标题（必填，去空白后不能为空）
    #[arg(long, required_unless_present = "input_json")]
    pub title: Option<String>,
    /// 详细描述
    #[arg(long)]
    pub description: Option<String>,
    /// 优先级（整数，应用约定取值）
    #[arg(long, allow_hyphen_values = true)]
    pub priority: Option<i64>,
    /// 截止时间（带时区偏移的 ISO 8601）
    #[arg(long)]
    pub due_at: Option<String>,
    /// 从 JSON 文件或 stdin(-) 读取完整输入；提供后忽略其他业务参数
    #[arg(long, value_name = "PATH|-")]
    pub input_json: Option<String>,
}

/// 查询单条参数。
#[derive(Args, Debug, Deserialize)]
pub struct IdArgs {
    /// 待办 ID
    #[arg(long)]
    pub id: String,
}

/// 列表查询参数。
#[derive(Args, Debug, Deserialize, Default)]
#[serde(default)]
pub struct ListArgs {
    /// 按状态筛选：pending / completed / cancelled
    #[arg(long)]
    pub status: Option<String>,
    /// 按优先级筛选（整数）
    #[arg(long, allow_hyphen_values = true)]
    pub priority: Option<i64>,
    /// 截止时间范围起点（含）
    #[arg(long)]
    pub due_from: Option<String>,
    /// 截止时间范围终点（含）
    #[arg(long)]
    pub due_to: Option<String>,
    /// 标题关键词（包含匹配）
    #[arg(long)]
    pub keyword: Option<String>,
}

/// 修改参数。
#[derive(Args, Debug, Deserialize, Default)]
#[serde(default)]
pub struct UpdateArgs {
    /// 待办 ID（必填）
    #[arg(long, required_unless_present = "input_json")]
    pub id: Option<String>,
    /// 新标题
    #[arg(long)]
    pub title: Option<String>,
    /// 新描述
    #[arg(long)]
    pub description: Option<String>,
    /// 新优先级（整数）
    #[arg(long, allow_hyphen_values = true)]
    pub priority: Option<i64>,
    /// 新截止时间
    #[arg(long)]
    pub due_at: Option<String>,
    /// 显式清空描述
    #[arg(long)]
    pub clear_description: bool,
    /// 显式清空优先级
    #[arg(long)]
    pub clear_priority: bool,
    /// 显式清空截止时间
    #[arg(long)]
    pub clear_due_at: bool,
    /// 从 JSON 文件或 stdin(-) 读取修改内容；提供后忽略其他业务参数
    #[arg(long, value_name = "PATH|-")]
    pub input_json: Option<String>,
}

/// 待办命令入口。
pub fn run(cli: &Cli, command: &TaskCommand) -> Result<CliOutput, AppError> {
    match command {
        TaskCommand::Create(args) => run_create(cli, args),
        TaskCommand::Get(args) => run_get(cli, args),
        TaskCommand::List(args) => run_list(cli, args),
        TaskCommand::Update(args) => run_update(cli, args),
        TaskCommand::Complete(args) => run_complete(cli, args),
        TaskCommand::Cancel(args) => run_cancel(cli, args),
        TaskCommand::Delete(args) => run_delete(cli, args),
    }
}

fn run_create(cli: &Cli, args: &CreateArgs) -> Result<CliOutput, AppError> {
    let args = effective_create_args(args)?;
    let input = CreateTaskInput {
        title: args.title,
        description: args.description,
        priority: args.priority,
        due_at: args.due_at,
    };

    let db = cli.open_db()?;
    let user_id = crate::cli::require_user(&db, cli)?;
    let item = item_service::create_task(db.connection(), &user_id, input)?;
    single_item_output(item, "已创建待办")
}

fn run_get(cli: &Cli, args: &IdArgs) -> Result<CliOutput, AppError> {
    let db = cli.open_db()?;
    let user_id = crate::cli::require_user(&db, cli)?;
    let item = item_service::get_task(db.connection(), &user_id, &args.id)?;
    single_item_output(item, "待办")
}

fn run_list(cli: &Cli, args: &ListArgs) -> Result<CliOutput, AppError> {
    let input = ListTasksInput {
        status: args.status.clone(),
        priority: args.priority,
        due_from: args.due_from.clone(),
        due_to: args.due_to.clone(),
        keyword: args.keyword.clone(),
    };

    let db = cli.open_db()?;
    let user_id = crate::cli::require_user(&db, cli)?;
    let items = item_service::list_tasks(db.connection(), &user_id, &input)?;
    list_output(items, "没有符合条件的待办")
}

fn run_update(cli: &Cli, args: &UpdateArgs) -> Result<CliOutput, AppError> {
    let args = effective_update_args(args)?;
    let input = UpdateTaskInput {
        id: args.id.unwrap_or_default(),
        title: args.title,
        description: args.description,
        priority: args.priority,
        due_at: args.due_at,
        clear_description: args.clear_description,
        clear_priority: args.clear_priority,
        clear_due_at: args.clear_due_at,
    };

    let db = cli.open_db()?;
    let user_id = crate::cli::require_user(&db, cli)?;
    let item = item_service::update_task(db.connection(), &user_id, input)?;
    single_item_output(item, "已更新待办")
}

fn run_complete(cli: &Cli, args: &IdArgs) -> Result<CliOutput, AppError> {
    let db = cli.open_db()?;
    let user_id = crate::cli::require_user(&db, cli)?;
    let item = item_service::complete_task(db.connection(), &user_id, &args.id)?;
    single_item_output(item, "已完成待办")
}

fn run_cancel(cli: &Cli, args: &IdArgs) -> Result<CliOutput, AppError> {
    let db = cli.open_db()?;
    let user_id = crate::cli::require_user(&db, cli)?;
    let item = item_service::cancel_task(db.connection(), &user_id, &args.id)?;
    single_item_output(item, "已取消待办")
}

fn run_delete(cli: &Cli, args: &IdArgs) -> Result<CliOutput, AppError> {
    let db = cli.open_db()?;
    let user_id = crate::cli::require_user(&db, cli)?;
    item_service::delete_task(db.connection(), &user_id, &args.id)?;
    delete_output(args.id.trim())
}

// ---------------------------------------------------------------------------
// 内部辅助
// ---------------------------------------------------------------------------

/// 若提供了 `--input-json`，用 JSON 输入整体替换命令行参数；否则沿用 CLI 参数。
fn effective_create_args(args: &CreateArgs) -> Result<CreateArgs, AppError> {
    if let Some(source) = args.input_json.as_deref() {
        return crate::cli::load_input_json::<CreateArgs>(source);
    }
    Ok(CreateArgs {
        title: args.title.clone(),
        description: args.description.clone(),
        priority: args.priority,
        due_at: args.due_at.clone(),
        input_json: None,
    })
}

/// 若提供了 `--input-json`，用 JSON 输入整体替换命令行参数；否则沿用 CLI 参数。
fn effective_update_args(args: &UpdateArgs) -> Result<UpdateArgs, AppError> {
    if let Some(source) = args.input_json.as_deref() {
        return crate::cli::load_input_json::<UpdateArgs>(source);
    }
    Ok(UpdateArgs {
        id: args.id.clone(),
        title: args.title.clone(),
        description: args.description.clone(),
        priority: args.priority,
        due_at: args.due_at.clone(),
        clear_description: args.clear_description,
        clear_priority: args.clear_priority,
        clear_due_at: args.clear_due_at,
        input_json: None,
    })
}

/// 单条事项的 JSON + 人类可读输出。
fn single_item_output(item: Item, action: &str) -> Result<CliOutput, AppError> {
    let human = format!(
        "{action} {}: {} [{}/{}]",
        item.id,
        item.title,
        item.item_type.as_str(),
        item.status.as_str()
    );
    let data = serde_json::to_value(&item)?;
    Ok(CliOutput::new(data, human))
}

/// 列表的 JSON + 人类可读输出。
fn list_output(items: Vec<Item>, empty_hint: &str) -> Result<CliOutput, AppError> {
    let rows = items
        .iter()
        .map(serde_json::to_value)
        .collect::<Result<Vec<_>, _>>()?;
    let count = rows.len();
    let data = serde_json::json!({ "items": rows, "count": count });
    let human = if items.is_empty() {
        empty_hint.to_string()
    } else {
        items.iter().map(human_line).collect::<Vec<_>>().join("\n")
    };
    Ok(CliOutput::new(data, human))
}

/// 删除事项的统一 JSON + 人类可读输出。
fn delete_output(id: &str) -> Result<CliOutput, AppError> {
    let data = serde_json::json!({ "id": id, "deleted": true });
    Ok(CliOutput::new(data, format!("已删除: {id}")))
}

/// 列表的人类可读单行摘要。
fn human_line(item: &Item) -> String {
    let mut line = format!(
        "- {} [{}/{}] {}",
        item.id,
        item.item_type.as_str(),
        item.status.as_str(),
        item.title
    );
    if let Some(priority) = item.priority {
        line.push_str(&format!(" priority={priority}"));
    }
    if let Some(due_at) = &item.due_at {
        line.push_str(&format!(" due={due_at}"));
    }
    if let Some(start_at) = &item.start_at {
        line.push_str(&format!(" start={start_at}"));
    }
    line
}
