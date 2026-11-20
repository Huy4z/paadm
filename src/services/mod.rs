//! 业务服务层：实现业务规则、跨字段校验、协调 Repository。
//!
//! 服务不处理自然语言，也不直接拼接 SQL 字符串。

pub mod item;
pub mod note;
pub mod reminder;
pub mod transaction;
pub mod user;
