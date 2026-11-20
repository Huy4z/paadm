//! Task（待办）领域集成测试。
//!
//! 待办与日程在新表结构中统一存于 `items` 表（`type='task'` / `type='event'`），
//! 所有数据按 `user_id` 隔离。测试通过 `common::TestDb` 驱动真实 `paadm` 二进制，
//! 每个测试使用独立临时数据库（`TestDb` 会自动创建测试用户并给命令加 `--user-id`）。
//!
//! 覆盖 `docs/design/PAA_paadm_第一版测试用例.md` 第 3 节 TASK-001..TASK-028，
//! 以及 CLI-004/005/006/008/009 的 Task 侧契约、用户隔离与 items 类型隔离。

mod common;

use common::{TestDb, assert_error_code, id_of, parse_json};
use serde_json::{Value, json};

/// 固定的带时区截止时间（测试不得依赖当前时间）。
const DUE_A: &str = "2026-11-17T23:59:00+08:00";

// ---------------------------------------------------------------------------
// 测试辅助
// ---------------------------------------------------------------------------

/// 运行 `task create ...` 并断言成功，返回 data（items 行）。
fn create(db: &TestDb, args: &[&str]) -> Value {
    let mut command: Vec<&str> = vec!["task", "create"];
    command.extend_from_slice(args);
    db.ok_json(&command)
}

/// 运行 `task list ...` 并断言成功，返回 items。
fn list(db: &TestDb, args: &[&str]) -> Vec<Value> {
    let mut command: Vec<&str> = vec!["task", "list"];
    command.extend_from_slice(args);
    db.ok_items(&command)
}

/// 从列表项中取出 id 列表。
fn ids(items: &[Value]) -> Vec<String> {
    items.iter().map(id_of).collect()
}

/// 排序后的 id 列表，用于与顺序无关的集合比较。
fn sorted_ids(items: &[Value]) -> Vec<String> {
    let mut values = ids(items);
    values.sort();
    values
}

/// 把 `created_at`（UTC RFC 3339，可能带变长小数秒）归一化为固定宽度字符串，
/// 使其字典序与时间顺序一致，用于断言 task 列表排序规则。
fn timestamp_key(value: &str) -> String {
    let without_z = value.trim_end_matches('Z');
    let (base, fraction) = match without_z.split_once('.') {
        Some((base, fraction)) => (base, fraction),
        None => (without_z, ""),
    };
    format!("{base}.{fraction:0<9}")
}

/// 断言 `task list` 按 `created_at ASC, id ASC` 稳定排序。
fn assert_stable_task_order(items: &[Value]) {
    let actual: Vec<(String, String)> = items
        .iter()
        .map(|item| {
            let created_at = item["created_at"]
                .as_str()
                .unwrap_or_else(|| panic!("missing created_at in {item}"));
            (timestamp_key(created_at), id_of(item))
        })
        .collect();
    let mut expected = actual.clone();
    expected.sort();
    assert_eq!(
        actual, expected,
        "items are not sorted by (created_at, id): {items:?}"
    );
}

/// 断言创建的最小待办的 items 行形状（含新表结构字段）。
fn assert_minimal_task_shape(data: &Value, user_id: &str) {
    assert!(data["id"].is_string(), "id must be a string: {data}");
    assert_eq!(data["user_id"], user_id, "item must belong to current user");
    assert_eq!(data["type"], "task", "task command must create type=task");
    assert_eq!(data["status"], "pending", "new task defaults to pending");
    assert_eq!(data["priority"], Value::Null);
    assert_eq!(data["all_day"], false);
    assert!(data["start_at"].is_null());
    assert!(data["end_at"].is_null());
    assert!(data["due_at"].is_null());
    assert!(data["location"].is_null());
    assert!(data["description"].is_null());
    assert!(data["completed_at"].is_null());
    assert!(data["created_at"].is_string());
    assert!(data["updated_at"].is_string());
}

// ---------------------------------------------------------------------------
// 创建：TASK-001 .. TASK-008
// ---------------------------------------------------------------------------

/// TASK-001：创建最小待办（只提供标题）→ 成功，type=task、默认 status=pending。
#[test]
fn task_001_create_minimal_task() {
    let db = TestDb::new();
    let data = create(&db, &["--title", "买牛奶"]);

    assert_eq!(data["title"], "买牛奶");
    assert_minimal_task_shape(&data, &db.user_id);

    // 换一个新进程按 ID 查询，验证数据已持久化。
    let got = db.ok_json(&["task", "get", "--id", id_of(&data).as_str()]);
    assert_eq!(got, data);
}

