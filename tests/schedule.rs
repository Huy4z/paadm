//! Schedule（日程）领域集成测试。
//!
//! 日程与待办统一存于 `items` 表（`type='event'`），所有数据按 `user_id` 隔离。
//! 测试通过 `common::TestDb` 驱动真实 `paadm` 二进制，每个测试使用独立临时数据库。
//!
//! 覆盖测试用例 `SCH-001` ~ `SCH-014`（以及补充的更新/排序/关键词/用户隔离等）。
//!
//! 固定规则：
//! - `start_at` 必填、必须带时区偏移；`end_at` 可选，存在时必须 **严格晚于** `start_at`
//!   （应用层按时间点比较，数据库 CHECK 兜底）；
//! - `all_day` 用 0/1 存储，JSON 输出 bool；
//! - 列表排序 `julianday(start_at) ASC, id ASC`（跨时区按时间点正确排序）；
//! - 区间查询固定为 `[from, to)` 与日程区间相交：
//!   `start < to AND COALESCE(end_at, start_at) > from`（无 end_at 视为瞬时事件）。

mod common;

use common::{TestDb, assert_error_code, id_of, parse_json};
use serde_json::Value;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

/// 2026-11-20 这一天（+08:00）。
const DAY_FROM: &str = "2026-11-20T00:00:00+08:00";
/// 2026-11-21 00:00（+08:00），即 20 日区间的右开边界。
const DAY_TO: &str = "2026-11-21T00:00:00+08:00";

// ---------------------------------------------------------------------------
// 测试辅助
// ---------------------------------------------------------------------------

fn create(db: &TestDb, args: &[&str]) -> Value {
    let mut command: Vec<&str> = vec!["schedule", "create"];
    command.extend_from_slice(args);
    db.ok_json(&command)
}

fn list(db: &TestDb, args: &[&str]) -> Vec<Value> {
    let mut command: Vec<&str> = vec!["schedule", "list"];
    command.extend_from_slice(args);
    db.ok_items(&command)
}

fn ids(items: &[Value]) -> Vec<String> {
    items.iter().map(id_of).collect()
}

fn sorted_ids(items: &[Value]) -> Vec<String> {
    let mut values = ids(items);
    values.sort();
    values
}

/// 解析带时区偏移的 ISO 8601 时间点，用于按真实时间顺序断言。
fn instant(value: &str) -> OffsetDateTime {
    OffsetDateTime::parse(value, &Rfc3339)
        .unwrap_or_else(|err| panic!("invalid datetime {value}: {err}"))
}

/// 断言 event 列表按 `start_at` 时间点 ASC、id ASC 排序（跨时区正确）。
fn assert_event_order(items: &[Value]) {
    let actual: Vec<(OffsetDateTime, String)> = items
        .iter()
        .map(|item| {
            let start_at = item["start_at"]
                .as_str()
                .unwrap_or_else(|| panic!("missing start_at in {item}"));
            (instant(start_at), id_of(item))
        })
        .collect();
    let mut expected = actual.clone();
    expected.sort();
    assert_eq!(actual, expected, "events not sorted by (start instant, id)");
}

fn assert_minimal_event_shape(data: &Value, user_id: &str) {
    assert!(data["id"].is_string(), "id must be a string: {data}");
    assert_eq!(data["user_id"], user_id);
    assert_eq!(data["type"], "event", "schedule create must set type=event");
    assert_eq!(data["status"], "pending");
    assert_eq!(data["priority"], Value::Null);
    assert_eq!(data["all_day"], false, "all_day defaults to false/0");
    assert!(data["end_at"].is_null());
    assert!(data["due_at"].is_null());
    assert!(data["location"].is_null());
    assert!(data["description"].is_null());
    assert!(data["completed_at"].is_null());
    assert!(data["created_at"].is_string());
    assert!(data["updated_at"].is_string());
}

// ---------------------------------------------------------------------------
// 创建：SCH-001 .. SCH-007
// ---------------------------------------------------------------------------

/// SCH-001：创建最小日程（标题 + 开始时间）→ 成功，type=event、status=pending。
#[test]
fn sch_001_create_minimal_schedule() {
    let db = TestDb::new();
    let data = create(
        &db,
        &["--title", "组会", "--start-at", "2026-11-20T10:00:00+08:00"],
    );

    assert_eq!(data["title"], "组会");
    assert_eq!(data["start_at"], "2026-11-20T10:00:00+08:00");
    assert_minimal_event_shape(&data, &db.user_id);

    let got = db.ok_json(&["schedule", "get", "--id", id_of(&data).as_str()]);
    assert_eq!(got, data);
}

