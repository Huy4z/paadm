//! Reminder（提醒记录）领域集成测试 —— 按权威表结构（docs/design/paadm数据表结构设计.md）。
//!
//! reminders 字段：id, user_id, item_id(可空), content, remind_at,
//! status(pending|triggered|cancelled), created_at, updated_at。
//! 旧字段 repeat_rule 已删除；旧状态 active 改为 pending，并提供 triggered。
//!
//! 固定契约（本文件即约定）：
//! - 单条返回 `data = {记录}`；列表 `data = {items, count}`；删除 `{id, deleted:true}`；
//! - `status` 默认 pending；`trigger` 只允许 pending → triggered（triggered 幂等，
//!   cancelled 触发按参数错误 INVALID_ARGUMENT 处理）；
//! - `cancel` 只改状态、保留记录（重复取消幂等）；
//! - 列表按 `julianday(remind_at) ASC, id ASC` 排序，`--from/--to` 为闭区间；
//! - 所有操作按 user_id 隔离，跨用户访问表现为“不存在”；
//! - item_id 必须指向当前用户存在的 items.id，否则 ITEM_NOT_FOUND；
//! - 删除被关联事项时：同一事务先把 reminders.item_id 置 NULL，再删事项，
//!   因此提醒保留且 item_id 变为 null。

mod common;

use common::{TestDb, assert_error_code, id_of, parse_json};
use serde_json::Value;

const PENDING: &str = "pending";
const TRIGGERED: &str = "triggered";
const CANCELLED: &str = "cancelled";

/// 组装 `reminder create` 参数（默认必填项 + 额外参数）。
fn create_args<'a>(content: &'a str, remind_at: &'a str, extra: &[&'a str]) -> Vec<&'a str> {
    let mut args = vec![
        "reminder",
        "create",
        "--content",
        content,
        "--remind-at",
        remind_at,
    ];
    args.extend_from_slice(extra);
    args
}

/// 创建提醒并断言成功，返回记录对象。
fn create_reminder(db: &TestDb, content: &str, remind_at: &str) -> Value {
    db.ok_json(&create_args(content, remind_at, &[]))
}

/// 运行 `reminder list` 并返回 items。
fn list_reminders(db: &TestDb, extra: &[&str]) -> Vec<Value> {
    let mut args = vec!["reminder", "list"];
    args.extend_from_slice(extra);
    db.ok_items(&args)
}

/// 以指定用户运行 `reminder list` 并返回 items。
fn list_reminders_as(db: &TestDb, user_id: &str, extra: &[&str]) -> Vec<Value> {
    let mut args = vec!["reminder", "list"];
    args.extend_from_slice(extra);
    let data = db.ok_json_as(user_id, &args);
    assert_eq!(data["count"], data["items"].as_array().unwrap().len());
    data["items"].as_array().unwrap().clone()
}

/// 通过任务命令创建待办（items 表，type='task'）并返回 ID。
fn create_task(db: &TestDb, title: &str) -> String {
    let data = db.ok_json(&["task", "create", "--title", title]);
    id_of(&data)
}

fn ids(items: &[Value]) -> Vec<String> {
    items.iter().map(id_of).collect::<Vec<String>>()
}

// ---------------------------------------------------------------------------
// REM-001 创建一次性独立提醒：默认 pending，item_id 为空
// ---------------------------------------------------------------------------
#[test]
fn rem_001_create_one_time_reminder() {
    let db = TestDb::new();
    let data = create_reminder(&db, "交数学作业", "2026-11-17T15:00:00+08:00");

    let id = id_of(&data);
    assert!(!id.is_empty(), "id 不能为空");
    assert_eq!(data["user_id"], db.user_id, "记录必须归属当前用户");
    assert_eq!(data["content"], "交数学作业");
    // 保留用户输入的原始时区语义（不擅自改写为 UTC）。
    assert_eq!(data["remind_at"], "2026-11-17T15:00:00+08:00");
    assert_eq!(data["status"], PENDING, "新提醒默认 status=pending");
    assert!(data["item_id"].is_null(), "独立提醒 item_id 应为 null");
    assert!(data["created_at"].is_string(), "created_at 必须存在");
    assert!(data["updated_at"].is_string(), "updated_at 必须存在");
    // 不应再出现已废弃的 repeat_rule 字段。
    assert!(data.get("repeat_rule").is_none(), "repeat_rule 字段已删除");

    // 跨进程持久化：按 ID 查询返回同一条记录。
    let fetched = db.ok_json(&["reminder", "get", "--id", id.as_str()]);
    assert_eq!(fetched, data, "get 应返回与创建时一致的记录");
}

