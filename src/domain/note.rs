//! note 领域模型：实体、输入结构与领域校验规则。
//!
//! 表结构以 `docs/design/paadm数据表结构设计.md` 第 2.6 节为唯一权威来源：
//! `notes(id, user_id, title, content, created_at, updated_at)`。
//! 第一版**没有**分类、标签、关联关系、树形结构或向量检索。
//!
//! 规则：
//! - `content` 必填，`trim` 后不能为空，保存 `trim` 后的内容；
//! - `title` 可选；
//! - 所有数据都归属某个 `user_id`，领域层不判断归属，由 service/repository 按用户过滤；
//! - 列表排序固定为 `created_at ASC, id ASC`；
//! - 修改时未提供的字段不覆盖，`clear_title` 显式清空。

use serde::Serialize;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use crate::domain::common;
use crate::error::AppError;

/// 一条笔记。
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Note {
    /// 唯一 ID（UUID 字符串）。
    pub id: String,
    /// 所属用户 ID。
    pub user_id: String,
    /// 可选标题。
    pub title: Option<String>,
    /// 正文（`trim` 后保存；设计上保留内容本身，不擅自改写）。
    pub content: String,
    /// 创建时间（UTC，RFC 3339）。
    pub created_at: String,
    /// 更新时间（UTC，RFC 3339）。
    pub updated_at: String,
}

/// 创建笔记时经过校验的输入。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewNote {
    pub title: Option<String>,
    pub content: String,
}

impl NewNote {
    /// 校验并规范化创建输入：正文必填，标题可选（空字符串按未提供处理）。
    pub fn parse(title: Option<&str>, content: Option<&str>) -> Result<Self, AppError> {
        let content = content.ok_or_else(|| AppError::invalid_argument("content is required"))?;
        Ok(Self {
            title: common::validate_optional_text("title", title)?,
            content: common::validate_required_text("content", content)?,
        })
    }
}

/// 修改笔记的可选字段；`clear_title` 表示显式清空标题。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NoteChanges {
    /// 外层 `Some` 表示提供了该字段；内层 `None` 表示清空（例如 `--title ""`）。
    pub title: Option<Option<String>>,
    /// 提供时必须是合法正文（正文不允许清空）。
    pub content: Option<String>,
    /// 显式清空标题。
    pub clear_title: bool,
}

impl NoteChanges {
    /// 校验并规范化修改输入。
    pub fn parse(
        title: Option<&str>,
        content: Option<&str>,
        clear_title: bool,
    ) -> Result<Self, AppError> {
        Ok(Self {
            title: match title {
                Some(raw) => Some(common::validate_optional_text("title", Some(raw))?),
                None => None,
            },
            content: match content {
                Some(raw) => Some(common::validate_required_text("content", raw)?),
                None => None,
            },
            clear_title,
        })
    }

    /// 基于现有记录计算修改后的记录；未提供的字段保持不变。
    ///
    /// `clear_title` 优先于同时提供的 `title` 值。
    pub fn apply(&self, current: &Note, updated_at: String) -> Note {
        let mut updated = current.clone();
        if self.clear_title {
            updated.title = None;
        } else if let Some(title) = &self.title {
            updated.title = title.clone();
        }
        if let Some(content) = &self.content {
            updated.content = content.clone();
        }
        updated.updated_at = updated_at;
        updated
    }
}

/// 列表查询条件（已完成校验的规范化值）。
///
/// 时间范围是闭区间，比较由 SQL 的 `julianday()` 完成，因此这里保留用户
/// 输入的原始 ISO 8601 字符串（保留时区偏移，不自行归一化）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NoteQuery {
    /// 关键词：匹配 `title` 或 `content` 的包含关系；纯空白视为未提供。
    pub keyword: Option<String>,
    /// 创建时间范围起点（ISO 8601，含）。
    pub from: Option<String>,
    /// 创建时间范围终点（ISO 8601，含）。
    pub to: Option<String>,
}

impl NoteQuery {
    /// 校验列表过滤参数；时间必须是带时区偏移的 ISO 8601。
    pub fn parse(
        keyword: Option<&str>,
        from: Option<&str>,
        to: Option<&str>,
    ) -> Result<Self, AppError> {
        let keyword = match keyword {
            Some(raw) => {
                let trimmed = raw.trim();
                if trimmed.is_empty() {
                    None
                } else {
                    common::validate_text_length("keyword", trimmed)?;
                    Some(trimmed.to_string())
                }
            }
            None => None,
        };

        let from = match from {
            Some(value) => Some(common::validate_datetime("from", value)?),
            None => None,
        };
        let to = match to {
            Some(value) => Some(common::validate_datetime("to", value)?),
            None => None,
        };

        // 两个边界都存在时才比较先后；只给一端时不限制区间。
        let ordering = match (&from, &to) {
            (Some(from), Some(to)) => Some(compare_datetime(from, to)?),
            _ => None,
        };
        if ordering == Some(std::cmp::Ordering::Greater) {
            return Err(AppError::invalid_argument("from must not be later than to"));
        }

        Ok(Self { keyword, from, to })
    }
}

/// 比较两个合法 ISO 8601 时间（跨时区正确）。
fn compare_datetime(left: &str, right: &str) -> Result<std::cmp::Ordering, AppError> {
    let parse = |value: &str| {
        OffsetDateTime::parse(value, &Rfc3339)
            .map_err(|_| AppError::internal("failed to parse datetime"))
    };
    Ok(parse(left)?.cmp(&parse(right)?))
}
