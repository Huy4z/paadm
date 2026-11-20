//! Lead 负责的跨领域集成测试：CLI 契约、用户隔离、items 统一表语义、数据库行为、
//! Agent 调用契约、跨领域独立性与并发/健壮性。
//!
//! 表结构以 `docs/design/paadm数据表结构设计.md` 为准。
//! 覆盖测试用例：CLI-001..010、DB-001..006、SYS-001..008、AGENT-001..010、
//! REL-005/006，以及新增的 USER-001..004 与 items 统一表语义。
//! 领域内部用例见各 `tests/<domain>.rs`。

mod common;

use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use common::{TestDb, assert_error_code, id_of, parse_json};
use serde_json::Value;
use tempfile::TempDir;

/// 构造一个不依附 TestDb 的 paadm 子进程（用于 env 变量/异常路径测试）。
fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_paadm"))
}

/// 构造一个带 --db/--json/--user-id 的子进程，用于需要自行控制生命周期的测试。
fn spawn_json(db_path: &Path, user_id: &str, args: &[&str]) -> std::process::Child {
    let mut cmd = bin();
    cmd.arg("--db")
        .arg(db_path)
        .arg("--json")
        .arg("--user-id")
        .arg(user_id)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_remove("PAADM_DB_PATH")
        .env_remove("PAADM_DATA_DIR");
    cmd.spawn().expect("spawn paadm")
}

/// 解析子进程 stdout 并断言成功，返回 data。
fn child_data(child: std::process::Child) -> Value {
    let output = child.wait_with_output().expect("wait paadm");
    common::assert_success(&output, &[]);
    parse_json(&output)["data"].clone()
}

// ---------------------------------------------------------------------------
// CLI 契约
// ---------------------------------------------------------------------------

/// CLI-001：`paadm --help` 正常退出并展示可用命令。
#[test]
fn cli001_help_lists_commands() {
    let db = TestDb::new();
    let output = db.raw(&["--help"]);
    assert!(output.status.success());
    let text = String::from_utf8_lossy(&output.stdout);
    for expected in [
        "init",
        "version",
        "user",
        "task",
        "schedule",
        "reminder",
        "transaction",
        "note",
    ] {
        assert!(text.contains(expected), "help should mention {expected}");
    }
}

/// CLI-002：`paadm --version` 正常退出版本号；`paadm version --json` 输出结构化版本。
#[test]
fn cli002_version() {
    let db = TestDb::new();
    let output = db.raw(&["--version"]);
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("paadm"));

    let data = db.ok_json(&["version"]);
    assert_eq!(data["name"], "paadm");
    assert!(data["version"].is_string());
}

/// CLI-003：未知命令非零退出且不崩溃。
#[test]
fn cli003_unknown_command_fails_gracefully() {
    let db = TestDb::new();
    let output = db.raw(&["no-such-command"]);
    assert!(!output.status.success());
    assert!(output.status.code().is_some(), "process must exit normally");
}

/// CLI-004：缺少必需参数返回参数错误，且不写入数据。
#[test]
fn cli004_missing_required_argument() {
    let db = TestDb::new();
    let (error, code) = db.fail_json(&["task", "create"]);
    assert_error_code(&error, "INVALID_ARGUMENT");
    assert_eq!(code, 2);

    let items = db.ok_items(&["task", "list"]);
    assert!(items.is_empty(), "failed command must not write data");
}