/// TASK-002：创建完整待办（标题/描述/整数优先级/截止时间）→ 字段正确保存并返回。
#[test]
fn task_002_create_full_task() {
    let db = TestDb::new();
    let data = create(
        &db,
        &[
            "--title",
            "完成数学作业",
            "--description",
            "第 3 章全部习题",
            "--priority",
            "3",
            "--due-at",
            DUE_A,
        ],
    );

    assert_eq!(data["title"], "完成数学作业");
    assert_eq!(data["description"], "第 3 章全部习题");
    // priority 在新表结构中是 INTEGER，JSON 输出整数。
    assert_eq!(data["priority"], 3);
    assert_eq!(data["due_at"], DUE_A);
    assert_eq!(data["status"], "pending");
    assert_eq!(data["type"], "task");

    let got = db.ok_json(&["task", "get", "--id", id_of(&data).as_str()]);
    assert_eq!(got["description"], "第 3 章全部习题");
    assert_eq!(got["priority"], 3);
    assert_eq!(got["due_at"], DUE_A);
}

/// TASK-003：标题为空字符串 → INVALID_ARGUMENT，且不创建记录。
#[test]
fn task_003_create_rejects_empty_title() {
    let db = TestDb::new();
    let (error, exit_code) = db.fail_json(&["task", "create", "--title", ""]);
    assert_error_code(&error, "INVALID_ARGUMENT");
    assert_eq!(exit_code, 2);
    assert!(list(&db, &[]).is_empty());
}

/// TASK-004：标题只有空格 → INVALID_ARGUMENT，且不创建记录。
#[test]
fn task_004_create_rejects_whitespace_title() {
    let db = TestDb::new();
    let (error, exit_code) = db.fail_json(&["task", "create", "--title", "   "]);
    assert_error_code(&error, "INVALID_ARGUMENT");
    assert_eq!(exit_code, 2);
    assert!(list(&db, &[]).is_empty());
}

/// TASK-005（适配新语义）：priority 必须是整数；非整数 → 参数错误，不创建记录。
#[test]
fn task_005_create_rejects_non_integer_priority() {
    let db = TestDb::new();
    let (error, exit_code) = db.fail_json(&[
        "task",
        "create",
        "--title",
        "优先级非法",
        "--priority",
        "urgent",
    ]);
    assert_error_code(&error, "INVALID_ARGUMENT");
    assert_eq!(exit_code, 2);
    assert!(list(&db, &[]).is_empty());
}

/// TASK-006：非法截止时间（格式错误 / 缺少时区偏移 / 纯日期）→ INVALID_ARGUMENT。
#[test]
fn task_006_create_rejects_invalid_due_at() {
    let db = TestDb::new();

    for bad_due in ["not-a-time", "2026-11-17T15:00:00", "2026-11-17"] {
        let (error, _) =
            db.fail_json(&["task", "create", "--title", "时间非法", "--due-at", bad_due]);
        assert_error_code(&error, "INVALID_ARGUMENT");
    }

    assert!(list(&db, &[]).is_empty());
}

/// TASK-007：ID 唯一性 → 两条待办的 ID 不同，且都能按 ID 查到。
#[test]
fn task_007_task_ids_are_unique() {
    let db = TestDb::new();
    let first = create(&db, &["--title", "第一条"]);
    let second = create(&db, &["--title", "第二条"]);

    let first_id = id_of(&first);
    let second_id = id_of(&second);
    assert_ne!(first_id, second_id);

    assert_eq!(
        db.ok_json(&["task", "get", "--id", first_id.as_str()])["title"],
        "第一条"
    );
    assert_eq!(
        db.ok_json(&["task", "get", "--id", second_id.as_str()])["title"],
        "第二条"
    );
    assert_eq!(list(&db, &[]).len(), 2);
}

/// TASK-008（兼 DB-006）：标题含中文、引号、换行、分号等特殊字符 → 按原文保存并可查询。
#[test]
fn task_008_special_characters_title() {
    let db = TestDb::new();
    let title = "他说：\"你好\"；DROP TABLE items;--\n第二行 'quoted'";
    let description = "描述里有 ; -- ' \" \\ 和 % _ 通配符";

    let data = create(&db, &["--title", title, "--description", description]);
    assert_eq!(data["title"], title);
    assert_eq!(data["description"], description);

    let got = db.ok_json(&["task", "get", "--id", id_of(&data).as_str()]);
    assert_eq!(got["title"], title);
    assert_eq!(got["description"], description);

    // SQL 语义未被改变：表还在，后续写入仍然正常。
    let another = create(&db, &["--title", "普通标题"]);
    assert_eq!(another["title"], "普通标题");
    assert_eq!(list(&db, &[]).len(), 2);
}