/// SCH-002：创建完整日程（标题/描述/起止时间/地点/全天）→ 字段正确保存并返回。
#[test]
fn sch_002_create_full_schedule() {
    let db = TestDb::new();
    let data = create(
        &db,
        &[
            "--title",
            "项目评审",
            "--description",
            "带材料",
            "--start-at",
            "2026-11-20T10:00:00+08:00",
            "--end-at",
            "2026-11-20T11:30:00+08:00",
            "--location",
            "会议室 A",
            "--all-day",
            "1",
        ],
    );

    assert_eq!(data["title"], "项目评审");
    assert_eq!(data["description"], "带材料");
    assert_eq!(data["start_at"], "2026-11-20T10:00:00+08:00");
    assert_eq!(data["end_at"], "2026-11-20T11:30:00+08:00");
    assert_eq!(data["location"], "会议室 A");
    // all_day 在数据库中为 0/1，JSON 输出 bool。
    assert_eq!(data["all_day"], true);

    let got = db.ok_json(&["schedule", "get", "--id", id_of(&data).as_str()]);
    assert_eq!(got["all_day"], true);
    assert_eq!(got["end_at"], "2026-11-20T11:30:00+08:00");
}

/// SCH-003：标题为空 → INVALID_ARGUMENT，不创建记录。
#[test]
fn sch_003_empty_title_rejected() {
    let db = TestDb::new();
    let (error, exit_code) = db.fail_json(&[
        "schedule",
        "create",
        "--title",
        "",
        "--start-at",
        "2026-11-20T10:00:00+08:00",
    ]);
    assert_error_code(&error, "INVALID_ARGUMENT");
    assert_eq!(exit_code, 2);
    assert!(list(&db, &[]).is_empty());
}

/// SCH-004：开始时间非法 / 缺少开始时间 → INVALID_ARGUMENT，不创建记录。
#[test]
fn sch_004_invalid_start_at_rejected() {
    let db = TestDb::new();

    for bad_start in ["not-a-time", "2026-11-20T10:00:00", "2026-11-20"] {
        let (error, _) = db.fail_json(&[
            "schedule",
            "create",
            "--title",
            "时间非法",
            "--start-at",
            bad_start,
        ]);
        assert_error_code(&error, "INVALID_ARGUMENT");
    }

    // 缺少 --start-at（CLI 必填参数）同样是参数错误。
    let (error, exit_code) = db.fail_json(&["schedule", "create", "--title", "没有开始时间"]);
    assert_error_code(&error, "INVALID_ARGUMENT");
    assert_eq!(exit_code, 2);

    assert!(list(&db, &[]).is_empty());
}

/// SCH-005：结束时间早于开始时间 → INVALID_ARGUMENT，不创建记录。
#[test]
fn sch_005_end_before_start_rejected() {
    let db = TestDb::new();
    let (error, _) = db.fail_json(&[
        "schedule",
        "create",
        "--title",
        "结束早于开始",
        "--start-at",
        "2026-11-20T10:00:00+08:00",
        "--end-at",
        "2026-11-20T09:00:00+08:00",
    ]);
    assert_error_code(&error, "INVALID_ARGUMENT");
    assert!(list(&db, &[]).is_empty());
}

/// SCH-006：结束时间等于开始时间 → 拒绝（规则固定为“必须严格晚于”）。
#[test]
fn sch_006_end_equal_start_rejected() {
    let db = TestDb::new();
    let (error, _) = db.fail_json(&[
        "schedule",
        "create",
        "--title",
        "零长度日程",
        "--start-at",
        "2026-11-20T10:00:00+08:00",
        "--end-at",
        "2026-11-20T10:00:00+08:00",
    ]);
    assert_error_code(&error, "INVALID_ARGUMENT");
    assert!(list(&db, &[]).is_empty());
}