/// CLI-005：成功命令的 stdout 是单个合法 JSON 响应。
#[test]
fn cli005_success_outputs_single_json_object() {
    let db = TestDb::new();
    let output = db.raw_json(&["task", "create", "--title", "json task"]);
    common::assert_success(&output, &["task", "create"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        stdout.trim().lines().count(),
        1,
        "stdout must be one JSON line"
    );

    let value = parse_json(&output);
    assert_eq!(value["success"], Value::Bool(true));
    assert!(value["data"]["id"].is_string());
    assert_eq!(value["data"]["type"], "task");
    assert!(value["error"].is_null());
}

/// CLI-006：失败命令输出合法错误 JSON，且退出码非零。
#[test]
fn cli006_failure_outputs_json_and_nonzero_exit() {
    let db = TestDb::new();
    let output = db.raw_json(&["task", "get", "--id", "missing-id"]);
    assert!(!output.status.success());

    let value = parse_json(&output);
    assert_eq!(value["success"], Value::Bool(false));
    assert!(value["data"].is_null());
    assert_error_code(&value, "TASK_NOT_FOUND");
}

/// CLI-007：JSON 模式 stdout 不混入日志。
#[test]
fn cli007_stdout_is_clean_in_json_mode() {
    let db = TestDb::new();
    let ok = db.raw_json(&["task", "create", "--title", "clean stdout"]);
    let fail = db.raw_json(&["task", "get", "--id", "nope"]);

    for output in [&ok, &fail] {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let trimmed = stdout.trim();
        assert!(trimmed.starts_with('{') && trimmed.ends_with('}'));
        assert_eq!(trimmed.lines().count(), 1);
        serde_json::from_str::<Value>(trimmed).expect("stdout must be pure JSON");
    }
}

/// CLI-008：通过 stdin JSON 创建数据；也支持 JSON 文件路径。
#[test]
fn cli008_stdin_json_input() {
    let db = TestDb::new();
    let output = db.raw_json_with_stdin(
        &["task", "create", "--input-json", "-"],
        r#"{"title":"来自 stdin 的待办","priority":3,"due_at":"2026-11-17T23:59:00+08:00"}"#,
    );
    common::assert_success(&output, &["task", "create", "--input-json", "-"]);
    let value = parse_json(&output);
    assert_eq!(value["data"]["title"], "来自 stdin 的待办");
    assert_eq!(value["data"]["priority"], 3);
    assert_eq!(value["data"]["due_at"], "2026-11-17T23:59:00+08:00");

    // JSON 文件路径输入
    let dir = TempDir::new().unwrap();
    let input_path = dir.path().join("input.json");
    std::fs::write(&input_path, r#"{"title":"来自文件的待办"}"#).unwrap();
    let data = db.ok_json(&[
        "task",
        "create",
        "--input-json",
        input_path.to_str().unwrap(),
    ]);
    assert_eq!(data["title"], "来自文件的待办");
}

/// CLI-009：stdin 非法 JSON 返回明确输入错误，且不写入数据。
#[test]
fn cli009_invalid_json_input_is_rejected() {
    let db = TestDb::new();
    for bad in [r#"{"title": "#, r#"not json at all"#] {
        let output = db.raw_json_with_stdin(&["task", "create", "--input-json", "-"], bad);
        assert!(!output.status.success());
        let value = parse_json(&output);
        assert_error_code(&value, "INVALID_INPUT");
    }
    let items = db.ok_items(&["task", "list"]);
    assert!(items.is_empty(), "invalid input must not write data");
}

/// CLI-010 / REL-006：`--db` 指定数据库路径，两个数据库互不影响；环境变量也可配置路径。
#[test]
fn cli010_database_path_configuration() {
    let db1 = TestDb::new();
    let db2 = TestDb::new();
    db1.ok_json(&["task", "create", "--title", "only in db1"]);
    let items = db2.ok_items(&["task", "list"]);
    assert!(items.is_empty(), "databases must be isolated");

    // PAADM_DB_PATH 环境变量
    let dir = TempDir::new().unwrap();
    let env_db = dir.path().join("env.db");
    let output = bin()
        .args(["init", "--json"])
        .env("PAADM_DB_PATH", &env_db)
        .env_remove("PAADM_DATA_DIR")
        .output()
        .unwrap();
    common::assert_success(&output, &["init"]);
    assert!(env_db.exists(), "PAADM_DB_PATH database should be created");

    // PAADM_DATA_DIR 环境变量
    let dir2 = TempDir::new().unwrap();
    let output = bin()
        .args(["init", "--json"])
        .env("PAADM_DATA_DIR", dir2.path())
        .env_remove("PAADM_DB_PATH")
        .output()
        .unwrap();
    common::assert_success(&output, &["init"]);
    assert!(dir2.path().join("paa.db").exists());
}

// ---------------------------------------------------------------------------
// 用户与数据隔离（USER-001..004）
// ---------------------------------------------------------------------------

/// USER-001：创建用户返回稳定 ID，provider 统一小写。
#[test]
fn user001_create_user() {
    let db = TestDb::new();
    let data = db.ok_json(&[
        "user",
        "create",
        "--provider",
        "QQ",
        "--external-user-id",
        "external-1",
    ]);
    assert_eq!(data["provider"], "qq");
    assert_eq!(data["external_user_id"], "external-1");
    let user_id = id_of(&data);

    let fetched = db.ok_json(&["user", "get", "--id", &user_id]);
    assert_eq!(fetched["id"], user_id);
}

/// USER-002：同一（provider, external_user_id）重复创建报 DUPLICATE_USER；
/// resolve 是幂等的“查找或创建”。
#[test]
fn user002_duplicate_create_and_resolve() {
    let db = TestDb::new();
    let first = db.ok_json(&[
        "user",
        "create",
        "--provider",
        "qq",
        "--external-user-id",
        "dup-1",
    ]);
    let first_id = id_of(&first);

    let output = db.raw_json_without_user(&[
        "user",
        "create",
        "--provider",
        "qq",
        "--external-user-id",
        "dup-1",
    ]);
    assert!(!output.status.success());
    let error = parse_json(&output);
    assert_error_code(&error, "DUPLICATE_USER");

    // resolve：已存在时 created=false 且返回同一个用户。
    let resolved = db.ok_json(&[
        "user",
        "resolve",
        "--provider",
        "qq",
        "--external-user-id",
        "dup-1",
    ]);
    assert_eq!(resolved["id"], first_id);
    assert_eq!(resolved["created"], false);

    // 新身份 resolve 时创建（created=true）。
    let created = db.ok_json(&[
        "user",
        "resolve",
        "--provider",
        "wechat",
        "--external-user-id",
        "new-1",
    ]);
    assert_eq!(created["created"], true);
    assert!(created["id"].is_string());
}

/// USER-003：不存在的用户 / 缺少 --user-id 的业务命令返回明确错误。
#[test]
fn user003_unknown_or_missing_user() {
    let db = TestDb::new();

    let output = db.raw_json_without_user(&["task", "list"]);
    assert!(!output.status.success());
    let error = parse_json(&output);
    assert_error_code(&error, "INVALID_ARGUMENT");

    let output = db.raw_json_as("11111111-1111-1111-1111-111111111111", &["task", "list"]);
    assert!(!output.status.success());
    let error = parse_json(&output);
    assert_error_code(&error, "USER_NOT_FOUND");
}

/// USER-004：所有业务数据按 user_id 隔离，跨用户访问表现为“不存在”。
#[test]
fn user004_cross_user_data_isolation() {
    let db = TestDb::new();
    let other_user = db.add_user("qq", "other-user");

    // user1（默认测试用户）创建各领域数据。
    let task = db.ok_json(&["task", "create", "--title", "user1 task"]);
    let task_id = id_of(&task);
    let schedule = db.ok_json(&[
        "schedule",
        "create",
        "--title",
        "user1 event",
        "--start-at",
        "2026-11-20T10:00:00+08:00",
    ]);
    let schedule_id = id_of(&schedule);
    let reminder = db.ok_json(&[
        "reminder",
        "create",
        "--content",
        "user1 remind",
        "--remind-at",
        "2026-11-18T09:00:00+08:00",
    ]);
    let reminder_id = id_of(&reminder);
    let transaction = db.ok_json(&[
        "transaction",
        "create",
        "--type",
        "expense",
        "--amount",
        "1850",
        "--occurred-at",
        "2026-11-17T12:00:00+08:00",
    ]);
    let transaction_id = id_of(&transaction);
    let note = db.ok_json(&["note", "create", "--content", "user1 note"]);
    let note_id = id_of(&note);

    // user2 看不到 user1 的任何数据。
    for args in [
        vec!["task", "list"],
        vec!["schedule", "list"],
        vec!["reminder", "list"],
        vec!["transaction", "list"],
        vec!["note", "list"],
    ] {
        let data = db.ok_json_as(&other_user, &args);
        assert_eq!(data["count"], 0, "leaked data for args={args:?}");
    }
    assert_error_code(
        &db.fail_json_as(&other_user, &["task", "get", "--id", &task_id])
            .0,
        "TASK_NOT_FOUND",
    );
    assert_error_code(
        &db.fail_json_as(&other_user, &["schedule", "get", "--id", &schedule_id])
            .0,
        "SCHEDULE_NOT_FOUND",
    );
    assert_error_code(
        &db.fail_json_as(&other_user, &["reminder", "get", "--id", &reminder_id])
            .0,
        "REMINDER_NOT_FOUND",
    );
    assert_error_code(
        &db.fail_json_as(
            &other_user,
            &["transaction", "get", "--id", &transaction_id],
        )
        .0,
        "TRANSACTION_NOT_FOUND",
    );
    assert_error_code(
        &db.fail_json_as(&other_user, &["note", "get", "--id", &note_id])
            .0,
        "NOTE_NOT_FOUND",
    );

    // 跨用户的修改/删除也必须失败，且不能影响原数据。
    assert_error_code(
        &db.fail_json_as(
            &other_user,
            &["task", "update", "--id", &task_id, "--title", "hacked"],
        )
        .0,
        "TASK_NOT_FOUND",
    );
    assert_error_code(
        &db.fail_json_as(&other_user, &["note", "delete", "--id", &note_id])
            .0,
        "NOTE_NOT_FOUND",
    );
    let fetched = db.ok_json(&["task", "get", "--id", &task_id]);
    assert_eq!(fetched["title"], "user1 task");
    let fetched = db.ok_json(&["note", "get", "--id", &note_id]);
    assert_eq!(fetched["content"], "user1 note");
}

// ---------------------------------------------------------------------------
// items 统一表语义（task / event 区分）
// ---------------------------------------------------------------------------

/// ITEM-001：task 与 event 共用 items 表但命令组互不串数据。
#[test]
fn item001_task_and_event_are_separated_by_type() {
    let db = TestDb::new();
    let task = db.ok_json(&["task", "create", "--title", "a task"]);
    let task_id = id_of(&task);
    let event = db.ok_json(&[
        "schedule",
        "create",
        "--title",
        "an event",
        "--start-at",
        "2026-11-20T10:00:00+08:00",
    ]);
    let event_id = id_of(&event);

    assert_eq!(task["type"], "task");
    assert_eq!(event["type"], "event");
    assert_eq!(task["all_day"], false);

    let tasks = db.ok_items(&["task", "list"]);
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0]["type"], "task");
    let events = db.ok_items(&["schedule", "list"]);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["type"], "event");

    // 用错命令组访问另一类型的记录 → 不存在。
    assert_error_code(
        &db.fail_json(&["task", "get", "--id", &event_id]).0,
        "TASK_NOT_FOUND",
    );
    assert_error_code(
        &db.fail_json(&["schedule", "get", "--id", &task_id]).0,
        "SCHEDULE_NOT_FOUND",
    );
    // 用错命令组删除也不能影响另一类型。
    assert_error_code(
        &db.fail_json(&["task", "delete", "--id", &event_id]).0,
        "TASK_NOT_FOUND",
    );
    assert!(
        db.ok_json(&["schedule", "get", "--id", &event_id])
            .get("id")
            .is_some()
    );
}