// ---------------------------------------------------------------------------
// 查询：TASK-009 .. TASK-015
// ---------------------------------------------------------------------------

/// TASK-009：查询全部待办 → 返回所有 type=task 记录，且按 created_at ASC, id ASC 排序。
#[test]
fn task_009_list_all_tasks() {
    let db = TestDb::new();
    let first = create(&db, &["--title", "待办一"]);
    let second = create(&db, &["--title", "待办二"]);
    let third = create(&db, &["--title", "待办三"]);

    let items = list(&db, &[]);
    assert_eq!(items.len(), 3);
    assert!(items.iter().all(|item| item["type"] == "task"));

    let expected = sorted_ids(&[first.clone(), second.clone(), third]);
    assert_eq!(sorted_ids(&items), expected);
    assert_stable_task_order(&items);
}

/// TASK-010：按 ID 查询存在的待办 → 返回对应记录。
#[test]
fn task_010_get_task_by_id() {
    let db = TestDb::new();
    let created = create(
        &db,
        &[
            "--title",
            "按 ID 查询",
            "--description",
            "描述",
            "--priority",
            "2",
            "--due-at",
            DUE_A,
        ],
    );

    let got = db.ok_json(&["task", "get", "--id", id_of(&created).as_str()]);
    assert_eq!(got, created);
}

/// TASK-011：查询不存在的 ID → TASK_NOT_FOUND，退出码 1。
#[test]
fn task_011_get_missing_task_returns_not_found() {
    let db = TestDb::new();
    let (error, exit_code) = db.fail_json(&[
        "task",
        "get",
        "--id",
        "00000000-0000-4000-8000-000000000000",
    ]);
    assert_error_code(&error, "TASK_NOT_FOUND");
    assert_eq!(exit_code, 1);
}

/// TASK-012：按状态筛选 → 只返回匹配状态的待办；非法状态 → INVALID_ARGUMENT。
#[test]
fn task_012_list_filter_by_status() {
    let db = TestDb::new();
    let pending_a = create(&db, &["--title", "待处理甲"]);
    let pending_b = create(&db, &["--title", "待处理乙"]);
    let to_complete = create(&db, &["--title", "将完成"]);
    let to_cancel = create(&db, &["--title", "将取消"]);

    let completed = db.ok_json(&["task", "complete", "--id", id_of(&to_complete).as_str()]);
    assert_eq!(completed["status"], "completed");
    let cancelled = db.ok_json(&["task", "cancel", "--id", id_of(&to_cancel).as_str()]);
    assert_eq!(cancelled["status"], "cancelled");

    let pending_items = list(&db, &["--status", "pending"]);
    let expected_pending = sorted_ids(&[pending_a, pending_b]);
    assert_eq!(sorted_ids(&pending_items), expected_pending);
    assert!(pending_items.iter().all(|item| item["status"] == "pending"));

    let completed_items = list(&db, &["--status", "completed"]);
    assert_eq!(ids(&completed_items), vec![id_of(&to_complete)]);
    assert_eq!(completed_items[0]["status"], "completed");

    let cancelled_items = list(&db, &["--status", "cancelled"]);
    assert_eq!(ids(&cancelled_items), vec![id_of(&to_cancel)]);
    assert_eq!(cancelled_items[0]["status"], "cancelled");

    let (error, _) = db.fail_json(&["task", "list", "--status", "done"]);
    assert_error_code(&error, "INVALID_ARGUMENT");
}

