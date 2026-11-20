//! Note（笔记/临时记录）CLI 命令定义。
//!
//! 第一版只提供笔记的增删改查，不加入分类、标签、关联关系。
//! 表结构以 `docs/design/paadm数据表结构设计.md` 为唯一权威来源：
//! 所有命令先打开数据库，再通过 `crate::cli::require_user` 取得当前用户，
//! 所有读写都按 `user_id` 隔离。

use clap::{Args, Subcommand};
use serde::Deserialize;
use serde_json::json;

use crate::cli::{Cli, CliOutput};
use crate::domain::note::Note;
use crate::error::AppError;
use crate::services::note as note_service;

#[derive(Subcommand, Debug)]
pub enum NoteCommand {
    /// 创建笔记
    Create(CreateArgs),
    /// 按 ID 查询笔记
    Get(IdArgs),
    /// 查询笔记列表
    List(ListArgs),
    /// 修改笔记
    Update(UpdateArgs),
    /// 删除笔记
    Delete(IdArgs),
}

/// 创建笔记参数。
#[derive(Args, Debug, Deserialize, Default)]
#[serde(default)]
pub struct CreateArgs {
    /// 标题（可选）
    #[arg(long)]
    pub title: Option<String>,
    /// 正文内容（必填，去空白后不能为空）
    #[arg(long, required_unless_present = "input_json")]
    pub content: Option<String>,
    /// 从 JSON 文件或 stdin(-) 读取完整输入；提供后忽略其他业务参数
    #[arg(long, value_name = "PATH|-")]
    pub input_json: Option<String>,
}

/// 查询单条参数。
#[derive(Args, Debug, Deserialize)]
pub struct IdArgs {
    /// 笔记 ID
    #[arg(long)]
    pub id: String,
}

/// 列表查询参数。
#[derive(Args, Debug, Deserialize, Default)]
#[serde(default)]
pub struct ListArgs {
    /// 内容/标题关键词（包含匹配）
    #[arg(long)]
    pub keyword: Option<String>,
    /// 创建时间范围起点（含）
    #[arg(long)]
    pub from: Option<String>,
    /// 创建时间范围终点（含）
    #[arg(long)]
    pub to: Option<String>,
}

/// 修改参数。
#[derive(Args, Debug, Deserialize, Default)]
#[serde(default)]
pub struct UpdateArgs {
    /// 笔记 ID（必填）
    #[arg(long, required_unless_present = "input_json")]
    pub id: Option<String>,
    /// 新标题
    #[arg(long)]
    pub title: Option<String>,
    /// 新正文
    #[arg(long)]
    pub content: Option<String>,
    /// 显式清空标题
    #[arg(long)]
    pub clear_title: bool,
    /// 从 JSON 文件或 stdin(-) 读取修改内容；提供后忽略其他业务参数
    #[arg(long, value_name = "PATH|-")]
    pub input_json: Option<String>,
}

/// 笔记命令入口。
pub fn run(cli: &Cli, command: &NoteCommand) -> Result<CliOutput, AppError> {
    let db = cli.open_db()?;
    let user_id = crate::cli::require_user(&db, cli)?;

    match command {
        NoteCommand::Create(args) => run_create(&db, &user_id, args),
        NoteCommand::Get(args) => run_get(&db, &user_id, args),
        NoteCommand::List(args) => run_list(&db, &user_id, args),
        NoteCommand::Update(args) => run_update(&db, &user_id, args),
        NoteCommand::Delete(args) => run_delete(&db, &user_id, args),
    }
}

/// `note create`：创建笔记，`data` 直接是记录对象。
fn run_create(
    db: &crate::storage::Database,
    user_id: &str,
    args: &CreateArgs,
) -> Result<CliOutput, AppError> {
    let args = resolve_create_args(args)?;
    let note = note_service::create(
        db.connection(),
        user_id,
        args.title.as_deref(),
        args.content.as_deref(),
    )?;

    let human = format!("已创建笔记 {}：{}", note.id, preview(&note.content));
    Ok(CliOutput::new(note_to_json(&note)?, human))
}