// ---------------------------------------------------------------------------
// 数据库行为
// ---------------------------------------------------------------------------

/// DB-001：首次运行初始化 SQLite（新 schema v2）。
#[test]
fn db001_first_run_initializes_database() {
    let db = TestDb::new();
    let data = db.ok_json(&["init"]);
    assert!(db.path.exists());
    assert_eq!(data["schema_version"], 2);
    assert!(data["db_path"].as_str().unwrap().contains("paa.db"));

    // 任意业务命令也会自动初始化数据库。
    let db2 = TestDb::new();
    db2.ok_json(&["task", "create", "--title", "auto init"]);
    assert!(db2.path.exists());
}

/// DB-002：重复初始化不破坏已有数据。
#[test]
fn db002_repeated_init_keeps_data() {
    let db = TestDb::new();
    db.ok_json(&["init"]);
    db.ok_json(&["task", "create", "--title", "keep me"]);
    db.ok_json(&["init"]);
    let items = db.ok_items(&["task", "list"]);
    assert_eq!(items.len(), 1);
}

/// DB-003：进程退出后重新启动，数据仍然存在。
#[test]
fn db003_data_persists_across_processes() {
    let db = TestDb::new();
    let created = db.ok_json(&["task", "create", "--title", "persistent"]);
    let id = id_of(&created);

    // 每次 ok_json 都是一个新的 paadm 进程，等价于退出后重启查询。
    let fetched = db.ok_json(&["task", "get", "--id", &id]);
    assert_eq!(fetched["title"], "persistent");
}

