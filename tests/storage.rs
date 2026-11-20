//! 存储层集成测试：初始化、迁移、WAL、外键、事务回滚、删除规则、损坏处理。
//!
//! 表结构以 `docs/design/paadm数据表结构设计.md` 为权威来源：
//! 6 张表（users/items/reminders/transactions/notes/schema_migrations），
//! 所有业务表带 user_id，`schema_migrations` 只有 version/applied_at。
//!
//! 直接使用库 API（`paadm::storage::Database`）验证存储行为；CLI 级测试见
//! `tests/integration.rs`。每个测试使用独立临时数据库。

use std::path::PathBuf;

use paadm::error::codes;
use paadm::storage::Database;
use tempfile::TempDir;

fn temp_db_path() -> (TempDir, PathBuf) {
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join("paa.db");
    (dir, path)
}

/// 建库并插入一条测试用户，返回用户 ID。
fn seed_user(conn: &rusqlite::Connection, id: &str) {
    conn.execute(
        "INSERT INTO users (id, provider, external_user_id, created_at) \
         VALUES (?1, 'test', ?2, '2026-11-17T00:00:00Z')",
        [id, &format!("ext-{id}")],
    )
    .expect("insert user");
}

/// DB-001：首次打开自动创建数据库，且新 schema 为 6 张表 / 8 个索引 / v2。
#[test]
fn db001_first_open_initializes_new_schema() {
    let (_dir, path) = temp_db_path();
    assert!(!path.exists());

    let db = Database::open_path(&path).expect("open database");
    assert!(path.exists(), "database file should be created");
    assert_eq!(db.schema_version().unwrap(), 2);

    // 只允许 6 张业务/迁移表，不存在旧的 tasks/schedules 表。
    let mut tables: Vec<String> = db
        .connection()
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    tables.retain(|name| !name.starts_with("sqlite_"));
    assert_eq!(
        tables,
        vec![
            "items".to_string(),
            "notes".to_string(),
            "reminders".to_string(),
            "schema_migrations".to_string(),
            "transactions".to_string(),
            "users".to_string()
        ]
    );

    // 8 个第一版索引。
    let indexes: Vec<String> = db
        .connection()
        .prepare("SELECT name FROM sqlite_master WHERE type = 'index' AND name LIKE 'idx_%' ORDER BY name")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(indexes.len(), 8, "unexpected indexes: {indexes:?}");

    // schema_migrations 只有 version / applied_at 两列。
    let migration_columns: Vec<String> = db
        .connection()
        .prepare("PRAGMA table_info(schema_migrations)")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(1))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(migration_columns, vec!["version", "applied_at"]);
}