/// TASK-013：按截止时间范围筛选（含边界、跨时区按时间点比较，无 due_at 不返回）。
#[test]
fn task_013_list_filter_by_due_range() {
    let db = TestDb::new();
    let morning = create(
        &db,
        &[
            "--title",
            "17 日早晨",
            "--due-at",
            "2026-11-17T09:00:00+08:00",
        ],
    );
    let evening = create(
        &db,
        &[
            "--title",
            "17 日晚上",
            "--due-at",
            "2026-11-17T20:00:00+08:00",
        ],
    );
    let next_day = create(
        &db,
        &["--title", "18 日", "--due-at", "2026-11-18T09:00:00+08:00"],
    );
    // 同一时刻用 UTC 表示（02:00Z = 10:00+08:00），应落在 17 日范围内。
    let utc_equivalent = create(
        &db,
        &[
            "--title",
            "17 日 UTC 表示",
            "--due-at",
            "2026-11-17T02:00:00+00:00",
        ],
    );
    let without_due = create(&db, &["--title", "没有截止时间"]);

    let day_items = list(
        &db,
        &[
            "--due-from",
            "2026-11-17T00:00:00+08:00",
            "--due-to",
            "2026-11-17T23:59:59+08:00",
        ],
    );
    let expected_day = sorted_ids(&[morning.clone(), evening, utc_equivalent.clone()]);
    assert_eq!(sorted_ids(&day_items), expected_day);
    assert!(!ids(&day_items).contains(&id_of(&next_day)));
    assert!(!ids(&day_items).contains(&id_of(&without_due)));

    let after_from = list(&db, &["--due-from", "2026-11-18T00:00:00+08:00"]);
    assert_eq!(sorted_ids(&after_from), vec![id_of(&next_day)]);

    let before_to = list(&db, &["--due-to", "2026-11-17T12:00:00+08:00"]);
    let expected_before = sorted_ids(&[morning, utc_equivalent]);
    assert_eq!(sorted_ids(&before_to), expected_before);

    // 非法范围参数同样返回参数错误。
    let (error, _) = db.fail_json(&["task", "list", "--due-from", "2026-11-17"]);
    assert_error_code(&error, "INVALID_ARGUMENT");
}

/// TASK-014：空数据库查询列表 → 成功返回空数组，不视为错误。
#[test]
fn task_014_empty_list() {
    let db = TestDb::new();
    let data = db.ok_json(&["task", "list"]);
    assert_eq!(data["count"], 0);
    assert_eq!(data["items"], json!([]));
}

/// TASK-015：关键词查询 → 按标题包含匹配（ASCII 大小写不敏感），不匹配描述。
#[test]
fn task_015_list_filter_by_keyword() {
    let db = TestDb::new();
    create(&db, &["--title", "完成数学作业"]);
    create(&db, &["--title", "买牛奶"]);
    create(&db, &["--title", "数学复习"]);
    // 关键词只匹配标题：description 里出现“数学”不算匹配。
    create(&db, &["--title", "英语", "--description", "数学练习册"]);
    create(&db, &["--title", "Drink MILK"]);

    let math_items = list(&db, &["--keyword", "数学"]);
    assert_eq!(math_items.len(), 2);
    let titles: Vec<&str> = math_items
        .iter()
        .map(|item| item["title"].as_str().unwrap())
        .collect();
    assert!(titles.contains(&"完成数学作业"));
    assert!(titles.contains(&"数学复习"));

    assert_eq!(list(&db, &["--keyword", "牛奶"]).len(), 1);
    assert_eq!(list(&db, &["--keyword", "milk"]).len(), 1);
    assert!(list(&db, &["--keyword", "不存在的关键词"]).is_empty());
}

// ---------------------------------------------------------------------------
// 修改与状态变更：TASK-016 .. TASK-024
// ---------------------------------------------------------------------------

/// TASK-016：修改标题 → 标题更新，其他字段不变。
#[test]
fn task_016_update_title() {
    let db = TestDb::new();
    let created = create(
        &db,
        &[
            "--title",
            "旧标题",
            "--description",
            "描述",
            "--priority",
            "1",
            "--due-at",
            DUE_A,
        ],
    );

    let updated = db.ok_json(&[
        "task",
        "update",
        "--id",
        id_of(&created).as_str(),
        "--title",
        "新标题",
    ]);
    assert_eq!(updated["title"], "新标题");
    assert_eq!(updated["description"], "描述");
    assert_eq!(updated["priority"], 1);
    assert_eq!(updated["due_at"], DUE_A);
    assert_eq!(updated["status"], "pending");

    let got = db.ok_json(&["task", "get", "--id", id_of(&created).as_str()]);
    assert_eq!(got["title"], "新标题");
}