/// DB-004：数据库路径不可写时返回明确错误，不报告成功。
#[test]
fn db004_unwritable_database_path() {
    let dir = TempDir::new().unwrap();
    let blocker = dir.path().join("not-a-dir");
    std::fs::write(&blocker, b"file").unwrap();
    let path = blocker.join("paa.db");

    let output = bin()
        .arg("--db")
        .arg(&path)
        .args(["--json", "init"])
        .env_remove("PAADM_DB_PATH")
        .env_remove("PAADM_DATA_DIR")
        .output()
        .unwrap();
    assert!(!output.status.success());
    let value = parse_json(&output);
    assert_error_code(&value, "DATABASE_ERROR");
}

/// DB-006：含引号、分号等特殊字符的文本作为普通数据保存，不改变 SQL 语义。
#[test]
fn db006_sql_injection_text_is_stored_as_data() {
    let db = TestDb::new();
    let evil = "he said \"hi\"; DROP TABLE items; --";
    let created = db.ok_json(&["task", "create", "--title", evil]);
    let id = id_of(&created);
    let fetched = db.ok_json(&["task", "get", "--id", &id]);
    assert_eq!(fetched["title"], evil);

    // items 表仍然可用。
    db.ok_json(&["task", "create", "--title", "still works"]);
    assert_eq!(db.ok_items(&["task", "list"]).len(), 2);
}