// ---------------------------------------------------------------------------
// REM-002 / REL-001 创建时关联存在的待办（items.id）：item_id 正确保存
// ---------------------------------------------------------------------------
#[test]
fn rem_002_rel_001_create_linked_reminder() {
    let db = TestDb::new();
    let task_id = create_task(&db, "写周报");

    let data = db.ok_json(&create_args(
        "提醒写周报",
        "2026-11-17T15:00:00+08:00",
        &["--item-id", task_id.as_str()],
    ));
    assert_eq!(data["item_id"], task_id, "item_id 应指向关联的待办");
    assert_eq!(data["status"], PENDING);

    let fetched = db.ok_json(&["reminder", "get", "--id", id_of(&data).as_str()]);
    assert_eq!(fetched["item_id"], task_id);
}

// ---------------------------------------------------------------------------
// REM-003 空提醒内容校验失败
// ---------------------------------------------------------------------------
#[test]
fn rem_003_empty_content_is_rejected() {
    let db = TestDb::new();
    for content in ["", "   ", "\t\n"] {
        let (error, exit_code) =
            db.fail_json(&create_args(content, "2026-11-17T15:00:00+08:00", &[]));
        assert_error_code(&error, "INVALID_ARGUMENT");
        assert_eq!(exit_code, 2, "参数校验错误退出码应为 2");
    }

    // 缺少 content 同样属于参数不合法。
    let (error, exit_code) = db.fail_json(&[
        "reminder",
        "create",
        "--remind-at",
        "2026-11-17T15:00:00+08:00",
    ]);
    assert_error_code(&error, "INVALID_ARGUMENT");
    assert_eq!(exit_code, 2);

    // 失败时不得留下任何记录。
    assert!(list_reminders(&db, &[]).is_empty());
}

// ---------------------------------------------------------------------------
// REM-004 非法提醒时间校验失败
// ---------------------------------------------------------------------------
#[test]
fn rem_004_invalid_remind_at_is_rejected() {
    let db = TestDb::new();
    let invalid_times = [
        "not-a-datetime",
        "2026-11-17",
        "2026-11-17T15:00:00",  // 缺少时区偏移，不能被悄悄当成 UTC
        "2026-13-17T15:00:00Z", // 非法月份
    ];
    for time in invalid_times {
        let (error, exit_code) = db.fail_json(&create_args("提醒", time, &[]));
        assert_error_code(&error, "INVALID_ARGUMENT");
        assert_eq!(exit_code, 2);
    }

    // 缺少 remind_at 必填项。
    let (error, exit_code) = db.fail_json(&["reminder", "create", "--content", "提醒"]);
    assert_error_code(&error, "INVALID_ARGUMENT");
    assert_eq!(exit_code, 2);

    assert!(list_reminders(&db, &[]).is_empty());
}

// ---------------------------------------------------------------------------
// REM-005 非法状态筛选值校验失败（新状态集合 pending/triggered/cancelled）
// ---------------------------------------------------------------------------
#[test]
fn rem_005_invalid_status_filter_is_rejected() {
    let db = TestDb::new();
    // 旧状态 active 现在是非法的；其他未知值同样非法。
    for status in ["active", "done", "completed", "PENDING", "triggerd"] {
        let (error, exit_code) = db.fail_json(&["reminder", "list", "--status", status]);
        assert_error_code(&error, "INVALID_ARGUMENT");
        assert_eq!(exit_code, 2);
    }
}