/// SCH-007：不提供 end_at 是合法的，不自动编造结束时间。
#[test]
fn sch_007_no_end_at_allowed() {
    let db = TestDb::new();
    let data = create(
        &db,
        &[
            "--title",
            "瞬时日程",
            "--start-at",
            "2026-11-20T10:00:00+08:00",
        ],
    );
    assert!(data["end_at"].is_null());
    assert!(list(&db, &[])[0]["end_at"].is_null());
}

// ---------------------------------------------------------------------------
// 查询：SCH-008 .. SCH-009
// ---------------------------------------------------------------------------

/// SCH-008：按日期查询 → 返回与该日期区间 [from, to) 相交的日程，边界规则明确。
#[test]
fn sch_008_list_by_date_returns_intersecting_schedules() {
    let db = TestDb::new();
    // 完全在 20 日内。
    let inside = create(
        &db,
        &[
            "--title",
            "日内",
            "--start-at",
            "2026-11-20T10:00:00+08:00",
            "--end-at",
            "2026-11-20T11:00:00+08:00",
        ],
    );
    // 从 19 日跨入 20 日。
    let crossing_in = create(
        &db,
        &[
            "--title",
            "跨入",
            "--start-at",
            "2026-11-19T23:00:00+08:00",
            "--end-at",
            "2026-11-20T01:00:00+08:00",
        ],
    );
    // 恰好从 21 日 00:00 开始 → 不在 [20 日, 21 日) 内。
    let starts_at_end = create(
        &db,
        &[
            "--title",
            "右边界开始",
            "--start-at",
            "2026-11-21T00:00:00+08:00",
            "--end-at",
            "2026-11-21T01:00:00+08:00",
        ],
    );
    // 从 20 日跨出到 21 日。
    let crossing_out = create(
        &db,
        &[
            "--title",
            "跨出",
            "--start-at",
            "2026-11-20T23:30:00+08:00",
            "--end-at",
            "2026-11-21T00:30:00+08:00",
        ],
    );
    // 完全在 19 日。
    let previous_day = create(
        &db,
        &[
            "--title",
            "前一天",
            "--start-at",
            "2026-11-19T09:00:00+08:00",
            "--end-at",
            "2026-11-19T10:00:00+08:00",
        ],
    );

    let items = list(&db, &["--from", DAY_FROM, "--to", DAY_TO]);
    let expected = sorted_ids(&[inside, crossing_in, crossing_out]);
    assert_eq!(sorted_ids(&items), expected);
    assert!(!ids(&items).contains(&id_of(&starts_at_end)));
    assert!(!ids(&items).contains(&id_of(&previous_day)));
}

/// SCH-009：查询指定时间段 → 返回与 [from, to) 相交的日程，不把无记录说成绝对空闲。
#[test]
fn sch_009_list_within_time_range() {
    let db = TestDb::new();
    let before = create(
        &db,
        &[
            "--title",
            "更早",
            "--start-at",
            "2026-11-20T09:00:00+08:00",
            "--end-at",
            "2026-11-20T10:00:00+08:00",
        ],
    );
    let overlapping_start = create(
        &db,
        &[
            "--title",
            "与起点相交",
            "--start-at",
            "2026-11-20T10:00:00+08:00",
            "--end-at",
            "2026-11-20T11:00:00+08:00",
        ],
    );
    let overlapping_end = create(
        &db,
        &[
            "--title",
            "与终点相交",
            "--start-at",
            "2026-11-20T11:00:00+08:00",
            "--end-at",
            "2026-11-20T12:00:00+08:00",
        ],
    );
    let after = create(
        &db,
        &[
            "--title",
            "更晚",
            "--start-at",
            "2026-11-20T12:00:00+08:00",
            "--end-at",
            "2026-11-20T13:00:00+08:00",
        ],
    );

    let items = list(
        &db,
        &[
            "--from",
            "2026-11-20T10:15:00+08:00",
            "--to",
            "2026-11-20T11:30:00+08:00",
        ],
    );
    let expected = sorted_ids(&[overlapping_start, overlapping_end]);
    assert_eq!(sorted_ids(&items), expected);
    assert!(!ids(&items).contains(&id_of(&before)));
    assert!(!ids(&items).contains(&id_of(&after)));
}

// ---------------------------------------------------------------------------
// 修改与删除：SCH-010 .. SCH-014
// ---------------------------------------------------------------------------