/// TASK-017：修改截止时间 → 新时间正确保存（保存原始串），并参与范围查询。
#[test]
fn task_017_update_due_at() {
    let db = TestDb::new();
    let created = create(&db, &["--title", "改截止时间", "--due-at", DUE_A]);
    let new_due = "2026-12-01T10:00:00+08:00";

    let updated = db.ok_json(&[
        "task",
        "update",
        "--id",
        id_of(&created).as_str(),
        "--due-at",
        new_due,
    ]);
    assert_eq!(updated["due_at"], new_due);

    let got = db.ok_json(&["task", "get", "--id", id_of(&created).as_str()]);
    assert_eq!(got["due_at"], new_due);

    let items = list(&db, &["--due-from", "2026-12-01T00:00:00+08:00"]);
    assert_eq!(ids(&items), vec![id_of(&created)]);
}

/// TASK-018：显式清除可选字段 → description / priority / due_at 变为 null。
#[test]
fn task_018_clear_optional_fields() {
    let db = TestDb::new();
    let created = create(
        &db,
        &[
            "--title",
            "清除可选字段",
            "--description",
            "描述",
            "--priority",
            "3",
            "--due-at",
            DUE_A,
        ],
    );

    let cleared = db.ok_json(&[
        "task",
        "update",
        "--id",
        id_of(&created).as_str(),
        "--clear-description",
        "--clear-priority",
        "--clear-due-at",
    ]);
    assert!(cleared["description"].is_null());
    assert!(cleared["priority"].is_null());
    assert!(cleared["due_at"].is_null());
    assert_eq!(cleared["title"], "清除可选字段");

    let got = db.ok_json(&["task", "get", "--id", id_of(&created).as_str()]);
    assert!(got["description"].is_null());
    assert!(got["priority"].is_null());
    assert!(got["due_at"].is_null());
}

/// TASK-019：修改不存在的待办 → TASK_NOT_FOUND。
#[test]
fn task_019_update_missing_task_returns_not_found() {
    let db = TestDb::new();
    let (error, exit_code) = db.fail_json(&[
        "task",
        "update",
        "--id",
        "00000000-0000-4000-8000-000000000000",
        "--title",
        "新标题",
    ]);
    assert_error_code(&error, "TASK_NOT_FOUND");
    assert_eq!(exit_code, 1);
}

/// TASK-020：完成待办 → 状态变为 completed，completed_at 被设置。
#[test]
fn task_020_complete_task() {
    let db = TestDb::new();
    let created = create(&db, &["--title", "要完成的事"]);
    assert_eq!(created["status"], "pending");
    assert!(created["completed_at"].is_null());

    let completed = db.ok_json(&["task", "complete", "--id", id_of(&created).as_str()]);
    assert_eq!(completed["status"], "completed");
    assert!(completed["completed_at"].is_string());
    assert!(!completed["completed_at"].as_str().unwrap().is_empty());

    let got = db.ok_json(&["task", "get", "--id", id_of(&created).as_str()]);
    assert_eq!(got["status"], "completed");
    assert_eq!(got["completed_at"], completed["completed_at"]);
}

/// TASK-021：重复完成 → 幂等：成功、仍为 completed、completed_at 不被改写。
#[test]
fn task_021_complete_is_idempotent() {
    let db = TestDb::new();
    let created = create(&db, &["--title", "重复完成"]);

    let first = db.ok_json(&["task", "complete", "--id", id_of(&created).as_str()]);
    let second = db.ok_json(&["task", "complete", "--id", id_of(&created).as_str()]);

    assert_eq!(second["status"], "completed");
    assert_eq!(second["completed_at"], first["completed_at"]);
    assert_eq!(list(&db, &[]).len(), 1, "重复完成不得产生额外记录");
}

/// TASK-022：取消待办 → cancelled，且不会被误标记为 completed；取消会清空 completed_at。
#[test]
fn task_022_cancel_task() {
    let db = TestDb::new();
    let created = create(&db, &["--title", "不再需要"]);
    let cancelled = db.ok_json(&["task", "cancel", "--id", id_of(&created).as_str()]);
    assert_eq!(cancelled["status"], "cancelled");
    assert!(cancelled["completed_at"].is_null());

    let got = db.ok_json(&["task", "get", "--id", id_of(&created).as_str()]);
    assert_eq!(got["status"], "cancelled");
    assert!(got["completed_at"].is_null());

    // 已完成的任务再取消：取消语义优先，completed_at 被清除（固定规则）。
    let completed_task = create(&db, &["--title", "先完成再取消"]);
    let completed = db.ok_json(&["task", "complete", "--id", id_of(&completed_task).as_str()]);
    assert_eq!(completed["status"], "completed");
    let cancelled_after_completed =
        db.ok_json(&["task", "cancel", "--id", id_of(&completed_task).as_str()]);
    assert_eq!(cancelled_after_completed["status"], "cancelled");
    assert!(cancelled_after_completed["completed_at"].is_null());
}