// ---------------------------------------------------------------------------
// REM-006 查询提醒：全部 / 时间范围（闭区间、按 julianday 跨时区比较）/ 排序
// ---------------------------------------------------------------------------
#[test]
fn rem_006_list_all_and_time_range() {
    let db = TestDb::new();
    // A = 01:00Z（输入为 +08:00），B = 05:00Z，D = 05:00Z（与 B 同一时刻），C = 06:00Z。
    let a = create_reminder(&db, "A", "2026-11-17T09:00:00+08:00");
    let b = create_reminder(&db, "B", "2026-11-17T05:00:00Z");
    let d = create_reminder(&db, "D", "2026-11-17T05:00:00Z");
    let c = create_reminder(&db, "C", "2026-11-17T06:00:00Z");

    // 全部：按 julianday(remind_at) 升序（A 的原始串字典序并不小于 B，证明按真实时刻排序）；
    // 同一时刻按 id ASC 稳定排序。
    let mut same_time = [id_of(&b), id_of(&d)];
    same_time.sort();
    let all = list_reminders(&db, &[]);
    assert_eq!(
        ids(&all),
        vec![
            id_of(&a),
            same_time[0].clone(),
            same_time[1].clone(),
            id_of(&c)
        ]
    );
    assert_eq!(all[0]["remind_at"], "2026-11-17T09:00:00+08:00");

    // 闭区间 [01:00Z, 05:00Z]：包含两端边界。
    let ranged = list_reminders(
        &db,
        &[
            "--from",
            "2026-11-17T01:00:00Z",
            "--to",
            "2026-11-17T05:00:00Z",
        ],
    );
    assert_eq!(
        ids(&ranged),
        vec![id_of(&a), same_time[0].clone(), same_time[1].clone()]
    );

    // 两端边界一致时命中该时刻的记录。
    let exact = list_reminders(
        &db,
        &[
            "--from",
            "2026-11-17T05:00:00Z",
            "--to",
            "2026-11-17T05:00:00Z",
        ],
    );
    assert_eq!(exact.len(), 2);

    // 只有 from / 只有 to。
    let from_only = list_reminders(&db, &["--from", "2026-11-17T05:00:00Z"]);
    assert_eq!(from_only.len(), 3);
    let to_only = list_reminders(&db, &["--to", "2026-11-17T01:00:00Z"]);
    assert_eq!(ids(&to_only), vec![id_of(&a)]);

    // 无命中返回空列表且成功。
    let none = list_reminders(
        &db,
        &[
            "--from",
            "2026-11-18T00:00:00Z",
            "--to",
            "2026-11-19T00:00:00Z",
        ],
    );
    assert!(none.is_empty());

    // 非法时间范围参数必须报错，而不是被忽略。
    let (error, exit_code) = db.fail_json(&["reminder", "list", "--from", "not-a-time"]);
    assert_error_code(&error, "INVALID_ARGUMENT");
    assert_eq!(exit_code, 2);
    let (error, _) = db.fail_json(&["reminder", "list", "--to", "2026-11-17"]);
    assert_error_code(&error, "INVALID_ARGUMENT");
}

// ---------------------------------------------------------------------------
// REM-007 修改提醒时间：新时间生效，未提供字段不覆盖
// ---------------------------------------------------------------------------
#[test]
fn rem_007_update_remind_at_keeps_other_fields() {
    let db = TestDb::new();
    let task_id = create_task(&db, "关联待办");
    let created = db.ok_json(&create_args(
        "每天喝水",
        "2026-11-17T15:00:00+08:00",
        &["--item-id", task_id.as_str()],
    ));
    let id = id_of(&created);

    let updated = db.ok_json(&[
        "reminder",
        "update",
        "--id",
        id.as_str(),
        "--remind-at",
        "2026-11-18T09:30:00+08:00",
    ]);

    assert_eq!(id_of(&updated), id);
    assert_eq!(updated["remind_at"], "2026-11-18T09:30:00+08:00");
    // 未提供字段保持原值。
    assert_eq!(updated["content"], "每天喝水");
    assert_eq!(updated["item_id"], task_id);
    assert_eq!(updated["status"], PENDING);
    assert_eq!(updated["user_id"], db.user_id);

    // 新时间生效：按新时间闭区间可查到，旧时间范围查不到。
    let hit = list_reminders(
        &db,
        &[
            "--from",
            "2026-11-18T09:30:00+08:00",
            "--to",
            "2026-11-18T09:30:00+08:00",
        ],
    );
    assert_eq!(ids(&hit), vec![id.clone()]);
    let miss = list_reminders(
        &db,
        &[
            "--from",
            "2026-11-17T15:00:00+08:00",
            "--to",
            "2026-11-17T15:00:00+08:00",
        ],
    );
    assert!(miss.is_empty());
}