/// SCH-010：修改时间与地点 → 新值生效，其他字段不变。
#[test]
fn sch_010_update_time_and_location_keeps_other_fields() {
    let db = TestDb::new();
    let created = create(
        &db,
        &[
            "--title",
            "改时间",
            "--description",
            "原描述",
            "--start-at",
            "2026-11-20T10:00:00+08:00",
            "--end-at",
            "2026-11-20T11:00:00+08:00",
            "--location",
            "旧地点",
            "--all-day",
            "1",
        ],
    );

    let updated = db.ok_json(&[
        "schedule",
        "update",
        "--id",
        id_of(&created).as_str(),
        "--start-at",
        "2026-11-21T14:00:00+08:00",
        "--end-at",
        "2026-11-21T15:30:00+08:00",
        "--location",
        "新地点",
    ]);
    assert_eq!(updated["start_at"], "2026-11-21T14:00:00+08:00");
    assert_eq!(updated["end_at"], "2026-11-21T15:30:00+08:00");
    assert_eq!(updated["location"], "新地点");
    assert_eq!(updated["title"], "改时间");
    assert_eq!(updated["description"], "原描述");
    assert_eq!(updated["all_day"], true);
    assert_eq!(updated["created_at"], created["created_at"]);
}

/// SCH-011：删除存在的日程 → 删除成功，之后查询返回 SCHEDULE_NOT_FOUND。
#[test]
fn sch_011_delete_schedule() {
    let db = TestDb::new();
    let created = create(
        &db,
        &[
            "--title",
            "要删除",
            "--start-at",
            "2026-11-20T10:00:00+08:00",
        ],
    );
    let id = id_of(&created);

    let data = db.ok_json(&["schedule", "delete", "--id", id.as_str()]);
    assert_eq!(data["id"], id);
    assert_eq!(data["deleted"], true);
    assert!(list(&db, &[]).is_empty());

    let (error, _) = db.fail_json(&["schedule", "get", "--id", id.as_str()]);
    assert_error_code(&error, "SCHEDULE_NOT_FOUND");
}

/// SCH-012：查询不存在的日程 → SCHEDULE_NOT_FOUND，退出码 1。
#[test]
fn sch_012_get_missing_schedule_returns_not_found() {
    let db = TestDb::new();
    let (error, exit_code) = db.fail_json(&["schedule", "get", "--id", "does-not-exist"]);
    assert_error_code(&error, "SCHEDULE_NOT_FOUND");
    assert_eq!(exit_code, 1);
}

/// SCH-013：时间边界按统一区间规则 [from, to) 处理。
#[test]
fn sch_013_boundary_instants_follow_half_open_rule() {
    let db = TestDb::new();
    // 恰好在 from 开始、但仍覆盖区间内部 → 相交。
    let starts_at_from = create(
        &db,
        &[
            "--title",
            "左边界开始",
            "--start-at",
            "2026-11-20T10:00:00+08:00",
            "--end-at",
            "2026-11-20T11:00:00+08:00",
        ],
    );
    // 恰好在 to 开始 → 不相交（右开）。
    let starts_at_to = create(
        &db,
        &[
            "--title",
            "右边界开始",
            "--start-at",
            "2026-11-20T12:00:00+08:00",
            "--end-at",
            "2026-11-20T13:00:00+08:00",
        ],
    );
    // 恰好在 from 结束 → 不相交（左边界按 > from 判断）。
    let ends_at_from = create(
        &db,
        &[
            "--title",
            "左边界结束",
            "--start-at",
            "2026-11-20T09:00:00+08:00",
            "--end-at",
            "2026-11-20T10:00:00+08:00",
        ],
    );
    // 区间内部。
    let inside = create(
        &db,
        &[
            "--title",
            "区间内部",
            "--start-at",
            "2026-11-20T11:00:00+08:00",
            "--end-at",
            "2026-11-20T11:30:00+08:00",
        ],
    );
    // 无 end_at 的瞬时事项：COALESCE(end_at, start_at) = start_at。
    // 恰好等于 from 时按 “> from” 规则不相交（固定规则）。
    let instant_at_from = create(
        &db,
        &[
            "--title",
            "瞬时左边界",
            "--start-at",
            "2026-11-20T10:00:00+08:00",
        ],
    );
    let instant_inside = create(
        &db,
        &[
            "--title",
            "瞬时内部",
            "--start-at",
            "2026-11-20T10:30:00+08:00",
        ],
    );

    let items = list(
        &db,
        &[
            "--from",
            "2026-11-20T10:00:00+08:00",
            "--to",
            "2026-11-20T12:00:00+08:00",
        ],
    );
    let expected = sorted_ids(&[starts_at_from, inside, instant_inside]);
    assert_eq!(sorted_ids(&items), expected);
    assert!(!ids(&items).contains(&id_of(&starts_at_to)));
    assert!(!ids(&items).contains(&id_of(&ends_at_from)));
    assert!(!ids(&items).contains(&id_of(&instant_at_from)));
}

