//! Schedule（日程）CLI 命令：把参数转换成 Service 调用，并组织 JSON data / 人类可读输出。
//!
//! 待办与日程统一存于 `items` 表：本命令组的记录 `type='event'`。
//! 输出契约（见 impl-notes 第 4 节）：
//! - 单条记录：`data` 直接是 item 对象（含 `id`）；
//! - 列表：`data = {"items": [...], "count": N}`，空列表也是成功；
//! - 删除：`data = {"id": "...", "deleted": true}`；
//! - `--json` 的打印由 `cli::mod` 统一处理，本文件只返回 `CliOutput`。

use clap::{Args, Subcommand};
use serde::Deserialize;

use crate::cli::{Cli, CliOutput};
use crate::domain::item::{CreateEventInput, Item, ListEventsInput, UpdateEventInput};
use crate::error::AppError;
use crate::services::item as item_service;

#[derive(Subcommand, Debug)]
pub enum ScheduleCommand {
    /// 创建日程
    Create(CreateArgs),
    /// 按 ID 查询日程
    Get(IdArgs),
    /// 查询日程列表
    List(ListArgs),
    /// 修改日程
    Update(UpdateArgs),
    /// 删除日程
    Delete(IdArgs),
}

/// 创建日程参数。
#[derive(Args, Debug, Deserialize, Default)]
#[serde(default)]
pub struct CreateArgs {
    /// 日程标题（必填）
    #[arg(long, required_unless_present = "input_json")]
    pub title: Option<String>,
    /// 描述
    #[arg(long)]
    pub description: Option<String>,
    /// 开始时间（带时区偏移的 ISO 8601，必填）
    #[arg(long, required_unless_present = "input_json")]
    pub start_at: Option<String>,
    /// 结束时间（可选；提供时必须晚于开始时间）
    #[arg(long)]
    pub end_at: Option<String>,
    /// 地点
    #[arg(long)]
    pub location: Option<String>,
    /// 是否全天事项：0 或 1（默认 0）
    #[arg(long, allow_hyphen_values = true)]
    pub all_day: Option<i64>,
    /// 从 JSON 文件或 stdin(-) 读取完整输入；提供后忽略其他业务参数
    #[arg(long, value_name = "PATH|-")]
    pub input_json: Option<String>,
}

/// 查询单条参数。
#[derive(Args, Debug, Deserialize)]
pub struct IdArgs {
    /// 日程 ID
    #[arg(long)]
    pub id: String,
}

/// 列表查询参数（区间相交规则：[from, to)）。
#[derive(Args, Debug, Deserialize, Default)]
#[serde(default)]
pub struct ListArgs {
    /// 查询区间起点（含）
    #[arg(long)]
    pub from: Option<String>,
    /// 查询区间终点（不含）
    #[arg(long)]
    pub to: Option<String>,
    /// 标题关键词（包含匹配）
    #[arg(long)]
    pub keyword: Option<String>,
}

/// 修改参数。
#[derive(Args, Debug, Deserialize, Default)]
#[serde(default)]
pub struct UpdateArgs {
    /// 日程 ID（必填）
    #[arg(long, required_unless_present = "input_json")]
    pub id: Option<String>,
    /// 新标题
    #[arg(long)]
    pub title: Option<String>,
    /// 新描述
    #[arg(long)]
    pub description: Option<String>,
    /// 新开始时间
    #[arg(long)]
    pub start_at: Option<String>,
    /// 新结束时间
    #[arg(long)]
    pub end_at: Option<String>,
    /// 新地点
    #[arg(long)]
    pub location: Option<String>,
    /// 新的全天标记：0 或 1
    #[arg(long, allow_hyphen_values = true)]
    pub all_day: Option<i64>,
    /// 显式清空描述
    #[arg(long)]
    pub clear_description: bool,
    /// 显式清空结束时间
    #[arg(long)]
    pub clear_end_at: bool,
    /// 显式清空地点
    #[arg(long)]
    pub clear_location: bool,
    /// 从 JSON 文件或 stdin(-) 读取修改内容；提供后忽略其他业务参数
    #[arg(long, value_name = "PATH|-")]
    pub input_json: Option<String>,
}

