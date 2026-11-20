//! 领域层：实体、枚举、状态与通用校验规则。
//!
//! 领域层不依赖 CLI 与具体数据库实现，便于单元测试与复用。

pub mod common;
pub mod item;
pub mod note;
pub mod reminder;
pub mod transaction;
pub mod user;
