//! CLI 入口：参数解析、命令分发、输出模式与错误处理。
//!
//! 输出约定（见设计方案 6.3/6.4 节）：
//! - 默认人类可读模式；`--json` 时 stdout 只输出一个合法 JSON 对象；
//! - 成功退出码 0；参数/输入错误退出码 2；业务与存储错误退出码 1；
//! - 即使参数解析失败，只要请求了 `--json`，也输出统一错误 JSON 到 stdout。
//!
//! 用户隔离约定（见数据表结构设计第 4.1 节）：
//! - 所有业务命令都必须通过全局 `--user-id <USER_ID>` 指定当前用户；
//! - 所有查询按 `user_id` 过滤，跨用户访问表现为“记录不存在”；
//! - 用户可通过 `paadm user create/resolve` 创建或解析。

use std::io::Read;
use std::path::PathBuf;

use clap::{Parser, Subcommand};
use serde::de::DeserializeOwned;

use crate::error::AppError;
use crate::output::response;
use crate::storage::Database;

pub mod note;
pub mod reminder;
pub mod schedule;
pub mod task;
pub mod transaction;
pub mod user;

/// 一个命令的执行结果：既包含 JSON 数据，也包含人类可读摘要。
pub struct CliOutput {
    /// JSON 模式下写入 `data` 字段的内容。
    pub data: serde_json::Value,
    /// 人类可读模式下直接打印的内容。
    pub human: String,
}

impl CliOutput {
    pub fn new(data: serde_json::Value, human: impl Into<String>) -> Self {
        Self {
            data,
            human: human.into(),
        }
    }
}

/// 顶层命令行参数。
#[derive(Parser, Debug)]
#[command(
    name = "paadm",
    version,
    about = "PAA 个人事务数据管理 CLI（供 Agent 与用户本人使用）"
)]
pub struct Cli {
    /// 输出机器可读的 JSON（供 Agent 调用）
    #[arg(long, global = true)]
    pub json: bool,

    /// SQLite 数据库路径（优先于 PAADM_DB_PATH / PAADM_DATA_DIR）
    #[arg(long, global = true, value_name = "PATH")]
    pub db: Option<PathBuf>,

    /// 当前 PAA 用户 ID（`users.id`）；所有业务命令必填
    #[arg(long, global = true, value_name = "USER_ID")]
    pub user_id: Option<String>,

    #[command(subcommand)]
    pub command: Command,
}

impl Cli {
    /// 打开（必要时自动初始化）数据库。
    pub fn open_db(&self) -> Result<Database, AppError> {
        Database::open_cli(self.db.as_deref())
    }
}

/// 第一版命令树。
#[derive(Subcommand, Debug)]
pub enum Command {
    /// 初始化数据库（其它命令首次启动时也会自动初始化）
    Init,
    /// 打印版本信息
    Version,
    /// 用户管理（创建 / 查询 / 按外部身份解析）
    User {
        #[command(subcommand)]
        command: user::UserCommand,
    },
    /// 待办事项（items 表中 type=task 的记录）
    Task {
        #[command(subcommand)]
        command: task::TaskCommand,
    },
    /// 日程（items 表中 type=event 的记录）
    Schedule {
        #[command(subcommand)]
        command: schedule::ScheduleCommand,
    },
    /// 提醒记录
    Reminder {
        #[command(subcommand)]
        command: reminder::ReminderCommand,
    },
    /// 收支记录
    Transaction {
        #[command(subcommand)]
        command: transaction::TransactionCommand,
    },
    /// 笔记/临时记录
    Note {
        #[command(subcommand)]
        command: note::NoteCommand,
    },
}

/// 进程入口：解析参数 -> 执行 -> 输出 -> 返回退出码。
pub fn run() -> i32 {
    let raw_args: Vec<String> = std::env::args().collect();
    let json_requested = raw_args.iter().any(|arg| arg == "--json");

    match Cli::try_parse_from(&raw_args) {
        Ok(cli) => execute(cli),
        Err(err) => handle_parse_error(err, json_requested),
    }
}

