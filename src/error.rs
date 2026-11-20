//! 统一错误类型。
//!
//! 设计要点（见设计方案第 10 节）：
//! - 错误码（code）是稳定的机器接口，Agent 依赖它判断错误类型；
//! - 错误信息（message）只用于人类诊断，不承诺稳定；
//! - 不向调用方暴露 SQL、堆栈、内部文件路径等实现细节。

/// 第一版稳定错误码。
pub mod codes {
    pub const INVALID_ARGUMENT: &str = "INVALID_ARGUMENT";
    pub const INVALID_INPUT: &str = "INVALID_INPUT";
    pub const TASK_NOT_FOUND: &str = "TASK_NOT_FOUND";
    pub const SCHEDULE_NOT_FOUND: &str = "SCHEDULE_NOT_FOUND";
    pub const REMINDER_NOT_FOUND: &str = "REMINDER_NOT_FOUND";
    pub const TRANSACTION_NOT_FOUND: &str = "TRANSACTION_NOT_FOUND";
    pub const NOTE_NOT_FOUND: &str = "NOTE_NOT_FOUND";
    pub const ITEM_NOT_FOUND: &str = "ITEM_NOT_FOUND";
    pub const USER_NOT_FOUND: &str = "USER_NOT_FOUND";
    pub const DUPLICATE_USER: &str = "DUPLICATE_USER";
    pub const DUPLICATE_ID: &str = "DUPLICATE_ID";
    pub const DATABASE_BUSY: &str = "DATABASE_BUSY";
    pub const DATABASE_ERROR: &str = "DATABASE_ERROR";
    pub const MIGRATION_FAILED: &str = "MIGRATION_FAILED";
    pub const INTERNAL_ERROR: &str = "INTERNAL_ERROR";
}

/// 业务错误：由错误码 + 可读信息组成。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("[{code}] {message}")]
pub struct AppError {
    /// 稳定错误码，见 [`codes`]。
    pub code: &'static str,
    /// 供人类阅读的诊断信息，不承诺稳定。
    pub message: String,
}

impl AppError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    // ---- 参数 / 输入错误：退出码 2 ----

    pub fn invalid_argument(message: impl Into<String>) -> Self {
        Self::new(codes::INVALID_ARGUMENT, message)
    }

    pub fn invalid_input(message: impl Into<String>) -> Self {
        Self::new(codes::INVALID_INPUT, message)
    }

    // ---- 业务“不存在”错误：退出码 1 ----

    pub fn task_not_found(id: &str) -> Self {
        Self::new(codes::TASK_NOT_FOUND, format!("Task '{id}' does not exist"))
    }

    pub fn schedule_not_found(id: &str) -> Self {
        Self::new(
            codes::SCHEDULE_NOT_FOUND,
            format!("Schedule '{id}' does not exist"),
        )
    }

    pub fn reminder_not_found(id: &str) -> Self {
        Self::new(
            codes::REMINDER_NOT_FOUND,
            format!("Reminder '{id}' does not exist"),
        )
    }

    pub fn transaction_not_found(id: &str) -> Self {
        Self::new(
            codes::TRANSACTION_NOT_FOUND,
            format!("Transaction '{id}' does not exist"),
        )
    }

    pub fn note_not_found(id: &str) -> Self {
        Self::new(codes::NOTE_NOT_FOUND, format!("Note '{id}' does not exist"))
    }

    pub fn item_not_found(id: &str) -> Self {
        Self::new(codes::ITEM_NOT_FOUND, format!("Item '{id}' does not exist"))
    }

    pub fn user_not_found(id: &str) -> Self {
        Self::new(codes::USER_NOT_FOUND, format!("User '{id}' does not exist"))
    }

    pub fn duplicate_user(provider: &str, external_user_id: &str) -> Self {
        Self::new(
            codes::DUPLICATE_USER,
            format!(
                "User with provider '{provider}' and external id '{external_user_id}' already exists"
            ),
        )
    }

    pub fn duplicate_id(id: &str) -> Self {
        Self::new(codes::DUPLICATE_ID, format!("Id '{id}' already exists"))
    }

    // ---- 存储 / 内部错误：退出码 1 ----

    pub fn database_busy(message: impl Into<String>) -> Self {
        Self::new(codes::DATABASE_BUSY, message)
    }

    pub fn database(message: impl Into<String>) -> Self {
        Self::new(codes::DATABASE_ERROR, message)
    }

    pub fn migration(message: impl Into<String>) -> Self {
        Self::new(codes::MIGRATION_FAILED, message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(codes::INTERNAL_ERROR, message)
    }

    /// 进程退出码约定：参数/输入错误用 2，其余业务与存储错误用 1。
    pub fn exit_code(&self) -> i32 {
        match self.code {
            codes::INVALID_ARGUMENT | codes::INVALID_INPUT => 2,
            _ => 1,
        }
    }

    /// 把 rusqlite 的原始错误转换成稳定错误码，避免把 SQL 细节暴露给调用方。
    pub fn from_sqlite(err: rusqlite::Error) -> Self {
        use rusqlite::ErrorCode;
        match err.sqlite_error_code() {
            Some(ErrorCode::DatabaseBusy) | Some(ErrorCode::DatabaseLocked) => {
                Self::database_busy("database is busy; please retry")
            }
            Some(ErrorCode::DatabaseCorrupt) | Some(ErrorCode::NotADatabase) => {
                Self::database("database file is corrupted or is not a valid SQLite database")
            }
            Some(ErrorCode::ConstraintViolation) => {
                // 约束冲突属于调用方输入问题，但不要把原始 SQL 返回给 Agent。
                Self::invalid_argument("operation violates a database constraint")
            }
            _ => Self::database("database operation failed"),
        }
    }
}

impl From<rusqlite::Error> for AppError {
    fn from(err: rusqlite::Error) -> Self {
        Self::from_sqlite(err)
    }
}

impl From<serde_json::Error> for AppError {
    fn from(err: serde_json::Error) -> Self {
        Self::invalid_input(format!("invalid JSON: {err}"))
    }
}

impl From<std::io::Error> for AppError {
    fn from(err: std::io::Error) -> Self {
        Self::database(format!("I/O error: {err}"))
    }
}