/// SCH-014：时区语义 —— 保存并返回用户原始时间串，范围/排序按时间点解释。
#[test]
fn sch_014_timezone_semantics_preserved() {
    let db = TestDb::new();
    // 02:00Z == 10:00+08:00。
    let utc_event = create(
        &db,
        &[
            "--title",
            "UTC 表示",
            "--start-at",
            "2026-11-20T02:00:00+00:00",
        ],
    );
    assert_eq!(utc_event["start_at"], "2026-11-20T02:00:00+00:00");

    let local_event = create(
        &db,
        &[
            "--title",
            "本地表示",
            "--start-at",
            "2026-11-20T11:00:00+08:00",
        ],
    );

    // 用 +08:00 的日期区间能查到 UTC 表示的事件（按时间点比较）。
    let items = list(&db, &["--from", DAY_FROM, "--to", DAY_TO]);
    let expected = sorted_ids(&[utc_event, local_event]);
    assert_eq!(sorted_ids(&items), expected);
}

// ---------------------------------------------------------------------------
// 补充：清除字段、排序、JSON 输入、校验、用户隔离
// ---------------------------------------------------------------------------

/// 补充：update 显式清除 description / end_at / location。
#[test]
fn sch_015_update_with_clear_flags() {
    let db = TestDb::new();
    let created = create(
        &db,
        &[
            "--title",
            "清除字段",
            "--description",
            "描述",
            "--start-at",
            "2026-11-20T10:00:00+08:00",
            "--end-at",
            "2026-11-20T11:00:00+08:00",
            "--location",
            "地点",
            "--all-day",
            "1",
        ],
    );

    let cleared = db.ok_json(&[
        "schedule",
        "update",
        "--id",
        id_of(&created).as_str(),
        "--clear-description",
        "--clear-end-at",
        "--clear-location",
    ]);
    assert!(cleared["description"].is_null());
    assert!(cleared["end_at"].is_null());
    assert!(cleared["location"].is_null());
    assert_eq!(cleared["all_day"], true);
    assert_eq!(cleared["title"], "清除字段");
}

/// 补充：列表按 start_at 的时间点 ASC、id ASC 排序（跨时区正确）。
#[test]
fn sch_016_list_sorted_by_start_instant() {
    let db = TestDb::new();
    // 故意乱序创建，且混用不同时区表示。
    let latest = create(
        &db,
        &["--title", "最晚", "--start-at", "2026-11-20T10:00:00+08:00"],
    );
    let earliest = create(
        &db,
        &["--title", "最早", "--start-at", "2026-11-20T01:30:00+00:00"],
    );
    let middle = create(
        &db,
        &["--title", "中间", "--start-at", "2026-11-20T09:45:00+08:00"],
    );

    let items = list(&db, &[]);
    assert_eq!(
        ids(&items),
        vec![id_of(&earliest), id_of(&middle), id_of(&latest)]
    );
    assert_event_order(&items);
}

