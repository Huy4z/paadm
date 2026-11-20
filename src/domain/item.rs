//! item 领域模型：待办(task)与日程(event)统一实体、枚举、输入结构。
//!
//! 表结构以 `docs/design/paadm数据表结构设计.md` 为唯一权威来源：
//! - `items` 一张表同时存待办与日程，通过 `type` 区分；
//! - 每条记录都属于某个 `user_id`，所有业务查询必须按用户过滤；
//! - `priority` 为可空整数；`all_day` 数据库存 0/1，JSON 输出 bool；
//! - 时间是带时区偏移的 ISO 8601 字符串，只保存一份（不再有 `*_utc` 列）。
//!
//! 领域层只表达数据与字段级枚举，不依赖 CLI 与 SQLite。

use serde::Serialize;

use crate::error::AppError;

/// 事项类型：待办（task）/ 日程（event）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ItemType {
    Task,
    Event,
}

impl ItemType {
    /// 数据库 / 过滤条件中使用的稳定字符串。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Task => "task",
            Self::Event => "event",
        }
    }

    /// 解析数据库中的存储值；出现不可能的值视为内部错误。
    pub fn from_stored(value: &str) -> Result<Self, AppError> {
        match value {
            "task" => Ok(Self::Task),
            "event" => Ok(Self::Event),
            _ => Err(AppError::internal("stored item type is invalid")),
        }
    }
}

/// 事项状态：pending / completed / cancelled。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ItemStatus {
    Pending,
    Completed,
    Cancelled,
}

impl ItemStatus {
    /// 数据库 / JSON 中使用的稳定字符串。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Completed => "completed",
            Self::Cancelled => "cancelled",
        }
    }

    /// 解析用户输入（严格小写）；非法值 → `INVALID_ARGUMENT`。
    pub fn parse(field: &str, value: &str) -> Result<Self, AppError> {
        match value.trim() {
            "pending" => Ok(Self::Pending),
            "completed" => Ok(Self::Completed),
            "cancelled" => Ok(Self::Cancelled),
            _ => Err(AppError::invalid_argument(format!(
                "{field} must be one of: pending, completed, cancelled"
            ))),
        }
    }

    /// 解析数据库中的存储值；出现不可能的值视为内部错误。
    pub fn from_stored(value: &str) -> Result<Self, AppError> {
        match value {
            "pending" => Ok(Self::Pending),
            "completed" => Ok(Self::Completed),
            "cancelled" => Ok(Self::Cancelled),
            _ => Err(AppError::internal("stored item status is invalid")),
        }
    }
}

/// 统一事项实体；JSON 输出即此结构。
///
/// 字段顺序与数据表设计一致；`type` 为 Rust 保留字，因此字段名为 `item_type`，
/// 序列化时输出为 `"type"`。
#[derive(Debug, Clone, Serialize)]
pub struct Item {
    pub id: String,
    pub user_id: String,
    pub title: String,
    pub description: Option<String>,
    #[serde(rename = "type")]
    pub item_type: ItemType,
    pub status: ItemStatus,
    pub priority: Option<i64>,
    pub start_at: Option<String>,
    pub end_at: Option<String>,
    pub due_at: Option<String>,
    pub all_day: bool,
    pub location: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub completed_at: Option<String>,
}

// ---------------------------------------------------------------------------
// 待办（task）输入结构
// ---------------------------------------------------------------------------

/// 创建待办的原始输入（由 Service 校验后再落库）。
#[derive(Debug, Clone, Default)]
pub struct CreateTaskInput {
    pub title: Option<String>,
    pub description: Option<String>,
    pub priority: Option<i64>,
    pub due_at: Option<String>,
}

/// 修改待办的原始输入；`None` 表示未提供、不修改，`clear_*` 表示显式清空。
#[derive(Debug, Clone, Default)]
pub struct UpdateTaskInput {
    pub id: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub priority: Option<i64>,
    pub due_at: Option<String>,
    pub clear_description: bool,
    pub clear_priority: bool,
    pub clear_due_at: bool,
}

/// 待办列表查询的原始过滤条件。
#[derive(Debug, Clone, Default)]
pub struct ListTasksInput {
    pub status: Option<String>,
    pub priority: Option<i64>,
    pub due_from: Option<String>,
    pub due_to: Option<String>,
    pub keyword: Option<String>,
}

// ---------------------------------------------------------------------------
// 日程（event）输入结构
// ---------------------------------------------------------------------------

/// 创建日程的原始输入。
#[derive(Debug, Clone, Default)]
pub struct CreateEventInput {
    pub title: Option<String>,
    pub description: Option<String>,
    pub start_at: Option<String>,
    pub end_at: Option<String>,
    pub location: Option<String>,
    /// 原始 0/1 值，由 Service 调用 `common::validate_all_day` 校验。
    pub all_day: Option<i64>,
}

/// 修改日程的原始输入；`None` 表示未提供、不修改，`clear_*` 表示显式清空。
#[derive(Debug, Clone, Default)]
pub struct UpdateEventInput {
    pub id: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub start_at: Option<String>,
    pub end_at: Option<String>,
    pub location: Option<String>,
    pub all_day: Option<i64>,
    pub clear_description: bool,
    pub clear_end_at: bool,
    pub clear_location: bool,
}

/// 日程列表查询的原始过滤条件（区间规则 `[from, to)`）。
#[derive(Debug, Clone, Default)]
pub struct ListEventsInput {
    pub from: Option<String>,
    pub to: Option<String>,
    pub keyword: Option<String>,
}