// ---------------------------------------------------------------------------
// REM-007（补充）修改内容 / --clear-item-id 显式解除关联
// ---------------------------------------------------------------------------
#[test]
fn rem_007_update_content_and_clear_item_id() {
    let db = TestDb::new();
    let task_id = create_task(&db, "待办");
    let created = db.ok_json(&create_args(
        "原始内容",
        "2026-11-17T15:00:00+08:00",
        &["--item-id", task_id.as_str()],
    ));
    let id = id_of(&created);

    // 只改内容：不覆盖已有 item_id / status。
    let updated = db.ok_json(&[
        "reminder",
        "update",
        "--id",
        id.as_str(),
        "--content",
        "新内容",
    ]);
    assert_eq!(updated["content"], "新内容");
    assert_eq!(updated["item_id"], task_id);
    assert_eq!(updated["status"], PENDING);

    // 显式清空：item_id 变为 null，而不是被忽略。
    let cleared = db.ok_json(&["reminder", "update", "--id", id.as_str(), "--clear-item-id"]);
    assert!(cleared["item_id"].is_null());
    assert_eq!(cleared["content"], "新内容");

    // 落库确认。
    let fetched = db.ok_json(&["reminder", "get", "--id", id.as_str()]);
    assert!(fetched["item_id"].is_null());

    // 修改不存在的提醒返回 REMINDER_NOT_FOUND。
    let (error, exit_code) =
        db.fail_json(&["reminder", "update", "--id", "no-such-id", "--content", "x"]);
    assert_error_code(&error, "REMINDER_NOT_FOUND");
    assert_eq!(exit_code, 1);

    // 修改内容为空同样被拒绝。
    let (error, _) = db.fail_json(&["reminder", "update", "--id", id.as_str(), "--content", "  "]);
    assert_error_code(&error, "INVALID_ARGUMENT");
}

// ---------------------------------------------------------------------------
// REM-008 取消提醒：记录保留，状态变为 cancelled
// ---------------------------------------------------------------------------
#[test]
fn rem_008_cancel_reminder_keeps_record() {
    let db = TestDb::new();
    let created = create_reminder(&db, "取消我", "2026-11-17T15:00:00+08:00");
    let id = id_of(&created);

    let cancelled = db.ok_json(&["reminder", "cancel", "--id", id.as_str()]);
    assert_eq!(id_of(&cancelled), id);
    assert_eq!(cancelled["status"], CANCELLED);
    // 取消不等于删除：记录与内容仍然保留。
    assert_eq!(cancelled["content"], "取消我");
    assert_eq!(cancelled["remind_at"], "2026-11-17T15:00:00+08:00");

    let fetched = db.ok_json(&["reminder", "get", "--id", id.as_str()]);
    assert_eq!(fetched["status"], CANCELLED);

    // 取消后不再出现在 pending 列表，但出现在 cancelled 列表。
    assert!(list_reminders(&db, &["--status", PENDING]).is_empty());
    let cancelled_items = list_reminders(&db, &["--status", CANCELLED]);
    assert_eq!(ids(&cancelled_items), vec![id.clone()]);

    // 重复取消幂等：仍然成功且保持 cancelled。
    let again = db.ok_json(&["reminder", "cancel", "--id", id.as_str()]);
    assert_eq!(again["status"], CANCELLED);

    // 取消不存在的提醒返回 REMINDER_NOT_FOUND。
    let (error, exit_code) = db.fail_json(&["reminder", "cancel", "--id", "no-such-id"]);
    assert_error_code(&error, "REMINDER_NOT_FOUND");
    assert_eq!(exit_code, 1);
}

