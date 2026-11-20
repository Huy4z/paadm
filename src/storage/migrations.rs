//! 数据库迁移。
//!
//! 表结构以 `docs/design/paadm数据表结构设计.md` 为唯一权威来源：
//! 第一版共 6 张表：`users`、`items`、`reminders`、`transactions`、`notes`、
//! `schema_migrations`。
//!
//! 迁移约定：
//! - `schema_migrations` 只记录 `version` 与 `applied_at`（与数据表设计一致）；
//! - 首次启动自动初始化；
//! - 未执行的迁移按 version 升序执行，每个迁移在独立事务中执行；
//! - 失败时整体回滚且不记录为成功；已执行的迁移不会重复执行，也不会覆盖旧数据。

use rusqlite::{Connection, params};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use crate::error::AppError;

/// 一次数据库迁移。
pub struct Migration {
    pub version: i64,
    pub sql: &'static str,
}

/// 第一版迁移列表（只追加，不修改历史迁移）。
pub const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        sql: r#"
-- 用户：PAA 内部用户 ID 与外部平台身份的映射。
-- 同一个自然人通过 QQ 和微信联系 PAA 会被视为两个独立用户。
CREATE TABLE users (
    id TEXT PRIMARY KEY,
    provider TEXT NOT NULL,
    external_user_id TEXT NOT NULL,
    created_at TEXT NOT NULL,

    UNIQUE (provider, external_user_id),
    UNIQUE (id)
);

-- 待办与日程统一存储，通过 type 区分（task / event）。
CREATE TABLE items (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,

    title TEXT NOT NULL,
    description TEXT,
    type TEXT NOT NULL CHECK (type IN ('task', 'event')),
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'completed', 'cancelled')),
    priority INTEGER,

    start_at TEXT,
    end_at TEXT,
    due_at TEXT,
    all_day INTEGER NOT NULL DEFAULT 0 CHECK (all_day IN (0, 1)),
    location TEXT,

    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    completed_at TEXT,

    UNIQUE (user_id, id),
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE,
    CHECK (
        start_at IS NULL
        OR end_at IS NULL
        OR end_at > start_at
    )
);

-- 提醒：item_id 为空表示独立提醒；非空表示关联某个待办/日程。
-- 使用 (user_id, item_id) 复合外键，防止提醒关联到其他用户的事项。
CREATE TABLE reminders (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    item_id TEXT,

    content TEXT NOT NULL,
    remind_at TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'triggered', 'cancelled')),

    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,

    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE,
    FOREIGN KEY (user_id, item_id)
        REFERENCES items(user_id, id)
        ON DELETE RESTRICT
);

-- 收入与支出统一存储，通过 type 区分；amount 为人民币分，始终为正数。
CREATE TABLE transactions (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,

    type TEXT NOT NULL CHECK (type IN ('income', 'expense')),
    amount INTEGER NOT NULL CHECK (amount > 0),
    category TEXT,
    description TEXT,

    occurred_at TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,

    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
);

-- 笔记：第一版不含分类、标签、关联关系。
CREATE TABLE notes (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,

    title TEXT,
    content TEXT NOT NULL,

    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,

    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
);
"#,
    },
    Migration {
        version: 2,
        sql: r#"
-- 第一版索引（独立迁移，便于验证旧库升级时补齐索引且保留数据）。
CREATE INDEX idx_items_user_status
    ON items(user_id, status);

CREATE INDEX idx_items_user_start
    ON items(user_id, start_at);

CREATE INDEX idx_items_user_due
    ON items(user_id, due_at);

CREATE INDEX idx_reminders_user_status_time
    ON reminders(user_id, status, remind_at);

CREATE INDEX idx_reminders_user_item
    ON reminders(user_id, item_id);

CREATE INDEX idx_transactions_user_time
    ON transactions(user_id, occurred_at);

CREATE INDEX idx_transactions_user_category
    ON transactions(user_id, category);

CREATE INDEX idx_notes_user_created
    ON notes(user_id, created_at);
"#,
    },
];

/// 执行所有未完成的迁移。
pub fn run(conn: &mut Connection) -> Result<(), AppError> {
    // 版本表本身是引导表，先确保存在（字段与数据表设计一致）。
    conn.execute_batch(
        r#"
CREATE TABLE IF NOT EXISTS schema_migrations (
    version INTEGER PRIMARY KEY,
    applied_at TEXT NOT NULL
);
"#,
    )
    .map_err(|_| AppError::migration("failed to prepare schema_migrations table"))?;

    let applied_versions: Vec<i64> = {
        let mut stmt = conn
            .prepare("SELECT version FROM schema_migrations")
            .map_err(|_| AppError::migration("failed to read applied migrations"))?;
        let rows = stmt
            .query_map([], |row| row.get::<_, i64>(0))
            .map_err(|_| AppError::migration("failed to read applied migrations"))?;
        let mut versions = Vec::new();
        for row in rows {
            versions
                .push(row.map_err(|_| AppError::migration("failed to read applied migrations"))?);
        }
        versions
    };

    for migration in MIGRATIONS {
        if applied_versions.contains(&migration.version) {
            continue;
        }

        // 每个迁移单独事务：失败即回滚，不记录成功。
        let tx = conn.transaction().map_err(|_| {
            AppError::migration(format!("failed to start migration {}", migration.version))
        })?;

        tx.execute_batch(migration.sql)
            .map_err(|_| AppError::migration(format!("migration {} failed", migration.version)))?;

        let applied_at = OffsetDateTime::now_utc()
            .format(&Rfc3339)
            .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string());
        tx.execute(
            "INSERT INTO schema_migrations (version, applied_at) VALUES (?1, ?2)",
            params![migration.version, applied_at],
        )
        .map_err(|_| {
            AppError::migration(format!("failed to record migration {}", migration.version))
        })?;

        tx.commit().map_err(|_| {
            AppError::migration(format!("failed to commit migration {}", migration.version))
        })?;
    }

    Ok(())
}
