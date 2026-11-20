//! 数据访问层：使用参数化 SQL 完成读写，并把数据库记录转换为领域对象。
//!
//! 禁止把用户输入拼进 SQL 字符串；所有值都通过参数绑定传入；
//! 所有业务查询都必须同时按 `user_id` 过滤。

pub mod item;
pub mod note;
pub mod reminder;
pub mod transaction;
pub mod user;