// ---------------------------------------------------------------------------
// REM-009 删除提醒：物理删除，不存在返回 REMINDER_NOT_FOUND
// ---------------------------------------------------------------------------
#[test]
fn rem_009_delete_reminder() {
    let db = TestDb::new();
    let created = create_reminder(&db, "待删除", "2026-11-17T15:00:00+08:00");
    let id = id_of(&created);

    let deleted = db.ok_json(&["reminder", "delete", "--id", id.as_str()]);
    assert_eq!(deleted["id"], id);
    assert_eq!(deleted["deleted"], true);

    // 删除后查询返回 REMINDER_NOT_FOUND。
    let (error, exit_code) = db.fail_json(&["reminder", "get", "--id", id.as_str()]);
    assert_error_code(&error, "REMINDER_NOT_FOUND");
    assert_eq!(exit_code, 1);

    // 重复删除返回明确的不存在错误（不崩溃）。
    let (error, exit_code) = db.fail_json(&["reminder", "delete", "--id", id.as_str()]);
    assert_error_code(&error, "REMINDER_NOT_FOUND");
    assert_eq!(exit_code, 1);

    assert!(list_reminders(&db, &[]).is_empty());
}

// ---------------------------------------------------------------------------
// REM-010 关联存在的待办：item_id 指向正确事项 + 按 item_id 筛选
// ---------------------------------------------------------------------------
#[test]
fn rem_010_link_to_item_and_filter_by_item_id() {
    let db = TestDb::new();
    let first_task = create_task(&db, "第一个待办");
    let second_task = create_task(&db, "第二个待办");

    // 关联第二个待办，item_id 必须精确指向它（而不是第一个）。
    let created = db.ok_json(&create_args(
        "关联提醒",
        "2026-11-17T15:00:00+08:00",
        &["--item-id", second_task.as_str()],
    ));
    assert_eq!(created["item_id"], second_task);
    assert_ne!(created["item_id"], first_task);

    // 按 item_id 筛选只返回关联该事项的提醒。
    let of_second = list_reminders(&db, &["--item-id", second_task.as_str()]);
    assert_eq!(ids(&of_second), vec![id_of(&created)]);
    assert!(list_reminders(&db, &["--item-id", first_task.as_str()]).is_empty());
}

// ---------------------------------------------------------------------------
// REM-011 关联不存在 / 不属于当前用户的事项：ITEM_NOT_FOUND，不产生孤立关联
// ---------------------------------------------------------------------------
#[test]
fn rem_011_link_to_missing_item_is_rejected() {
    let db = TestDb::new();
    let missing = "00000000-0000-0000-0000-000000000000";

    let (error, exit_code) = db.fail_json(&create_args(
        "孤立提醒",
        "2026-11-17T15:00:00+08:00",
        &["--item-id", missing],
    ));
    assert_error_code(&error, "ITEM_NOT_FOUND");
    assert_eq!(exit_code, 1);

    // 失败时不得留下指向不存在事项的孤立关联记录。
    assert!(list_reminders(&db, &[]).is_empty());

    // update 指向不存在的事项同样被拒绝，且原关联保持不变。
    let task_id = create_task(&db, "有效待办");
    let created = db.ok_json(&create_args(
        "有效关联",
        "2026-11-17T15:00:00+08:00",
        &["--item-id", task_id.as_str()],
    ));
    let id = id_of(&created);
    let (error, exit_code) = db.fail_json(&[
        "reminder",
        "update",
        "--id",
        id.as_str(),
        "--item-id",
        missing,
    ]);
    assert_error_code(&error, "ITEM_NOT_FOUND");
    assert_eq!(exit_code, 1);
    let fetched = db.ok_json(&["reminder", "get", "--id", id.as_str()]);
    assert_eq!(fetched["item_id"], task_id, "失败更新不得改变原关联");
}

