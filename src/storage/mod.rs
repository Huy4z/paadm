//! 存储层：SQLite 连接、路径解析、PRAGMA 配置与迁移。
//!
//! 数据库路径解析优先级（见设计方案 5.1 节）：
//! 1. CLI `--db <PATH>`
//! 2. 环境变量 `PAADM_DB_PATH`
//! 3. 环境变量 `PAADM_DATA_DIR` 下的 `paa.db`
//! 4. `$XDG_DATA_HOME/paa/paa.db`
//! 5. `$HOME/.local/share/paa/paa.db`
//!
//! 所有命令在打开数据库时都会自动执行迁移，因此“首次启动自动初始化”是天然满足的。

use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::Connection;

use crate::error::AppError;

pub mod migrations;

/// 已打开的数据库连接。
pub struct Database {
    conn: Connection,
    path: PathBuf,
}

impl Database {
    /// 根据 CLI 参数 / 环境变量打开数据库（自动创建目录、初始化与迁移）。
    pub fn open_cli(explicit_path: Option<&Path>) -> Result<Self, AppError> {
        let path = resolve_db_path(explicit_path)?;
        Self::open_path(&path)
    }

    /// 打开指定路径的数据库（测试中直接使用）。
    pub fn open_path(path: &Path) -> Result<Self, AppError> {
        // 路径来自明确配置，不允许通过路径覆盖系统文件；这里只负责创建父目录。
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent)
                .map_err(|_| AppError::database("failed to create database directory"))?;
        }

        let mut conn = Connection::open(path)
            .map_err(|_| AppError::database("failed to open database file"))?;

        configure(&conn)?;
        migrations::run(&mut conn)?;

        Ok(Self {
            conn,
            path: path.to_path_buf(),
        })
    }

    /// 数据库文件路径。
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 只读连接（查询/写入均可，但无法开启显式事务）。
    pub fn connection(&self) -> &Connection {
        &self.conn
    }

    /// 可变连接（需要显式事务时使用）。
    pub fn connection_mut(&mut self) -> &mut Connection {
        &mut self.conn
    }

    /// 当前已应用的最大迁移版本。
    pub fn schema_version(&self) -> Result<i64, AppError> {
        let version: Option<i64> = self
            .conn
            .query_row("SELECT MAX(version) FROM schema_migrations", [], |row| {
                row.get(0)
            })
            .map_err(AppError::from_sqlite)?;
        Ok(version.unwrap_or(0))
    }
}

/// 设置连接参数：外键、WAL、busy timeout、同步级别。
fn configure(conn: &Connection) -> Result<(), AppError> {
    conn.busy_timeout(Duration::from_millis(5_000))
        .map_err(|_| AppError::database("failed to configure database busy timeout"))?;
    conn.execute_batch(
        r#"
PRAGMA foreign_keys = ON;
PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;
"#,
    )
    .map_err(|_| AppError::database("failed to configure database connection"))?;
    Ok(())
}

/// 解析数据库文件路径。
pub fn resolve_db_path(explicit_path: Option<&Path>) -> Result<PathBuf, AppError> {
    if let Some(path) = explicit_path {
        return Ok(path.to_path_buf());
    }

    match std::env::var("PAADM_DB_PATH") {
        Ok(path) if !path.trim().is_empty() => return Ok(PathBuf::from(path)),
        _ => {}
    }

    match std::env::var("PAADM_DATA_DIR") {
        Ok(dir) if !dir.trim().is_empty() => return Ok(PathBuf::from(dir).join("paa.db")),
        _ => {}
    }

    match std::env::var("XDG_DATA_HOME") {
        Ok(xdg) if !xdg.trim().is_empty() => {
            return Ok(PathBuf::from(xdg).join("paa").join("paa.db"));
        }
        _ => {}
    }

    match std::env::var("HOME") {
        Ok(home) if !home.trim().is_empty() => {
            return Ok(PathBuf::from(home)
                .join(".local")
                .join("share")
                .join("paa")
                .join("paa.db"));
        }
        _ => {}
    }

    Err(AppError::database(
        "cannot determine database path; set --db, PAADM_DB_PATH or PAADM_DATA_DIR",
    ))
}