/// TASK-023：成功修改后 updated_at 被刷新，created_at 不变。
#[test]
fn task_023_update_refreshes_updated_at() {
    let db = TestDb::new();
    let created = create(&db, &["--title", "刷新更新时间", "--description", "描述"]);

    let updated = db.ok_json(&[
        "task",
        "update",
        "--id",
        id_of(&created).as_str(),
        "--title",
        "刷新更新时间（已改）",
    ]);
    assert_ne!(
        updated["updated_at"], created["updated_at"],
        "successful update must refresh updated_at"
    );
    assert_eq!(updated["created_at"], created["created_at"]);
}

/// TASK-024：只修改一个字段时，其他字段保持不变（不会被意外覆盖）。
#[test]
fn task_024_update_only_provided_fields() {
    let db = TestDb::new();
    let created = create(
        &db,
        &[
            "--title",
            "只改标题",
            "--description",
            "原描述",
            "--priority",
            "2",
            "--due-at",
            DUE_A,
        ],
    );

    let updated = db.ok_json(&[
        "task",
        "update",
        "--id",
        id_of(&created).as_str(),
        "--title",
        "只改标题（新）",
    ]);
    assert_eq!(updated["title"], "只改标题（新）");
    assert_eq!(updated["description"], "原描述");
    assert_eq!(updated["priority"], 2);
    assert_eq!(updated["due_at"], DUE_A);
    assert_eq!(updated["status"], "pending");
    assert_eq!(updated["created_at"], created["created_at"]);
}

// ---------------------------------------------------------------------------
// 删除：TASK-025 .. TASK-028
// ---------------------------------------------------------------------------

/// TASK-025：删除存在的待办 → data = {id, deleted: true}，列表中不再出现。
#[test]
fn task_025_delete_existing_task() {
    let db = TestDb::new();
    let created = create(&db, &["--title", "要删除的待办"]);
    let id = id_of(&created);

    let data = db.ok_json(&["task", "delete", "--id", id.as_str()]);
    assert_eq!(data["id"], id);
    assert_eq!(data["deleted"], true);
    assert!(list(&db, &[]).is_empty());
}

/// TASK-026：删除不存在的 ID → TASK_NOT_FOUND。
#[test]
fn task_026_delete_missing_task_returns_not_found() {
    let db = TestDb::new();
    let (error, exit_code) = db.fail_json(&["task", "delete", "--id", "not-exist-id"]);
    assert_error_code(&error, "TASK_NOT_FOUND");
    assert_eq!(exit_code, 1);
}

/// TASK-027：删除后按 ID 查询 → TASK_NOT_FOUND。
#[test]
fn task_027_get_after_delete_returns_not_found() {
    let db = TestDb::new();
    let created = create(&db, &["--title", "删除后查询"]);
    let id = id_of(&created);
    db.ok_json(&["task", "delete", "--id", id.as_str()]);

    let (error, _) = db.fail_json(&["task", "get", "--id", id.as_str()]);
    assert_error_code(&error, "TASK_NOT_FOUND");
}

/// TASK-028：重复删除同一 ID → 明确返回 TASK_NOT_FOUND，不崩溃。
#[test]
fn task_028_delete_twice_returns_not_found() {
    let db = TestDb::new();
    let created = create(&db, &["--title", "重复删除"]);
    let id = id_of(&created);

    db.ok_json(&["task", "delete", "--id", id.as_str()]);
    let (error, _) = db.fail_json(&["task", "delete", "--id", id.as_str()]);
    assert_error_code(&error, "TASK_NOT_FOUND");
}

// ---------------------------------------------------------------------------
// 新表结构附加语义：priority 过滤、用户隔离、items 类型隔离、删除事务
// ---------------------------------------------------------------------------

/// 补充：按整数优先级筛选。
#[test]
fn task_029_list_filter_by_integer_priority() {
    let db = TestDb::new();
    let high = create(&db, &["--title", "高优先级", "--priority", "3"]);
    let low = create(&db, &["--title", "低优先级", "--priority", "1"]);

    let level3 = list(&db, &["--priority", "3"]);
    assert_eq!(ids(&level3), vec![id_of(&high)]);
    assert_eq!(level3[0]["priority"], 3);

    let level1 = list(&db, &["--priority", "1"]);
    assert_eq!(ids(&level1), vec![id_of(&low)]);

    assert!(list(&db, &["--priority", "99"]).is_empty());
}