// ---------------------------------------------------------------------------
// REM-012 / REL-003 关联待办被删除：提醒保留且 item_id 置为 null
// ---------------------------------------------------------------------------
#[test]
fn rem_012_rel_003_delete_linked_task_sets_null() {
    let db = TestDb::new();
    let task_id = create_task(&db, "将被删除的待办");
    let created = db.ok_json(&create_args(
        "依赖待办的提醒",
        "2026-11-17T15:00:00+08:00",
        &["--item-id", task_id.as_str()],
    ));
    let reminder_id = id_of(&created);
    assert_eq!(created["item_id"], task_id);

    // 删除被提醒引用的待办（应用层在同一事务中先置空关联再删除事项）。
    let deleted = db.ok_json(&["task", "delete", "--id", task_id.as_str()]);
    assert_eq!(deleted["id"], task_id);
    assert_eq!(deleted["deleted"], true);

    // REL-003：提醒记录仍在，但 item_id 已解除关联。
    let fetched = db.ok_json(&["reminder", "get", "--id", reminder_id.as_str()]);
    assert_eq!(id_of(&fetched), reminder_id);
    assert_eq!(fetched["content"], "依赖待办的提醒");
    assert_eq!(fetched["status"], PENDING);
    assert!(fetched["item_id"].is_null(), "待办删除后 item_id 应为 null");
    assert_eq!(list_reminders(&db, &[]).len(), 1);
    // 不再能按旧的 item_id 筛到它。
    assert!(list_reminders(&db, &["--item-id", task_id.as_str()]).is_empty());
}

// ---------------------------------------------------------------------------
// REL-002 修改关联待办标题后，提醒关联仍然有效
// ---------------------------------------------------------------------------
#[test]
fn rel_002_update_task_title_keeps_reminder_link() {
    let db = TestDb::new();
    let task_id = create_task(&db, "原标题");
    let created = db.ok_json(&create_args(
        "联动提醒",
        "2026-11-17T15:00:00+08:00",
        &["--item-id", task_id.as_str()],
    ));
    let reminder_id = id_of(&created);

    let updated_task = db.ok_json(&[
        "task",
        "update",
        "--id",
        task_id.as_str(),
        "--title",
        "新标题",
    ]);
    assert_eq!(updated_task["title"], "新标题");

    // 提醒的 item_id 不因被关联事项的其他字段修改而改变。
    let fetched = db.ok_json(&["reminder", "get", "--id", reminder_id.as_str()]);
    assert_eq!(fetched["item_id"], task_id);
    assert_eq!(fetched["content"], "联动提醒");

    // 关联的目标仍可查回（真实有效，而非悬空 ID）。
    let task = db.ok_json(&["task", "get", "--id", task_id.as_str()]);
    assert_eq!(task["id"], task_id);
    assert_eq!(task["title"], "新标题");
}

// ---------------------------------------------------------------------------
// REM-013 按状态查询：正确区分 pending / triggered / cancelled
// ---------------------------------------------------------------------------
#[test]
fn rem_013_filter_by_status() {
    let db = TestDb::new();
    let pending = create_reminder(&db, "保持待触发", "2026-11-17T15:00:00+08:00");
    let triggered = create_reminder(&db, "已触发", "2026-11-17T16:00:00+08:00");
    let cancelled = create_reminder(&db, "已取消", "2026-11-17T17:00:00+08:00");
    db.ok_json(&["reminder", "trigger", "--id", id_of(&triggered).as_str()]);
    db.ok_json(&["reminder", "cancel", "--id", id_of(&cancelled).as_str()]);

    let pending_items = list_reminders(&db, &["--status", PENDING]);
    assert_eq!(ids(&pending_items), vec![id_of(&pending)]);
    assert_eq!(pending_items[0]["status"], PENDING);

    let triggered_items = list_reminders(&db, &["--status", TRIGGERED]);
    assert_eq!(ids(&triggered_items), vec![id_of(&triggered)]);
    assert_eq!(triggered_items[0]["status"], TRIGGERED);

    let cancelled_items = list_reminders(&db, &["--status", CANCELLED]);
    assert_eq!(ids(&cancelled_items), vec![id_of(&cancelled)]);
    assert_eq!(cancelled_items[0]["status"], CANCELLED);
}

