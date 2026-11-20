//! User CLI：创建用户、按 ID 查询、按外部身份解析（查找或创建）。

use clap::{Args, Subcommand};

use crate::cli::{Cli, CliOutput};
use crate::domain::user::User;
use crate::error::AppError;
use crate::services::user as user_service;

#[derive(Subcommand, Debug)]
pub enum UserCommand {
    /// 创建用户（同一平台身份重复创建会报错）
    Create(CreateArgs),
    /// 按 ID 查询用户
    Get(IdArgs),
    /// 按外部身份解析用户：存在则返回，不存在则创建
    Resolve(CreateArgs),
}

/// 创建 / 解析用户参数。
#[derive(Args, Debug)]
pub struct CreateArgs {
    /// 外部平台标识，例如 qq、wechat
    #[arg(long)]
    pub provider: String,
    /// 外部平台用户 ID
    #[arg(long)]
    pub external_user_id: String,
}

/// 查询参数。
#[derive(Args, Debug)]
pub struct IdArgs {
    /// PAA 用户 ID
    #[arg(long)]
    pub id: String,
}

/// 用户命令入口。
pub fn run(cli: &Cli, command: &UserCommand) -> Result<CliOutput, AppError> {
    let db = cli.open_db()?;
    match command {
        UserCommand::Create(args) => {
            let user =
                user_service::create(db.connection(), &args.provider, &args.external_user_id)?;
            Ok(render_user(user, None))
        }
        UserCommand::Get(args) => {
            let user = user_service::get(db.connection(), &args.id)?;
            Ok(render_user(user, None))
        }
        UserCommand::Resolve(args) => {
            let (user, created) =
                user_service::resolve(db.connection(), &args.provider, &args.external_user_id)?;
            Ok(render_user(user, Some(created)))
        }
    }
}

/// 组织用户命令输出；`created` 仅在 resolve 时返回。
fn render_user(user: User, created: Option<bool>) -> CliOutput {
    let mut data = serde_json::to_value(&user).unwrap_or_else(|_| serde_json::json!({}));
    if let (Some(created), Some(object)) = (created, data.as_object_mut()) {
        object.insert("created".to_string(), serde_json::Value::Bool(created));
    }
    let human = format!(
        "用户: {} (provider={}, external_user_id={})",
        user.id, user.provider, user.external_user_id
    );
    CliOutput::new(data, human)
}
