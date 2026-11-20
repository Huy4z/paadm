//! paadm：PAA 的个人事务数据管理层。
//!
//! 分层结构：
//! - `cli`：参数解析、命令分发、输出模式；
//! - `domain`：领域实体、枚举与校验规则；
//! - `services`：业务规则与跨字段校验；
//! - `repositories`：SQLite 读写（参数化 SQL）；
//! - `storage`：数据库连接、迁移、事务；
//! - `output`：稳定的 JSON 响应结构；
//! - `error`：稳定错误码。

pub mod cli;
pub mod domain;
pub mod error;
pub mod output;
pub mod repositories;
pub mod services;
pub mod storage;