// ---------------------------------------------------------------------------
// REM-014 重复创建：不擅自去重，两条记录都存在（ID 不同）
// ---------------------------------------------------------------------------
#[test]
fn rem_014_duplicate_create_does_not_dedupe() {
    let db = TestDb::new();
    let first = create_reminder(&db, "喝水", "2026-11-17T15:00:00+08:00");
    let second = create_reminder(&db, "喝水", "2026-11-17T15:00:00+08:00");

    assert_ne!(id_of(&first), id_of(&second), "重复创建必须生成不同 ID");
    assert_eq!(first["content"], second["content"]);
    assert_eq!(first["remind_at"], second["remind_at"]);

    let items = list_reminders(&db, &[]);
    assert_eq!(items.len(), 2, "相同内容与时间的提醒不应被去重");
}

// ---------------------------------------------------------------------------
// REM-015 触发提醒：pending → triggered 幂等；cancelled 不允许触发（固定规则）
// ---------------------------------------------------------------------------
#[test]
fn rem_015_trigger_lifecycle() {
    let db = TestDb::new();
    let created = create_reminder(&db, "到点提醒", "2026-11-17T15:00:00+08:00");
    let id = id_of(&created);
    assert_eq!(created["status"], PENDING);

    let triggered = db.ok_json(&["reminder", "trigger", "--id", id.as_str()]);
    assert_eq!(id_of(&triggered), id);
    assert_eq!(triggered["status"], TRIGGERED);

    // 重复触发幂等：保持 triggered 且不报错。
    let again = db.ok_json(&["reminder", "trigger", "--id", id.as_str()]);
    assert_eq!(again["status"], TRIGGERED);

    // 已取消的提醒不允许触发：固定规则为参数错误 INVALID_ARGUMENT。
    let cancelled = create_reminder(&db, "已取消提醒", "2026-11-17T16:00:00+08:00");
    let cancelled_id = id_of(&cancelled);
    db.ok_json(&["reminder", "cancel", "--id", cancelled_id.as_str()]);
    let (error, exit_code) = db.fail_json(&["reminder", "trigger", "--id", cancelled_id.as_str()]);
    assert_error_code(&error, "INVALID_ARGUMENT");
    assert_eq!(exit_code, 2);
    // 状态保持不变。
    let fetched = db.ok_json(&["reminder", "get", "--id", cancelled_id.as_str()]);
    assert_eq!(fetched["status"], CANCELLED);

    // 触发不存在的提醒返回 REMINDER_NOT_FOUND。
    let (error, exit_code) = db.fail_json(&["reminder", "trigger", "--id", "no-such-id"]);
    assert_error_code(&error, "REMINDER_NOT_FOUND");
    assert_eq!(exit_code, 1);
}