/// 补充：`--input-json -` 创建/修改日程。
#[test]
fn sch_017_create_from_input_json() {
    let db = TestDb::new();
    let output = db.raw_json_with_stdin(
        &["schedule", "create", "--input-json", "-"],
        r#"{"title": "JSON 日程", "start_at": "2026-11-20T10:00:00+08:00", "end_at": "2026-11-20T11:00:00+08:00", "location": "线上", "all_day": 0}"#,
    );
    assert!(
        output.status.success(),
        "expected success, stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value = parse_json(&output);
    assert_eq!(value["data"]["title"], "JSON 日程");
    assert_eq!(value["data"]["end_at"], "2026-11-20T11:00:00+08:00");
    assert_eq!(value["data"]["location"], "线上");
    assert_eq!(value["data"]["all_day"], false);

    let id = id_of(&value["data"]);
    let update_output = db.raw_json_with_stdin(
        &["schedule", "update", "--input-json", "-"],
        &format!(r#"{{"id": "{id}", "title": "JSON 更新后的日程"}}"#),
    );
    assert!(update_output.status.success());
    assert_eq!(
        parse_json(&update_output)["data"]["title"],
        "JSON 更新后的日程"
    );
}

/// 补充：修改不存在的日程 / 修改参数非法 → 明确错误，且不产生副作用。
#[test]
fn sch_018_update_not_found_and_validation() {
    let db = TestDb::new();
    let (error, _) = db.fail_json(&[
        "schedule",
        "update",
        "--id",
        "does-not-exist",
        "--title",
        "x",
    ]);
    assert_error_code(&error, "SCHEDULE_NOT_FOUND");

    let created = create(
        &db,
        &["--title", "校验", "--start-at", "2026-11-20T10:00:00+08:00"],
    );
    let id = id_of(&created);
    let (error, _) = db.fail_json(&[
        "schedule",
        "update",
        "--id",
        id.as_str(),
        "--start-at",
        "not-a-time",
    ]);
    assert_error_code(&error, "INVALID_ARGUMENT");
    let (error, _) = db.fail_json(&[
        "schedule",
        "update",
        "--id",
        id.as_str(),
        "--end-at",
        "2026-11-20",
    ]);
    assert_error_code(&error, "INVALID_ARGUMENT");

    // 失败的修改不得改变原记录。
    let fetched = db.ok_json(&["schedule", "get", "--id", id.as_str()]);
    assert_eq!(fetched["title"], "校验");
    assert_eq!(fetched["start_at"], "2026-11-20T10:00:00+08:00");
}

/// 补充：创建时 end_at 非法 → INVALID_ARGUMENT。
#[test]
fn sch_019_invalid_end_at_rejected() {
    let db = TestDb::new();
    let (error, _) = db.fail_json(&[
        "schedule",
        "create",
        "--title",
        "非法结束时间",
        "--start-at",
        "2026-11-20T10:00:00+08:00",
        "--end-at",
        "2026-11-20T11:00:00",
    ]);
    assert_error_code(&error, "INVALID_ARGUMENT");
    assert!(list(&db, &[]).is_empty());
}

/// 补充：关键词查询只匹配标题（ASCII 大小写不敏感），不匹配描述。
#[test]
fn sch_020_list_keyword_filter() {
    let db = TestDb::new();
    create(
        &db,
        &["--title", "组会", "--start-at", "2026-11-20T10:00:00+08:00"],
    );
    create(
        &db,
        &[
            "--title",
            "数学课",
            "--description",
            "组会相关材料",
            "--start-at",
            "2026-11-20T14:00:00+08:00",
        ],
    );
    create(
        &db,
        &[
            "--title",
            "Group Meeting",
            "--start-at",
            "2026-11-20T16:00:00+08:00",
        ],
    );

    assert_eq!(list(&db, &["--keyword", "组会"]).len(), 1);
    assert_eq!(list(&db, &["--keyword", "数学"]).len(), 1);
    assert_eq!(list(&db, &["--keyword", "group"]).len(), 1);
    assert!(list(&db, &["--keyword", "不存在的关键词"]).is_empty());
}

/// 补充：空数据库列表是成功，返回空数组。
#[test]
fn sch_021_empty_list() {
    let db = TestDb::new();
    let data = db.ok_json(&["schedule", "list"]);
    assert_eq!(data["count"], 0);
    assert_eq!(data["items"], serde_json::json!([]));
}

/// 补充：只修改开始时间时，与原有 end_at 的时间一致性必须重新校验。
#[test]
fn sch_022_update_time_consistency() {
    let db = TestDb::new();
    let created = create(
        &db,
        &[
            "--title",
            "时间一致性",
            "--start-at",
            "2026-11-20T10:00:00+08:00",
            "--end-at",
            "2026-11-20T11:00:00+08:00",
        ],
    );
    let id = id_of(&created);

    // 把 start_at 移到 end_at 之后 → 拒绝。
    let (error, _) = db.fail_json(&[
        "schedule",
        "update",
        "--id",
        id.as_str(),
        "--start-at",
        "2026-11-20T12:00:00+08:00",
    ]);
    assert_error_code(&error, "INVALID_ARGUMENT");
    let fetched = db.ok_json(&["schedule", "get", "--id", id.as_str()]);
    assert_eq!(fetched["start_at"], "2026-11-20T10:00:00+08:00");

    // 合法前移 → 成功。
    let updated = db.ok_json(&[
        "schedule",
        "update",
        "--id",
        id.as_str(),
        "--start-at",
        "2026-11-20T09:00:00+08:00",
    ]);
    assert_eq!(updated["start_at"], "2026-11-20T09:00:00+08:00");
    assert_eq!(updated["end_at"], "2026-11-20T11:00:00+08:00");
}

/// 补充：用户隔离 —— 其他用户看不到、改不到、删不到当前用户的日程。
#[test]
fn sch_023_user_isolation() {
    let db = TestDb::new();
    let event = create(
        &db,
        &[
            "--title",
            "属于用户 A 的日程",
            "--start-at",
            "2026-11-20T10:00:00+08:00",
        ],
    );
    let event_id = id_of(&event);
    let other_user = db.add_user("wechat", "schedule-user-b");

    assert_eq!(
        db.ok_json_as(&other_user, &["schedule", "list"])["count"],
        0
    );

    for args in [
        vec!["schedule", "get", "--id", event_id.as_str()],
        vec![
            "schedule",
            "update",
            "--id",
            event_id.as_str(),
            "--title",
            "x",
        ],
        vec!["schedule", "delete", "--id", event_id.as_str()],
    ] {
        let (error, exit_code) = db.fail_json_as(&other_user, &args);
        assert_error_code(&error, "SCHEDULE_NOT_FOUND");
        assert_eq!(exit_code, 1, "args={args:?}");
    }

    let fetched = db.ok_json(&["schedule", "get", "--id", event_id.as_str()]);
    assert_eq!(fetched["title"], "属于用户 A 的日程");
}

/// 补充：items 类型隔离 —— schedule 命令组只操作 type=event，待办 ID 对其是“不存在”。
#[test]
fn sch_024_schedule_commands_ignore_tasks() {
    let db = TestDb::new();
    let task = db.ok_json(&["task", "create", "--title", "普通待办"]);
    let task_id = id_of(&task);
    assert_eq!(task["type"], "task");

    // task 不出现在 schedule 列表中。
    assert!(list(&db, &[]).is_empty());

    for args in [
        vec!["schedule", "get", "--id", task_id.as_str()],
        vec![
            "schedule",
            "update",
            "--id",
            task_id.as_str(),
            "--title",
            "x",
        ],
        vec!["schedule", "delete", "--id", task_id.as_str()],
    ] {
        let (error, _) = db.fail_json(&args);
        assert_error_code(&error, "SCHEDULE_NOT_FOUND");
    }

    // task 未受影响。
    let fetched = db.ok_json(&["task", "get", "--id", task_id.as_str()]);
    assert_eq!(fetched["title"], "普通待办");
}

/// 补充：缺少 --user-id 时返回 INVALID_ARGUMENT（退出码 2）。
#[test]
fn sch_025_missing_user_id_is_rejected() {
    let db = TestDb::new();
    let output = db.raw_json_without_user(&["schedule", "list"]);
    assert_eq!(output.status.code(), Some(2));
    let value = parse_json(&output);
    assert_eq!(value["success"], false);
    assert_error_code(&value, "INVALID_ARGUMENT");
}

/// 补充：all_day 只允许 0/1，其他值 → INVALID_ARGUMENT。
#[test]
fn sch_026_invalid_all_day_rejected() {
    let db = TestDb::new();
    let (error, exit_code) = db.fail_json(&[
        "schedule",
        "create",
        "--title",
        "非法全天标记",
        "--start-at",
        "2026-11-20T10:00:00+08:00",
        "--all-day",
        "2",
    ]);
    assert_error_code(&error, "INVALID_ARGUMENT");
    assert_eq!(exit_code, 2);

    let created = create(
        &db,
        &[
            "--title",
            "合法日程",
            "--start-at",
            "2026-11-20T10:00:00+08:00",
        ],
    );
    let (error, _) = db.fail_json(&[
        "schedule",
        "update",
        "--id",
        id_of(&created).as_str(),
        "--all-day",
        "-1",
    ]);
    assert_error_code(&error, "INVALID_ARGUMENT");
    assert!(list(&db, &[]).len() == 1);
}