/// 处理 clap 的参数解析错误：帮助/版本正常退出，其余输出统一错误。
fn handle_parse_error(err: clap::Error, json_requested: bool) -> i32 {
    use clap::error::ErrorKind;

    match err.kind() {
        // --help / --version 属于正常输出，退出码 0。
        ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => {
            print!("{err}");
            0
        }
        _ => {
            if json_requested {
                // 参数错误也必须符合 JSON 协议，方便 Agent 稳定识别。
                response::print_failure_json(&AppError::invalid_argument(
                    "invalid command line arguments",
                ));
            } else {
                eprint!("{err}");
            }
            2
        }
    }
}

/// 执行已解析的命令，并把结果按输出模式打印。
fn execute(cli: Cli) -> i32 {
    let json_mode = cli.json;
    match dispatch(&cli) {
        Ok(output) => {
            if json_mode {
                if let Err(err) = response::print_success_json(&output.data) {
                    eprintln!("failed to write JSON response: {err}");
                    return 1;
                }
            } else {
                println!("{}", output.human);
            }
            0
        }
        Err(err) => {
            if json_mode {
                response::print_failure_json(&err);
            } else {
                eprintln!("错误 [{}]: {}", err.code, err.message);
            }
            err.exit_code()
        }
    }
}

/// 命令分发。
fn dispatch(cli: &Cli) -> Result<CliOutput, AppError> {
    match &cli.command {
        Command::Init => run_init(cli),
        Command::Version => run_version(),
        Command::User { command } => user::run(cli, command),
        Command::Task { command } => task::run(cli, command),
        Command::Schedule { command } => schedule::run(cli, command),
        Command::Reminder { command } => reminder::run(cli, command),
        Command::Transaction { command } => transaction::run(cli, command),
        Command::Note { command } => note::run(cli, command),
    }
}

/// 业务命令统一获取并校验当前用户。
///
/// - 未提供 `--user-id` → `INVALID_ARGUMENT`（退出码 2）；
/// - 用户不存在 → `USER_NOT_FOUND`（退出码 1）。
pub fn require_user(db: &Database, cli: &Cli) -> Result<String, AppError> {
    let user_id = cli
        .user_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| AppError::invalid_argument("--user-id is required for business commands"))?
        .to_string();
    crate::services::user::ensure_exists(db.connection(), &user_id)?;
    Ok(user_id)
}

/// `paadm init`：显式初始化数据库（幂等）。
fn run_init(cli: &Cli) -> Result<CliOutput, AppError> {
    let db = cli.open_db()?;
    let path = db.path().display().to_string();
    let schema_version = db.schema_version()?;
    let data = serde_json::json!({
        "db_path": path,
        "schema_version": schema_version,
    });
    let human = format!("数据库已就绪: {path} (schema v{schema_version})");
    Ok(CliOutput::new(data, human))
}

/// `paadm version`：输出名称与版本号（与 `--version` 等价，但可 JSON 化）。
fn run_version() -> Result<CliOutput, AppError> {
    let version = env!("CARGO_PKG_VERSION");
    let data = serde_json::json!({
        "name": "paadm",
        "version": version,
    });
    Ok(CliOutput::new(data, format!("paadm {version}")))
}

/// 读取 `--input-json` 参数指定的 JSON 输入；`-` 表示从 stdin 读取。
///
/// 约定：一旦使用 `--input-json`，其余业务参数被忽略，JSON 中的字段与
/// CLI 参数同名（例如 `{"title": "...", "due_at": "..."}`）。
pub fn load_input_json<T: DeserializeOwned>(source: &str) -> Result<T, AppError> {
    let text = if source == "-" {
        let mut buffer = String::new();
        std::io::stdin()
            .read_to_string(&mut buffer)
            .map_err(|err| AppError::invalid_input(format!("failed to read stdin: {err}")))?;
        buffer
    } else {
        std::fs::read_to_string(source).map_err(|err| {
            AppError::invalid_input(format!("failed to read JSON input file: {err}"))
        })?
    };

    serde_json::from_str(&text).map_err(|err| {
        // 截断/格式错误都返回明确的输入错误，且不会写入任何数据。
        AppError::invalid_input(format!("invalid JSON input: {err}"))
    })
}