/// 补充：用户隔离 —— 其他用户看不到、改不到、删不到当前用户的待办。
#[test]
fn task_030_user_isolation() {
    let db = TestDb::new();
    let task = create(&db, &["--title", "属于用户 A 的待办"]);
    let task_id = id_of(&task);
    let other_user = db.add_user("wechat", "task-user-b");

    // 其他用户列表为空（跨用户不可见）。
    assert!(db.ok_json_as(&other_user, &["task", "list"])["count"] == 0);

    // 其他用户按 ID 取/改/完成/删除都表现为“不存在”。
    for args in [
        vec!["task", "get", "--id", task_id.as_str()],
        vec!["task", "update", "--id", task_id.as_str(), "--title", "x"],
        vec!["task", "complete", "--id", task_id.as_str()],
        vec!["task", "cancel", "--id", task_id.as_str()],
        vec!["task", "delete", "--id", task_id.as_str()],
    ] {
        let (error, exit_code) = db.fail_json_as(&other_user, &args);
        assert_error_code(&error, "TASK_NOT_FOUND");
        assert_eq!(exit_code, 1, "args={args:?}");
    }

    // 原用户的数据仍然完好。
    let fetched = db.ok_json(&["task", "get", "--id", task_id.as_str()]);
    assert_eq!(fetched["title"], "属于用户 A 的待办");
    assert_eq!(list(&db, &[]).len(), 1);
}

/// 补充：items 类型隔离 —— task 命令组只操作 type=task，日程 ID 对 task 命令是“不存在”。
#[test]
fn task_031_task_commands_ignore_events() {
    let db = TestDb::new();
    let event = db.ok_json(&[
        "schedule",
        "create",
        "--title",
        "会议",
        "--start-at",
        "2026-11-20T10:00:00+08:00",
    ]);
    let event_id = id_of(&event);
    assert_eq!(event["type"], "event");

    // event 不出现在 task 列表中。
    assert!(list(&db, &[]).is_empty());

    for args in [
        vec!["task", "get", "--id", event_id.as_str()],
        vec!["task", "update", "--id", event_id.as_str(), "--title", "x"],
        vec!["task", "complete", "--id", event_id.as_str()],
        vec!["task", "delete", "--id", event_id.as_str()],
    ] {
        let (error, _) = db.fail_json(&args);
        assert_error_code(&error, "TASK_NOT_FOUND");
    }

    // event 本身未受影响。
    let fetched = db.ok_json(&["schedule", "get", "--id", event_id.as_str()]);
    assert_eq!(fetched["title"], "会议");
}

/// 补充：业务命令缺少 --user-id 时返回 INVALID_ARGUMENT（退出码 2）。
#[test]
fn task_032_missing_user_id_is_rejected() {
    let db = TestDb::new();
    let output = db.raw_json_without_user(&["task", "list"]);
    assert_eq!(output.status.code(), Some(2));
    let value = parse_json(&output);
    assert_eq!(value["success"], false);
    assert_error_code(&value, "INVALID_ARGUMENT");
}