// ---------------------------------------------------------------------------
// 系统并发与健壮性
// ---------------------------------------------------------------------------

/// SYS-001：多个进程并发读取正常。
#[test]
fn sys001_concurrent_reads() {
    let db = TestDb::new();
    db.ok_json(&["task", "create", "--title", "concurrent read"]);
    let path = db.path.clone();
    let user_id = db.user_id.clone();

    let handles: Vec<_> = (0..5)
        .map(|_| {
            let path = path.clone();
            let user_id = user_id.clone();
            thread::spawn(move || {
                let child = spawn_json(&path, &user_id, &["task", "list"]);
                let data = child_data(child);
                data["count"].as_i64().unwrap()
            })
        })
        .collect();

    for handle in handles {
        assert_eq!(handle.join().unwrap(), 1);
    }
}

/// SYS-002：多个进程并发创建，不丢失成功写入的数据。
#[test]
fn sys002_concurrent_writes() {
    let db = TestDb::new();
    db.ok_json(&["init"]);
    let path = db.path.clone();
    let user_id = db.user_id.clone();

    let handles: Vec<_> = (0..8)
        .map(|i| {
            let path = path.clone();
            let user_id = user_id.clone();
            thread::spawn(move || {
                let title = format!("concurrent-{i}");
                let child = spawn_json(&path, &user_id, &["task", "create", "--title", &title]);
                let data = child_data(child);
                id_of(&data)
            })
        })
        .collect();

    let mut ids = Vec::new();
    for handle in handles {
        ids.push(handle.join().unwrap());
    }
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), 8, "all created ids must be distinct");

    let items = db.ok_items(&["task", "list"]);
    assert_eq!(items.len(), 8, "no successful write may be lost");
}

/// SYS-003：写锁被占用时，另一个进程在 busy timeout 内等待后可成功写入。
#[test]
fn sys003_write_lock_waiting() {
    let db = TestDb::new();
    db.ok_json(&["init"]);

    let mut holder = paadm::storage::Database::open_path(&db.path).unwrap();
    holder
        .connection_mut()
        .execute_batch("BEGIN IMMEDIATE")
        .unwrap();

    // 持有写事务期间，子进程会在 busy_timeout 内等待。
    let child = spawn_json(
        &db.path,
        &db.user_id,
        &["task", "create", "--title", "waited"],
    );
    thread::sleep(Duration::from_millis(300));
    holder.connection_mut().execute_batch("COMMIT").unwrap();
    drop(holder);

    let data = child_data(child);
    assert_eq!(data["title"], "waited");
}