/// `note get`：按 ID 查询，`data` 直接是记录对象。
fn run_get(
    db: &crate::storage::Database,
    user_id: &str,
    args: &IdArgs,
) -> Result<CliOutput, AppError> {
    let note = note_service::get(db.connection(), user_id, &args.id)?;

    let title = note.title.as_deref().unwrap_or("（无标题）");
    let human = format!(
        "笔记 {}\n标题：{title}\n正文：{}",
        note.id,
        preview(&note.content)
    );
    Ok(CliOutput::new(note_to_json(&note)?, human))
}

/// `note list`：按条件查询，`data = {"items": [...], "count": N}`。
fn run_list(
    db: &crate::storage::Database,
    user_id: &str,
    args: &ListArgs,
) -> Result<CliOutput, AppError> {
    let notes = note_service::list(
        db.connection(),
        user_id,
        args.keyword.as_deref(),
        args.from.as_deref(),
        args.to.as_deref(),
    )?;

    let count = notes.len();
    // 人类可读模式：只打印 id、标题与正文摘要，完整内容由 JSON 模式提供。
    let human = if notes.is_empty() {
        "没有找到笔记".to_string()
    } else {
        let lines: Vec<String> = notes
            .iter()
            .map(|note| {
                let title = note.title.as_deref().unwrap_or("（无标题）");
                format!("- {}  {title}：{}", note.id, preview(&note.content))
            })
            .collect();
        format!("共 {count} 条笔记\n{}", lines.join("\n"))
    };

    let data = json!({ "items": notes, "count": count });
    Ok(CliOutput::new(data, human))
}

/// `note update`：修改笔记，`data` 是修改后的记录对象。
fn run_update(
    db: &crate::storage::Database,
    user_id: &str,
    args: &UpdateArgs,
) -> Result<CliOutput, AppError> {
    let args = resolve_update_args(args)?;
    let id = args
        .id
        .as_deref()
        .ok_or_else(|| AppError::invalid_argument("id is required"))?;

    let note = note_service::update(
        db.connection(),
        user_id,
        id,
        args.title.as_deref(),
        args.content.as_deref(),
        args.clear_title,
    )?;

    let human = format!("已更新笔记 {}：{}", note.id, preview(&note.content));
    Ok(CliOutput::new(note_to_json(&note)?, human))
}

/// `note delete`：删除笔记，`data = {"id": "...", "deleted": true}`。
fn run_delete(
    db: &crate::storage::Database,
    user_id: &str,
    args: &IdArgs,
) -> Result<CliOutput, AppError> {
    note_service::delete(db.connection(), user_id, &args.id)?;

    let data = json!({ "id": args.id, "deleted": true });
    let human = format!("已删除笔记 {}", args.id);
    Ok(CliOutput::new(data, human))
}

/// 若提供了 `--input-json`，完整读取 JSON 输入；否则复制 CLI 参数。
fn resolve_create_args(args: &CreateArgs) -> Result<CreateArgs, AppError> {
    match &args.input_json {
        Some(source) => crate::cli::load_input_json::<CreateArgs>(source),
        None => Ok(CreateArgs {
            title: args.title.clone(),
            content: args.content.clone(),
            input_json: None,
        }),
    }
}

/// 若提供了 `--input-json`，完整读取 JSON 输入；否则复制 CLI 参数。
fn resolve_update_args(args: &UpdateArgs) -> Result<UpdateArgs, AppError> {
    match &args.input_json {
        Some(source) => crate::cli::load_input_json::<UpdateArgs>(source),
        None => Ok(UpdateArgs {
            id: args.id.clone(),
            title: args.title.clone(),
            content: args.content.clone(),
            clear_title: args.clear_title,
            input_json: None,
        }),
    }
}

/// 领域对象 → JSON 值（保留 `title` 的 null 语义）。
fn note_to_json(note: &Note) -> Result<serde_json::Value, AppError> {
    serde_json::to_value(note).map_err(|_| AppError::internal("failed to serialize note"))
}

/// 人类可读模式下的正文摘要（按字符截断，避免超长输出）。
fn preview(content: &str) -> String {
    const MAX_CHARS: usize = 40;
    let mut chars = content.chars();
    let preview: String = chars.by_ref().take(MAX_CHARS).collect();
    if chars.next().is_some() {
        format!("{preview}…")
    } else {
        preview
    }
}