/// 补充：删除待办必须在同一事务中先解除 reminders 关联，再删除 items。
///
/// 复合外键是 `ON DELETE RESTRICT`，若实现只删 items，本测试会因外键冲突失败；
/// 若实现先 UPDATE 但不在同一事务/未提交，则提醒关联状态不符合预期。
#[test]
fn task_033_delete_clears_linked_reminder_in_transaction() {
    let db = TestDb::new();
    let task = create(&db, &["--title", "有关联提醒的待办"]);
    let task_id = id_of(&task);

    // 直接向测试库插入关联提醒（reminder 命令组由其他开发者并行实现，
    // 本测试只验证 items 删除事务本身）。
    {
        let database = paadm::storage::Database::open_path(&db.path).expect("open test database");
        database
            .connection()
            .execute(
                "INSERT INTO reminders (id, user_id, item_id, content, remind_at, status, created_at, updated_at) \
                 VALUES ('rem-linked', ?1, ?2, '提醒内容', '2026-11-17T09:00:00+08:00', 'pending', \
                         '2026-11-17T00:00:00Z', '2026-11-17T00:00:00Z')",
                rusqlite::params![db.user_id, task_id],
            )
            .expect("insert linked reminder");
    } // 关闭连接，避免 WAL 写锁影响后续子进程

    let data = db.ok_json(&["task", "delete", "--id", task_id.as_str()]);
    assert_eq!(data["deleted"], true);
    assert!(list(&db, &[]).is_empty());

    // 提醒保留，但 item_id 被置为 NULL。
    let database = paadm::storage::Database::open_path(&db.path).expect("open test database");
    let (content, item_id): (String, Option<String>) = database
        .connection()
        .query_row(
            "SELECT content, item_id FROM reminders WHERE id = 'rem-linked'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("linked reminder should still exist");
    assert_eq!(content, "提醒内容");
    assert!(item_id.is_none(), "reminder.item_id must be cleared first");
}

// ---------------------------------------------------------------------------
// CLI JSON 契约：CLI-004 / CLI-005 / CLI-006 / CLI-008 / CLI-009
// ---------------------------------------------------------------------------

/// CLI-004（Task 侧）：缺少必需参数（--title）→ 参数错误且不写入数据。
#[test]
fn cli_004_create_without_title_is_rejected() {
    let db = TestDb::new();
    let (error, exit_code) = db.fail_json(&["task", "create"]);
    assert_error_code(&error, "INVALID_ARGUMENT");
    assert_eq!(exit_code, 2);
    assert!(list(&db, &[]).is_empty());
}

/// CLI-005：成功命令的 stdout 是单个合法 JSON 响应（success=true, error=null）。
#[test]
fn cli_005_success_json_shape() {
    let db = TestDb::new();
    let output = db.raw_json(&["task", "create", "--title", "JSON 契约"]);
    assert!(output.status.success(), "expected exit code 0");
    let value = parse_json(&output);
    assert_eq!(value["success"], true);
    assert_eq!(value["error"], Value::Null);
    assert!(value["data"].is_object());
    assert!(!value["data"]["id"].as_str().unwrap_or("").is_empty());
    assert_eq!(value["data"]["type"], "task");
}

/// CLI-006：失败命令的 stdout 也是合法错误 JSON，且退出码非零。
#[test]
fn cli_006_failure_json_shape() {
    let db = TestDb::new();
    let output = db.raw_json(&["task", "get", "--id", "missing-id"]);
    assert_eq!(output.status.code(), Some(1));
    let value = parse_json(&output);
    assert_eq!(value["success"], false);
    assert_eq!(value["data"], Value::Null);
    assert_eq!(value["error"]["code"], "TASK_NOT_FOUND");
}

/// CLI-008：通过 stdin 的 `--input-json -` 创建/修改待办（priority 为整数）。
#[test]
fn cli_008_create_from_stdin_json() {
    let db = TestDb::new();
    let output = db.raw_json_with_stdin(
        &["task", "create", "--input-json", "-"],
        r#"{"title": "从 JSON 创建", "description": "来自 stdin", "priority": 2, "due_at": "2026-11-17T15:00:00+08:00"}"#,
    );
    assert!(
        output.status.success(),
        "expected success, stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value = parse_json(&output);
    assert_eq!(value["success"], true);
    assert_eq!(value["data"]["title"], "从 JSON 创建");
    assert_eq!(value["data"]["description"], "来自 stdin");
    assert_eq!(value["data"]["priority"], 2);
    assert_eq!(value["data"]["due_at"], "2026-11-17T15:00:00+08:00");

    // --input-json 同样适用于 update。
    let id = id_of(&value["data"]);
    let update_output = db.raw_json_with_stdin(
        &["task", "update", "--input-json", "-"],
        &format!(r#"{{"id": "{id}", "title": "从 JSON 更新"}}"#),
    );
    assert!(update_output.status.success());
    let updated = parse_json(&update_output);
    assert_eq!(updated["data"]["title"], "从 JSON 更新");
}

/// CLI-009：非法/截断的 JSON 输入 → INVALID_INPUT，且不写入数据。
#[test]
fn cli_009_invalid_stdin_json_is_rejected() {
    let db = TestDb::new();
    let output = db.raw_json_with_stdin(&["task", "create", "--input-json", "-"], r#"{"title": "#);
    assert_eq!(output.status.code(), Some(2));
    let value = parse_json(&output);
    assert_eq!(value["success"], false);
    assert_eq!(value["error"]["code"], "INVALID_INPUT");
    assert!(list(&db, &[]).is_empty());
}