/// SYS-004：进程被强制中断后，数据库仍可恢复并继续使用。
#[test]
fn sys004_abrupt_interruption_keeps_database_usable() {
    let db = TestDb::new();
    db.ok_json(&["init"]);

    let mut child = spawn_json(
        &db.path,
        &db.user_id,
        &["task", "create", "--title", "killed"],
    );
    let _ = child.kill();
    let _ = child.wait();

    // 下次启动应正常完成初始化/恢复，且数据库可继续写入。
    db.ok_json(&["init"]);
    let data = db.ok_json(&["task", "create", "--title", "after kill"]);
    assert!(data["id"].is_string());
}

/// SYS-007：空数据库上所有 list 命令返回空结果，不崩溃。
#[test]
fn sys007_empty_database_lists_are_empty_success() {
    let db = TestDb::new();
    for args in [
        vec!["task", "list"],
        vec!["schedule", "list"],
        vec!["reminder", "list"],
        vec!["transaction", "list"],
        vec!["note", "list"],
    ] {
        let data = db.ok_json(&args);
        assert_eq!(data["count"], 0, "args={args:?}");
        assert_eq!(data["items"].as_array().unwrap().len(), 0, "args={args:?}");
    }
}

/// SYS-008：损坏数据库文件返回明确错误，不静默覆盖原文件。
#[test]
fn sys008_corrupt_database_file() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("corrupt.db");
    let garbage = b"garbage, not sqlite";
    std::fs::write(&path, garbage).unwrap();

    let output = bin()
        .arg("--db")
        .arg(&path)
        .args(["--json", "task", "list"])
        .env_remove("PAADM_DB_PATH")
        .env_remove("PAADM_DATA_DIR")
        .output()
        .unwrap();
    assert!(!output.status.success());
    let value = parse_json(&output);
    let code = value["error"]["code"].as_str().unwrap();
    assert!(
        code == "DATABASE_ERROR" || code == "MIGRATION_FAILED",
        "unexpected code: {code}"
    );
    assert_eq!(std::fs::read(&path).unwrap(), garbage);
}

// ---------------------------------------------------------------------------
// Agent 调用契约
// ---------------------------------------------------------------------------