/// 日程命令入口。
pub fn run(cli: &Cli, command: &ScheduleCommand) -> Result<CliOutput, AppError> {
    match command {
        ScheduleCommand::Create(args) => run_create(cli, args),
        ScheduleCommand::Get(args) => run_get(cli, args),
        ScheduleCommand::List(args) => run_list(cli, args),
        ScheduleCommand::Update(args) => run_update(cli, args),
        ScheduleCommand::Delete(args) => run_delete(cli, args),
    }
}

fn run_create(cli: &Cli, args: &CreateArgs) -> Result<CliOutput, AppError> {
    let args = effective_create_args(args)?;
    let input = CreateEventInput {
        title: args.title,
        description: args.description,
        start_at: args.start_at,
        end_at: args.end_at,
        location: args.location,
        all_day: args.all_day,
    };

    let db = cli.open_db()?;
    let user_id = crate::cli::require_user(&db, cli)?;
    let item = item_service::create_event(db.connection(), &user_id, input)?;
    single_item_output(item, "已创建日程")
}

fn run_get(cli: &Cli, args: &IdArgs) -> Result<CliOutput, AppError> {
    let db = cli.open_db()?;
    let user_id = crate::cli::require_user(&db, cli)?;
    let item = item_service::get_event(db.connection(), &user_id, &args.id)?;
    single_item_output(item, "日程")
}

fn run_list(cli: &Cli, args: &ListArgs) -> Result<CliOutput, AppError> {
    let input = ListEventsInput {
        from: args.from.clone(),
        to: args.to.clone(),
        keyword: args.keyword.clone(),
    };

    let db = cli.open_db()?;
    let user_id = crate::cli::require_user(&db, cli)?;
    let items = item_service::list_events(db.connection(), &user_id, &input)?;
    list_output(items, "没有符合条件的日程")
}

fn run_update(cli: &Cli, args: &UpdateArgs) -> Result<CliOutput, AppError> {
    let args = effective_update_args(args)?;
    let input = UpdateEventInput {
        id: args.id.unwrap_or_default(),
        title: args.title,
        description: args.description,
        start_at: args.start_at,
        end_at: args.end_at,
        location: args.location,
        all_day: args.all_day,
        clear_description: args.clear_description,
        clear_end_at: args.clear_end_at,
        clear_location: args.clear_location,
    };

    let db = cli.open_db()?;
    let user_id = crate::cli::require_user(&db, cli)?;
    let item = item_service::update_event(db.connection(), &user_id, input)?;
    single_item_output(item, "已更新日程")
}

fn run_delete(cli: &Cli, args: &IdArgs) -> Result<CliOutput, AppError> {
    let db = cli.open_db()?;
    let user_id = crate::cli::require_user(&db, cli)?;
    item_service::delete_event(db.connection(), &user_id, &args.id)?;
    let id = args.id.trim();
    let data = serde_json::json!({ "id": id, "deleted": true });
    Ok(CliOutput::new(data, format!("已删除日程: {id}")))
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
        start_at: args.start_at.clone(),
        end_at: args.end_at.clone(),
        location: args.location.clone(),
        all_day: args.all_day,
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
        start_at: args.start_at.clone(),
        end_at: args.end_at.clone(),
        location: args.location.clone(),
        all_day: args.all_day,
        clear_description: args.clear_description,
        clear_end_at: args.clear_end_at,
        clear_location: args.clear_location,
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

/// 列表的人类可读单行摘要。
fn human_line(item: &Item) -> String {
    let mut line = format!(
        "- {} [{}/{}] {}",
        item.id,
        item.item_type.as_str(),
        item.status.as_str(),
        item.title
    );
    if let Some(start_at) = &item.start_at {
        line.push_str(&format!(" start={start_at}"));
    }
    if let Some(end_at) = &item.end_at {
        line.push_str(&format!(" end={end_at}"));
    }
    if let Some(location) = &item.location {
        line.push_str(&format!(" @{location}"));
    }
    if item.all_day {
        line.push_str(" 全天");
    }
    line
}