/// DB-002：重复打开/初始化是幂等的，不破坏已有数据。
#[test]
fn db002_reopen_is_idempotent_and_keeps_data() {
    let (_dir, path) = temp_db_path();
    {
        let db = Database::open_path(&path).unwrap();
        seed_user(db.connection(), "u1");
        db.connection()
            .execute_batch(
                "INSERT INTO items (id, user_id, title, type, status, created_at, updated_at) \
                 VALUES ('i1', 'u1', 'first', 'task', 'pending', '2026-11-17T00:00:00Z', '2026-11-17T00:00:00Z');",
            )
            .unwrap();
    }

    let db = Database::open_path(&path).unwrap();
    let count: i64 = db
        .connection()
        .query_row("SELECT COUNT(*) FROM items", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 1);
    assert_eq!(db.schema_version().unwrap(), 2);
    // 迁移记录不能重复写入。
    let migrations: i64 = db
        .connection()
        .query_row("SELECT COUNT(*) FROM schema_migrations", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(migrations, 2);
}

/// SYS-005：初始化后 WAL 模式与外键约束均已启用。
#[test]
fn sys005_wal_and_foreign_keys_enabled() {
    let (_dir, path) = temp_db_path();
    let db = Database::open_path(&path).unwrap();
    let mode: String = db
        .connection()
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .unwrap();
    assert_eq!(mode.to_lowercase(), "wal");
    let enabled: i64 = db
        .connection()
        .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
        .unwrap();
    assert_eq!(enabled, 1);
}

/// DB-005：旧版本数据库（仅 v1 表结构、缺索引）启动新版本时自动补迁移且保留数据。
#[test]
fn db005_upgrade_adds_indexes_and_keeps_data() {
    let (_dir, path) = temp_db_path();
    {
        let db = Database::open_path(&path).unwrap();
        seed_user(db.connection(), "u-old");
        db.connection()
            .execute_batch(
                "INSERT INTO items (id, user_id, title, type, status, created_at, updated_at) \
                 VALUES ('i-old', 'u-old', 'legacy item', 'task', 'pending', '2026-11-17T00:00:00Z', '2026-11-17T00:00:00Z');",
            )
            .unwrap();
    }

    // 模拟“旧版本数据库”：只有 v1 的表结构，v2 索引不存在。
    {
        let db = Database::open_path(&path).unwrap();
        db.connection()
            .execute_batch(
                "DELETE FROM schema_migrations WHERE version = 2;
                 DROP INDEX idx_items_user_status;
                 DROP INDEX idx_items_user_start;
                 DROP INDEX idx_items_user_due;
                 DROP INDEX idx_reminders_user_status_time;
                 DROP INDEX idx_reminders_user_item;
                 DROP INDEX idx_transactions_user_time;
                 DROP INDEX idx_transactions_user_category;
                 DROP INDEX idx_notes_user_created;",
            )
            .unwrap();
        assert_eq!(db.schema_version().unwrap(), 1);
    }

    // 新版本启动：自动执行 v2 迁移，原有数据必须保留。
    let db = Database::open_path(&path).unwrap();
    assert_eq!(db.schema_version().unwrap(), 2);
    let title: String = db
        .connection()
        .query_row("SELECT title FROM items WHERE id = 'i-old'", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(title, "legacy item");
    let indexes: i64 = db
        .connection()
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name LIKE 'idx_%'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(indexes, 8, "v2 migration must restore all indexes");
}

/// 设计方案 4.2：删除用户时业务数据通过 ON DELETE CASCADE 一并删除。
#[test]
fn fk_delete_user_cascades_business_data() {
    let (_dir, path) = temp_db_path();
    let db = Database::open_path(&path).unwrap();
    seed_user(db.connection(), "u1");
    db.connection()
        .execute_batch(
            "INSERT INTO items (id, user_id, title, type, status, created_at, updated_at) \
                 VALUES ('i1', 'u1', 'item', 'task', 'pending', '2026-11-17T00:00:00Z', '2026-11-17T00:00:00Z');
             INSERT INTO reminders (id, user_id, item_id, content, remind_at, status, created_at, updated_at) \
                 VALUES ('r1', 'u1', 'i1', 'remind', '2026-11-17T15:00:00+08:00', 'pending', \
                         '2026-11-17T00:00:00Z', '2026-11-17T00:00:00Z');
             INSERT INTO transactions (id, user_id, type, amount, occurred_at, created_at, updated_at) \
                 VALUES ('tx1', 'u1', 'expense', 1850, '2026-11-17T12:00:00+08:00', \
                         '2026-11-17T00:00:00Z', '2026-11-17T00:00:00Z');
             INSERT INTO notes (id, user_id, content, created_at, updated_at) \
                 VALUES ('n1', 'u1', 'note', '2026-11-17T00:00:00Z', '2026-11-17T00:00:00Z');
             DELETE FROM users WHERE id = 'u1';",
        )
        .unwrap();

    for table in ["items", "reminders", "transactions", "notes"] {
        let count: i64 = db
            .connection()
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 0, "{table} should be cascaded");
    }
}

/// 数据表设计 2.4：reminders 的 (user_id, item_id) 复合外键是 ON DELETE RESTRICT，
/// 直接删除被关联事项会被数据库拒绝（应用层必须先解除关联）。
#[test]
fn reminder_item_fk_restricts_direct_item_delete() {
    let (_dir, path) = temp_db_path();
    let db = Database::open_path(&path).unwrap();
    seed_user(db.connection(), "u1");
    db.connection()
        .execute_batch(
            "INSERT INTO items (id, user_id, title, type, status, created_at, updated_at) \
                 VALUES ('i1', 'u1', 'item', 'task', 'pending', '2026-11-17T00:00:00Z', '2026-11-17T00:00:00Z');
             INSERT INTO reminders (id, user_id, item_id, content, remind_at, status, created_at, updated_at) \
                 VALUES ('r1', 'u1', 'i1', 'remind', '2026-11-17T15:00:00+08:00', 'pending', \
                         '2026-11-17T00:00:00Z', '2026-11-17T00:00:00Z');",
        )
        .unwrap();

    let result = db
        .connection()
        .execute("DELETE FROM items WHERE id = 'i1'", []);
    assert!(result.is_err(), "composite FK must restrict item deletion");

    // 应用层规则：同一事务先解除关联，再删除事项。
    db.connection()
        .execute_batch(
            "BEGIN IMMEDIATE;
             UPDATE reminders SET item_id = NULL WHERE user_id = 'u1' AND item_id = 'i1';
             DELETE FROM items WHERE user_id = 'u1' AND id = 'i1';
             COMMIT;",
        )
        .unwrap();

    let task_id: Option<String> = db
        .connection()
        .query_row("SELECT item_id FROM reminders WHERE id = 'r1'", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(task_id, None, "reminder must survive with item_id NULL");
}

/// 数据表设计 4.2：删除用户的事项时，先解除提醒关联再删除，两步必须原子（失败则都回滚）。
#[test]
fn delete_item_and_unlink_reminders_is_atomic() {
    let (_dir, path) = temp_db_path();
    let db = Database::open_path(&path).unwrap();
    seed_user(db.connection(), "u1");
    db.connection()
        .execute_batch(
            "INSERT INTO items (id, user_id, title, type, status, created_at, updated_at) \
                 VALUES ('i1', 'u1', 'item', 'task', 'pending', '2026-11-17T00:00:00Z', '2026-11-17T00:00:00Z');
             INSERT INTO reminders (id, user_id, item_id, content, remind_at, status, created_at, updated_at) \
                 VALUES ('r1', 'u1', 'i1', 'remind', '2026-11-17T15:00:00+08:00', 'pending', \
                         '2026-11-17T00:00:00Z', '2026-11-17T00:00:00Z');",
        )
        .unwrap();

    // 模拟事务中途失败前回滚：提醒关联与事项都必须保持原样。
    {
        let mut conn = Database::open_path(&path).unwrap();
        let tx = conn.connection_mut().transaction().unwrap();
        tx.execute(
            "UPDATE reminders SET item_id = NULL WHERE user_id = 'u1' AND item_id = 'i1'",
            [],
        )
        .unwrap();
        tx.rollback().unwrap();
    }

    let item_count: i64 = db
        .connection()
        .query_row("SELECT COUNT(*) FROM items WHERE id = 'i1'", [], |row| {
            row.get(0)
        })
        .unwrap();
    let linked: Option<String> = db
        .connection()
        .query_row("SELECT item_id FROM reminders WHERE id = 'r1'", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(item_count, 1);
    assert_eq!(linked.as_deref(), Some("i1"));
}

/// DB-007 / REL-004：显式事务回滚时，不留下半成品数据。
#[test]
fn db007_transaction_rollback_leaves_no_partial_data() {
    let (_dir, path) = temp_db_path();
    let mut db = Database::open_path(&path).unwrap();
    seed_user(db.connection(), "u1");

    {
        let tx = db.connection_mut().transaction().unwrap();
        tx.execute_batch(
            "INSERT INTO items (id, user_id, title, type, status, created_at, updated_at) \
             VALUES ('rollback-item', 'u1', 'should disappear', 'task', 'pending', \
                     '2026-11-17T00:00:00Z', '2026-11-17T00:00:00Z');",
        )
        .unwrap();
        tx.rollback().unwrap();
    }

    let count: i64 = db
        .connection()
        .query_row("SELECT COUNT(*) FROM items", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 0, "rolled back insert must not be visible");
}

/// 数据表设计 2.2：items 的 CHECK 约束（end_at > start_at、all_day 仅 0/1）。
#[test]
fn items_check_constraints_enforced() {
    let (_dir, path) = temp_db_path();
    let db = Database::open_path(&path).unwrap();
    seed_user(db.connection(), "u1");

    let bad_end = db.connection().execute(
        "INSERT INTO items (id, user_id, title, type, status, start_at, end_at, created_at, updated_at) \
         VALUES ('bad-end', 'u1', 'bad', 'event', 'pending', '2026-11-20T11:00:00+08:00', \
                 '2026-11-20T10:00:00+08:00', '2026-11-17T00:00:00Z', '2026-11-17T00:00:00Z')",
        [],
    );
    assert!(bad_end.is_err(), "end_at <= start_at must be rejected");

    let bad_all_day = db.connection().execute(
        "INSERT INTO items (id, user_id, title, type, status, all_day, created_at, updated_at) \
         VALUES ('bad-all-day', 'u1', 'bad', 'event', 'pending', 2, \
                 '2026-11-17T00:00:00Z', '2026-11-17T00:00:00Z')",
        [],
    );
    assert!(bad_all_day.is_err(), "all_day must be 0 or 1");

    let bad_type = db.connection().execute(
        "INSERT INTO items (id, user_id, title, type, status, created_at, updated_at) \
         VALUES ('bad-type', 'u1', 'bad', 'note', 'pending', \
                 '2026-11-17T00:00:00Z', '2026-11-17T00:00:00Z')",
        [],
    );
    assert!(bad_type.is_err(), "type must be task or event");
}

/// SYS-006：通过 WAL checkpoint + 文件复制做安全备份，恢复到新路径后数据可查。
#[test]
fn sys006_backup_and_restore() {
    let (_dir, path) = temp_db_path();
    {
        let db = Database::open_path(&path).unwrap();
        seed_user(db.connection(), "u1");
        db.connection()
            .execute_batch(
                "INSERT INTO notes (id, user_id, content, created_at, updated_at) \
                 VALUES ('bk1', 'u1', 'backup me', '2026-11-17T00:00:00Z', '2026-11-17T00:00:00Z');
                 PRAGMA wal_checkpoint(TRUNCATE);",
            )
            .unwrap();
    }

    let backup_path = path.with_file_name("backup.db");
    std::fs::copy(&path, &backup_path).expect("copy database file");
    let restored = Database::open_path(&backup_path).expect("open restored database");
    let content: String = restored
        .connection()
        .query_row("SELECT content FROM notes WHERE id = 'bk1'", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(content, "backup me");
}

/// SYS-008：损坏的数据库文件必须返回明确错误，且不得被静默覆盖。
#[test]
fn sys008_corrupt_database_returns_clear_error() {
    let (_dir, path) = temp_db_path();
    let garbage = b"this is definitely not a sqlite database file........";
    std::fs::write(&path, garbage).unwrap();

    let err = match Database::open_path(&path) {
        Ok(_) => panic!("corrupt database must not open successfully"),
        Err(err) => err,
    };
    assert!(
        err.code == codes::DATABASE_ERROR || err.code == codes::MIGRATION_FAILED,
        "unexpected error code: {}",
        err.code
    );

    let after = std::fs::read(&path).unwrap();
    assert_eq!(after, garbage, "corrupt file must not be overwritten");
}

/// DB-004：数据库目录不可创建时返回明确错误，而不是报告成功。
#[test]
fn db004_unwritable_path_returns_error() {
    let dir = TempDir::new().unwrap();
    let blocker = dir.path().join("not-a-directory");
    std::fs::write(&blocker, b"file").unwrap();
    let path = blocker.join("paa.db");

    let err = match Database::open_path(&path) {
        Ok(_) => panic!("open must fail when parent path is a regular file"),
        Err(err) => err,
    };
    assert_eq!(err.code, codes::DATABASE_ERROR);
}

/// 显式路径优先，且父目录会被自动创建。
#[test]
fn explicit_path_is_used_and_parent_created() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("nested").join("deeper").join("paa.db");
    let db = Database::open_path(&path).unwrap();
    assert_eq!(db.path(), path.as_path());
    assert!(path.exists());
}