/// AGENT-001/002：Agent 以 JSON 模式创建并查询，返回机器可读结果。
#[test]
fn agent001_agent002_create_and_list_contract() {
    let db = TestDb::new();
    let created = db.ok_json(&["task", "create", "--title", "agent task"]);
    assert!(created["id"].is_string());

    let data = db.ok_json(&["task", "list"]);
    let items = data["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["title"], "agent task");
    assert_eq!(items[0]["status"], "pending");
    assert!(items[0]["id"].is_string());
}

/// AGENT-003/007：业务错误返回稳定错误码，退出码与结果一致。
#[test]
fn agent003_business_error_and_exit_code() {
    let db = TestDb::new();
    let (error, code) = db.fail_json(&["task", "get", "--id", "does-not-exist"]);
    assert_error_code(&error, "TASK_NOT_FOUND");
    assert_eq!(code, 1);

    let ok = db.raw_json(&["task", "create", "--title", "ok"]);
    assert_eq!(ok.status.code(), Some(0));
}

/// AGENT-004：非法字段返回参数/校验错误。
#[test]
fn agent004_invalid_field_returns_validation_error() {
    let db = TestDb::new();
    let (error, code) = db.fail_json(&[
        "task",
        "create",
        "--title",
        "bad priority",
        "--priority",
        "urgent",
    ]);
    assert_error_code(&error, "INVALID_ARGUMENT");
    assert_eq!(code, 2);
}

/// AGENT-005/006：stdout 只有一个 JSON，stderr 日志不影响解析。
#[test]
fn agent005_agent006_stdout_purity_with_stderr_logs() {
    let db = TestDb::new();
    let output = db.raw_json(&["task", "create", "--title", "stderr check"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(stdout.trim().lines().count(), 1);
    let _ = String::from_utf8_lossy(&output.stderr); // stderr 内容不参与解析
    serde_json::from_str::<Value>(stdout.trim()).expect("stdout JSON");
}

/// AGENT-008：特殊字符参数安全传递，内容不丢失。
#[test]
fn agent008_special_characters_in_arguments() {
    let db = TestDb::new();
    let title = "中文 \"quoted\" ; -- $VAR 'single' 换行\n第二行";
    let created = db.ok_json(&["task", "create", "--title", title]);
    let id = id_of(&created);
    let fetched = db.ok_json(&["task", "get", "--id", &id]);
    assert_eq!(fetched["title"], title);
}

/// AGENT-009：超长输入返回明确校验错误，不崩溃。
#[test]
fn agent009_overlong_input_is_rejected() {
    let db = TestDb::new();
    let long_title = "x".repeat(100_001);
    let (error, code) = db.fail_json(&["task", "create", "--title", &long_title]);
    assert_error_code(&error, "INVALID_ARGUMENT");
    assert_eq!(code, 2);
}

/// AGENT-010：单个命令内部部分失败时不写入半成品（全有或全无）。
#[test]
fn agent010_invalid_optional_field_writes_nothing() {
    let db = TestDb::new();
    let (error, _) = db.fail_json(&[
        "task",
        "create",
        "--title",
        "partial",
        "--due-at",
        "not-a-datetime",
    ]);
    assert_error_code(&error, "INVALID_ARGUMENT");
    assert!(db.ok_items(&["task", "list"]).is_empty());
}

// ---------------------------------------------------------------------------
// 跨领域独立性
// ---------------------------------------------------------------------------

/// REL-005：修改/删除一个领域的数据，不影响其他领域；删除事项时提醒保留并解除关联。
#[test]
fn rel005_domains_are_independent() {
    let db = TestDb::new();
    let task = db.ok_json(&["task", "create", "--title", "shared task"]);
    let task_id = id_of(&task);
    let schedule = db.ok_json(&[
        "schedule",
        "create",
        "--title",
        "meeting",
        "--start-at",
        "2026-11-20T10:00:00+08:00",
    ]);
    let schedule_id = id_of(&schedule);
    let reminder = db.ok_json(&[
        "reminder",
        "create",
        "--content",
        "linked remind",
        "--remind-at",
        "2026-11-18T09:00:00+08:00",
        "--item-id",
        &task_id,
    ]);
    let reminder_id = id_of(&reminder);
    let transaction = db.ok_json(&[
        "transaction",
        "create",
        "--type",
        "expense",
        "--amount",
        "1850",
        "--occurred-at",
        "2026-11-17T12:00:00+08:00",
    ]);
    let transaction_id = id_of(&transaction);
    let note = db.ok_json(&["note", "create", "--content", "an idea"]);
    let note_id = id_of(&note);

    // 修改并删除待办。
    db.ok_json(&["task", "update", "--id", &task_id, "--title", "renamed"]);
    db.ok_json(&["task", "delete", "--id", &task_id]);

    // 其他领域数据仍然存在且未被修改。
    let schedule = db.ok_json(&["schedule", "get", "--id", &schedule_id]);
    assert_eq!(schedule["title"], "meeting");
    let transaction = db.ok_json(&["transaction", "get", "--id", &transaction_id]);
    assert_eq!(transaction["amount"], 1850);
    let note = db.ok_json(&["note", "get", "--id", &note_id]);
    assert_eq!(note["content"], "an idea");

    // 关联提醒保留，item_id 被置为 NULL。
    let reminder = db.ok_json(&["reminder", "get", "--id", &reminder_id]);
    assert_eq!(reminder["item_id"], Value::Null);
}

/// REL-006：两个数据库路径的数据完全隔离。
#[test]
fn rel006_database_isolation() {
    let db_a = TestDb::new();
    let db_b = TestDb::new();
    db_a.ok_json(&["note", "create", "--content", "note A"]);
    assert!(db_b.ok_items(&["note", "list"]).is_empty());
    db_b.ok_json(&["note", "create", "--content", "note B"]);
    assert_eq!(db_a.ok_items(&["note", "list"]).len(), 1);
    assert_eq!(db_b.ok_items(&["note", "list"]).len(), 1);
}