// ---------------------------------------------------------------------------
// REM-016 跨用户隔离：数据按 user_id 隔离，跨用户访问表现为不存在
// ---------------------------------------------------------------------------
#[test]
fn rem_016_cross_user_isolation() {
    let db = TestDb::new();
    let other_user = db.add_user("qq", "other-user");

    let task_of_a = create_task(&db, "A 的待办");
    let reminder_of_a = create_reminder(&db, "A 的提醒", "2026-11-17T15:00:00+08:00");
    let id_a = id_of(&reminder_of_a);

    let reminder_of_b = db.ok_json_as(
        &other_user,
        &create_args("B 的提醒", "2026-11-17T16:00:00+08:00", &[]),
    );
    assert_eq!(reminder_of_b["user_id"], other_user);
    let id_b = id_of(&reminder_of_b);

    // B 的列表只看到自己的提醒。
    let list_b = list_reminders_as(&db, &other_user, &[]);
    assert_eq!(ids(&list_b), vec![id_b.clone()]);

    // B 不能 get / update / cancel / trigger / delete A 的提醒（表现为不存在）。
    for args in [
        vec!["reminder", "get", "--id", id_a.as_str()],
        vec![
            "reminder",
            "update",
            "--id",
            id_a.as_str(),
            "--content",
            "x",
        ],
        vec!["reminder", "cancel", "--id", id_a.as_str()],
        vec!["reminder", "trigger", "--id", id_a.as_str()],
        vec!["reminder", "delete", "--id", id_a.as_str()],
    ] {
        let (error, exit_code) = db.fail_json_as(&other_user, &args);
        assert_error_code(&error, "REMINDER_NOT_FOUND");
        assert_eq!(exit_code, 1);
    }

    // B 不能关联 A 的事项：ITEM_NOT_FOUND，且不落库。
    let (error, exit_code) = db.fail_json_as(
        &other_user,
        &create_args(
            "B 想关联 A 的待办",
            "2026-11-17T17:00:00+08:00",
            &["--item-id", task_of_a.as_str()],
        ),
    );
    assert_error_code(&error, "ITEM_NOT_FOUND");
    assert_eq!(exit_code, 1);
    assert_eq!(list_reminders_as(&db, &other_user, &[]).len(), 1);

    // A 的数据未被跨用户操作破坏。
    let fetched = db.ok_json(&["reminder", "get", "--id", id_a.as_str()]);
    assert_eq!(fetched["status"], PENDING);
    assert_eq!(fetched["content"], "A 的提醒");

    // 业务命令缺少 --user-id → INVALID_ARGUMENT（退出码 2）。
    let output = db.raw_json_without_user(&["reminder", "list"]);
    let exit_code = output.status.code().unwrap_or(-1);
    let error = parse_json(&output);
    assert_error_code(&error, "INVALID_ARGUMENT");
    assert_eq!(exit_code, 2);
}

// ---------------------------------------------------------------------------
// --input-json（stdin）创建 / 修改；非法 JSON 返回 INVALID_INPUT
// ---------------------------------------------------------------------------
#[test]
fn reminder_cli_supports_input_json() {
    let db = TestDb::new();

    let output = db.raw_json_with_stdin(
        &["reminder", "create", "--input-json", "-"],
        r#"{"content":"来自 JSON","remind_at":"2026-11-17T15:00:00+08:00"}"#,
    );
    common::assert_success(&output, &["reminder", "create", "--input-json", "-"]);
    let response = parse_json(&output);
    let data = response["data"].clone();
    let id = id_of(&data);
    assert_eq!(data["content"], "来自 JSON");
    assert_eq!(data["status"], PENDING);
    assert!(data["item_id"].is_null());

    // update 同样支持 stdin JSON：只修改提供的字段。
    let update_input = format!(r#"{{"id":"{id}","content":"JSON 更新"}}"#);
    let output =
        db.raw_json_with_stdin(&["reminder", "update", "--input-json", "-"], &update_input);
    common::assert_success(&output, &["reminder", "update", "--input-json", "-"]);
    let updated = parse_json(&output)["data"].clone();
    assert_eq!(updated["id"], id);
    assert_eq!(updated["content"], "JSON 更新");
    assert_eq!(updated["remind_at"], "2026-11-17T15:00:00+08:00");

    // 非法 JSON 输入 → INVALID_INPUT（退出码 2），不产生记录。
    let before = list_reminders(&db, &[]).len();
    let output = db.raw_json_with_stdin(
        &["reminder", "create", "--input-json", "-"],
        "{not valid json",
    );
    let exit_code = output.status.code().unwrap_or(-1);
    let error = parse_json(&output);
    assert_error_code(&error, "INVALID_INPUT");
    assert_eq!(exit_code, 2);
    assert_eq!(list_reminders(&db, &[]).len(), before);
}
