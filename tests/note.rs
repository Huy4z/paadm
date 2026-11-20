//! Note（笔记）领域集成测试。
//!
//! 表结构以 `docs/design/paadm数据表结构设计.md` 为唯一权威来源：
//! `notes(id, user_id, title, content, created_at, updated_at)`，
//! 第一版没有分类 / 标签 / 关联关系。
//!
//! 通过真实 `paadm` 二进制 + 每个测试独立的临时数据库驱动（`tests/common/mod.rs`）：
//! `TestDb` 会自动创建默认测试用户并给所有命令加 `--user-id`。
//! 覆盖设计文档第 7 节 NOTE-001..NOTE-008，并补充用户隔离、排序稳定性、
//! created_at 范围过滤、--input-json 等协作约定。

mod common;

use common::{TestDb, assert_error_code, id_of};
use serde_json::{Value, json};

// ---------------------------------------------------------------------------
// 辅助函数
// ---------------------------------------------------------------------------

/// 只带正文创建一条笔记。
fn create(db: &TestDb, content: &str) -> Value {
    db.ok_json(&["note", "create", "--content", content])
}

/// 带标题和正文创建一条笔记。
fn create_titled(db: &TestDb, title: &str, content: &str) -> Value {
    db.ok_json(&["note", "create", "--title", title, "--content", content])
}

/// 按 ID 查询一条笔记（断言成功）。
fn get(db: &TestDb, id: &str) -> Value {
    db.ok_json(&["note", "get", "--id", id])
}

/// 查询全部笔记（断言成功）。
fn list(db: &TestDb) -> Vec<Value> {
    db.ok_items(&["note", "list"])
}

/// 带额外过滤参数查询笔记列表。
fn list_with(db: &TestDb, extra: &[&str]) -> Vec<Value> {
    let mut args = vec!["note", "list"];
    args.extend_from_slice(extra);
    db.ok_items(&args)
}

/// 以指定用户身份查询笔记列表（`tests/common` 没有 ok_items_as，这里手动断言）。
fn list_as(db: &TestDb, user_id: &str, extra: &[&str]) -> Vec<Value> {
    let mut args = vec!["note", "list"];
    args.extend_from_slice(extra);
    let data = db.ok_json_as(user_id, &args);
    let items = data["items"].as_array().expect("items must be an array");
    assert_eq!(data["count"], items.len());
    items.clone()
}

/// 提取 id 列表，便于断言顺序。
fn ids(items: &[Value]) -> Vec<String> {
    items.iter().map(id_of).collect()
}

// ---------------------------------------------------------------------------
// NOTE-001 创建笔记
// ---------------------------------------------------------------------------

/// NOTE-001：只提供正文即可创建笔记；title 缺省为 null，记录归属当前用户。
#[test]
fn note_001_create_with_content_only() {
    let db = TestDb::new();
    let data = create(&db, "第一条笔记");

    let id = id_of(&data);
    assert!(!id.is_empty(), "id must not be empty: {data}");
    assert_eq!(data["content"], "第一条笔记");
    assert_eq!(data["title"], Value::Null);
    assert_eq!(data["user_id"], db.user_id.as_str());
    assert!(
        data["created_at"].as_str().is_some_and(|s| !s.is_empty()),
        "created_at must be a non-empty string: {data}"
    );
    assert!(
        data["updated_at"].as_str().is_some_and(|s| !s.is_empty()),
        "updated_at must be a non-empty string: {data}"
    );
    // 新建记录时 created_at 与 updated_at 相同。
    assert_eq!(data["created_at"], data["updated_at"]);

    // 每个命令都是独立子进程：重新启动后仍能按 id 查询到，验证持久化。
    let stored = get(&db, &id);
    assert_eq!(stored["id"], data["id"]);
    assert_eq!(stored["content"], "第一条笔记");
}

/// NOTE-001（补充）：title 可选；title / content 保存 trim 后的内容。
#[test]
fn note_001_create_with_title() {
    let db = TestDb::new();
    let data = create_titled(&db, "  购物清单  ", "  牛奶; 面包\n鸡蛋  ");

    assert_eq!(data["title"], "购物清单");
    assert_eq!(data["content"], "牛奶; 面包\n鸡蛋");

    let stored = get(&db, &id_of(&data));
    assert_eq!(stored["title"], "购物清单");
}

// ---------------------------------------------------------------------------
// NOTE-002 空内容校验
// ---------------------------------------------------------------------------

/// NOTE-002：正文为空（空字符串或纯空白）被拒绝，错误码 INVALID_ARGUMENT、退出码 2。
#[test]
fn note_002_empty_content_rejected() {
    let db = TestDb::new();

    let (error, code) = db.fail_json(&["note", "create", "--content", ""]);
    assert_error_code(&error, "INVALID_ARGUMENT");
    assert_eq!(code, 2);

    let (error, code) = db.fail_json(&["note", "create", "--content", "   \n\t "]);
    assert_error_code(&error, "INVALID_ARGUMENT");
    assert_eq!(code, 2);

    // 校验失败不写入任何数据。
    assert!(list(&db).is_empty());
}

/// NOTE-002（补充）：完全不提供 content 时参数校验失败，且不写入数据。
#[test]
fn note_002_missing_content_rejected() {
    let db = TestDb::new();
    let (error, code) = db.fail_json(&["note", "create", "--title", "只有标题"]);
    assert_error_code(&error, "INVALID_ARGUMENT");
    assert_eq!(code, 2);
    assert!(list(&db).is_empty());
}

// ---------------------------------------------------------------------------
// NOTE-003 长文本
// ---------------------------------------------------------------------------

/// NOTE-003：长文本（5200 字符）完整保存，不截断。
///
/// 通过 `--input-json -`（stdin JSON）传入，同时验证长文本输入路径。
#[test]
fn note_003_long_text_saved_in_full() {
    let db = TestDb::new();
    let content: String = "长文本内容用于验证不截断。".repeat(400);
    assert_eq!(content.chars().count(), 5200);

    let input = json!({ "title": "长文本", "content": content }).to_string();
    let args = ["note", "create", "--input-json", "-"];
    let output = db.raw_json_with_stdin(&args, &input);
    common::assert_success(&output, &args);
    let response = common::parse_json(&output);
    assert_eq!(response["success"], true, "unexpected response: {response}");

    let id = id_of(&response["data"]);
    let stored = get(&db, &id);
    let stored_content = stored["content"]
        .as_str()
        .expect("content must be a string");
    assert_eq!(
        stored_content.chars().count(),
        5200,
        "long content was truncated"
    );
    assert_eq!(
        stored_content, content,
        "long content changed during round-trip"
    );
    assert_eq!(stored["title"], "长文本");
}

// ---------------------------------------------------------------------------
// NOTE-004 中文与特殊字符
// ---------------------------------------------------------------------------

/// NOTE-004：中文、双引号、单引号、分号、换行、制表符、emoji、反斜杠完整保存。
#[test]
fn note_004_chinese_and_special_characters_preserved() {
    let db = TestDb::new();
    let content = "中文：引号\"双引号\"与'单引号'；分号；换行\n第二行\t制表符 🚀 \\反斜杠";
    let data = create(&db, content);

    assert_eq!(data["content"], content);
    let stored = get(&db, &id_of(&data));
    assert_eq!(stored["content"], content);
}

// ---------------------------------------------------------------------------
// NOTE-005 关键词查询
// ---------------------------------------------------------------------------

/// NOTE-005：关键词按 title 或 content 的包含关系匹配，返回记录保持创建顺序。
#[test]
fn note_005_keyword_matches_title_or_content() {
    let db = TestDb::new();
    let title_hit = create_titled(&db, "周会纪要", "讨论预算");
    let content_hit = create(&db, "记得参加周会并记录");
    let miss = create(&db, "买牛奶");

    let hits = list_with(&db, &["--keyword", "周会"]);
    assert_eq!(
        ids(&hits),
        vec![id_of(&title_hit), id_of(&content_hit)],
        "keyword must match title or content, in created_at ASC order"
    );
    assert!(!hits.iter().any(|note| note["id"] == miss["id"]));

    assert!(list_with(&db, &["--keyword", "不存在的关键词"]).is_empty());
}

/// NOTE-005（补充）：关键词中的 `%` / `_` 按字面匹配，不当作 LIKE 通配符。
#[test]
fn note_005_keyword_wildcards_are_literal() {
    let db = TestDb::new();
    let percent = create(&db, "完成度 100% 的总结");
    let underscore = create(&db, "变量名 user_name 的说明");
    let plain = create(&db, "完成度 1000 的总结");

    let hits = list_with(&db, &["--keyword", "100%"]);
    assert_eq!(ids(&hits), vec![id_of(&percent)]);
    assert!(!hits.iter().any(|note| note["id"] == plain["id"]));

    let hits = list_with(&db, &["--keyword", "user_name"]);
    assert_eq!(ids(&hits), vec![id_of(&underscore)]);
}

/// NOTE-005（补充）：ASCII 关键词大小写不敏感（简单关键词检索）。
#[test]
fn note_005_keyword_is_case_insensitive_for_ascii() {
    let db = TestDb::new();
    let data = create(&db, "Rust ownership notes");
    let hits = list_with(&db, &["--keyword", "rust"]);
    assert_eq!(ids(&hits), vec![id_of(&data)]);
}

// ---------------------------------------------------------------------------
// NOTE-006 修改内容
// ---------------------------------------------------------------------------

/// NOTE-006：修改正文后新内容生效，并刷新 updated_at。
#[test]
fn note_006_update_content_takes_effect() {
    let db = TestDb::new();
    let created = create_titled(&db, "标题", "旧正文");
    let id = id_of(&created);

    let updated = db.ok_json(&["note", "update", "--id", &id, "--content", "新正文"]);
    assert_eq!(updated["id"], created["id"]);
    assert_eq!(updated["content"], "新正文");
    // 未提供的字段不覆盖。
    assert_eq!(updated["title"], "标题");
    // 更新时间不早于创建时间（RFC 3339 UTC 字符串可直接按字典序比较）。
    assert!(
        updated["updated_at"].as_str() >= created["updated_at"].as_str(),
        "updated_at must be refreshed: created={} updated={}",
        created["updated_at"],
        updated["updated_at"]
    );

    let stored = get(&db, &id);
    assert_eq!(stored["content"], "新正文");
}

/// NOTE-006（补充）：update 只修改提供的字段；--clear-title 显式清空。
#[test]
fn note_006_update_only_provided_fields_and_clear_title() {
    let db = TestDb::new();
    let created = create_titled(&db, "原标题", "原正文");
    let id = id_of(&created);

    // 只改 content：title 不变。
    let updated = db.ok_json(&["note", "update", "--id", &id, "--content", "新正文"]);
    assert_eq!(updated["title"], "原标题");
    assert_eq!(updated["content"], "新正文");

    // 只提供 id（没有业务字段）：不覆盖任何字段，仍然成功。
    let untouched = db.ok_json(&["note", "update", "--id", &id]);
    assert_eq!(untouched["title"], "原标题");
    assert_eq!(untouched["content"], "新正文");

    // 先改标题，再用 --clear-title 显式清空。
    let retitled = db.ok_json(&["note", "update", "--id", &id, "--title", " 新标题 "]);
    assert_eq!(retitled["title"], "新标题");
    let cleared = db.ok_json(&["note", "update", "--id", &id, "--clear-title"]);
    assert_eq!(cleared["title"], Value::Null);
    assert_eq!(cleared["content"], "新正文");

    let stored = get(&db, &id);
    assert_eq!(stored["title"], Value::Null);
}

/// NOTE-006（补充）：更新时正文为空被拒绝，原内容保持不变。
#[test]
fn note_006_update_rejects_empty_content() {
    let db = TestDb::new();
    let created = create(&db, "正文");
    let id = id_of(&created);

    let (error, code) = db.fail_json(&["note", "update", "--id", &id, "--content", "  "]);
    assert_error_code(&error, "INVALID_ARGUMENT");
    assert_eq!(code, 2);

    let stored = get(&db, &id);
    assert_eq!(stored["content"], "正文");
}

/// NOTE-006（补充）：update 支持通过 `--input-json -` 从 stdin 读取完整参数。
#[test]
fn note_006_update_via_input_json() {
    let db = TestDb::new();
    let created = create(&db, "旧正文");

    let input = json!({
        "id": id_of(&created),
        "content": "从 stdin 更新的正文",
        "title": "新标题",
    })
    .to_string();
    let args = ["note", "update", "--input-json", "-"];
    let output = db.raw_json_with_stdin(&args, &input);
    common::assert_success(&output, &args);
    let response = common::parse_json(&output);
    assert_eq!(response["data"]["content"], "从 stdin 更新的正文");
    assert_eq!(response["data"]["title"], "新标题");
}

// ---------------------------------------------------------------------------
// NOTE-007 删除笔记
// ---------------------------------------------------------------------------

/// NOTE-007：删除笔记成功；删除后按 ID 查询返回 NOTE_NOT_FOUND，列表不再包含。
#[test]
fn note_007_delete_record() {
    let db = TestDb::new();
    let deleted_note = create(&db, "要删除的笔记");
    let kept = create(&db, "保留的笔记");
    let id = id_of(&deleted_note);

    let data = db.ok_json(&["note", "delete", "--id", &id]);
    assert_eq!(data["id"], id);
    assert_eq!(data["deleted"], true);

    let (error, code) = db.fail_json(&["note", "get", "--id", &id]);
    assert_error_code(&error, "NOTE_NOT_FOUND");
    assert_eq!(code, 1);

    assert_eq!(ids(&list(&db)), vec![id_of(&kept)]);

    // 重复删除不存在的记录返回 NOTE_NOT_FOUND。
    let (error, code) = db.fail_json(&["note", "delete", "--id", &id]);
    assert_error_code(&error, "NOTE_NOT_FOUND");
    assert_eq!(code, 1);
}

// ---------------------------------------------------------------------------
// NOTE-008 查询不存在记录
// ---------------------------------------------------------------------------

/// NOTE-008：查询不存在的记录返回 NOTE_NOT_FOUND（退出码 1，data 为 null）。
#[test]
fn note_008_get_missing_returns_note_not_found() {
    let db = TestDb::new();
    let fake = "00000000-0000-4000-8000-000000000000";

    let (error, code) = db.fail_json(&["note", "get", "--id", fake]);
    assert_error_code(&error, "NOTE_NOT_FOUND");
    assert_eq!(code, 1);
    assert_eq!(error["data"], Value::Null);

    let (error, code) = db.fail_json(&["note", "update", "--id", fake, "--content", "x"]);
    assert_error_code(&error, "NOTE_NOT_FOUND");
    assert_eq!(code, 1);

    let (error, code) = db.fail_json(&["note", "delete", "--id", fake]);
    assert_error_code(&error, "NOTE_NOT_FOUND");
    assert_eq!(code, 1);
}

/// NOTE-008（补充）：空 ID 属于参数错误 INVALID_ARGUMENT（退出码 2）。
#[test]
fn note_008_empty_id_is_invalid_argument() {
    let db = TestDb::new();
    let cases: [&[&str]; 3] = [
        &["note", "get", "--id", ""],
        &["note", "update", "--id", "", "--content", "x"],
        &["note", "delete", "--id", ""],
    ];
    for args in cases {
        let (error, code) = db.fail_json(args);
        assert_error_code(&error, "INVALID_ARGUMENT");
        assert_eq!(code, 2, "unexpected exit code for args={args:?}");
    }
}

// ---------------------------------------------------------------------------
// 列表排序 / 范围过滤 / 空列表
// ---------------------------------------------------------------------------

/// 补充：列表始终按 created_at ASC、id ASC 排序；关键词过滤后顺序保持不变。
#[test]
fn note_list_order_is_stable() {
    let db = TestDb::new();
    let first = create(&db, "共同的 关键词 一");
    let second = create(&db, "共同的 关键词 二");
    let third = create(&db, "共同的 关键词 三");

    let expected = vec![id_of(&first), id_of(&second), id_of(&third)];
    assert_eq!(ids(&list(&db)), expected);
    assert_eq!(ids(&list_with(&db, &["--keyword", "共同的"])), expected);
}

/// 补充：列表按 created_at 闭区间过滤（--from / --to，julianday 比较）。
#[test]
fn note_list_by_created_at_range() {
    let db = TestDb::new();
    let note = create(&db, "时间范围");
    let created_at = note["created_at"]
        .as_str()
        .expect("created_at string")
        .to_string();
    let id = id_of(&note);

    // 用记录自身的 created_at 作为闭区间两端，应恰好命中该记录。
    assert_eq!(
        ids(&list_with(
            &db,
            &["--from", &created_at, "--to", &created_at]
        )),
        vec![id.clone()]
    );
    // 起点晚于记录时间 -> 空；终点早于记录时间 -> 空。
    assert!(list_with(&db, &["--from", "2999-01-01T00:00:00+00:00"]).is_empty());
    assert!(list_with(&db, &["--to", "2000-01-01T00:00:00+00:00"]).is_empty());
    // 宽范围包含记录（跨时区写法也应正确）。
    assert_eq!(
        ids(&list_with(
            &db,
            &[
                "--from",
                "2000-01-01T00:00:00+08:00",
                "--to",
                "2999-01-01T00:00:00-05:00",
            ]
        )),
        vec![id]
    );
}

/// 补充：非法时间过滤参数返回 INVALID_ARGUMENT。
#[test]
fn note_list_rejects_invalid_time_filters() {
    let db = TestDb::new();

    // 缺少时区偏移。
    let (error, code) = db.fail_json(&["note", "list", "--from", "2026-11-17"]);
    assert_error_code(&error, "INVALID_ARGUMENT");
    assert_eq!(code, 2);

    // from 晚于 to。
    let (error, code) = db.fail_json(&[
        "note",
        "list",
        "--from",
        "2026-11-18T00:00:00+08:00",
        "--to",
        "2026-11-17T00:00:00+08:00",
    ]);
    assert_error_code(&error, "INVALID_ARGUMENT");
    assert_eq!(code, 2);
}

/// 补充：空列表是成功（success=true、items=[]、count=0）。
#[test]
fn note_list_empty_is_success() {
    let db = TestDb::new();
    let data = db.ok_json(&["note", "list"]);
    assert_eq!(data["count"], 0);
    assert_eq!(data["items"], json!([]));
}

// ---------------------------------------------------------------------------
// JSON 结构 / --input-json 输入
// ---------------------------------------------------------------------------

/// 补充：create/get 的 data 直接是记录对象，字段名稳定，且不再有 tags 字段。
#[test]
fn note_json_shape_is_stable() {
    let db = TestDb::new();
    let created = create_titled(&db, "标题", "正文");
    for value in [created.clone(), get(&db, &id_of(&created))] {
        for field in [
            "id",
            "user_id",
            "title",
            "content",
            "created_at",
            "updated_at",
        ] {
            assert!(
                value.get(field).is_some(),
                "missing field `{field}` in {value}"
            );
        }
        // 第一版已移除标签/分类等字段。
        assert!(
            value.get("tags").is_none(),
            "notes must not expose a tags field: {value}"
        );
        assert_eq!(value["id"], created["id"]);
        assert_eq!(value["user_id"], db.user_id.as_str());
        assert_eq!(value["content"], "正文");
    }
}

/// 补充：`--input-json` 缺少 content 时返回 INVALID_ARGUMENT，且不写库。
#[test]
fn note_create_input_json_missing_content_rejected() {
    let db = TestDb::new();
    let input = json!({ "title": "只有标题" }).to_string();
    let args = ["note", "create", "--input-json", "-"];
    let output = db.raw_json_with_stdin(&args, &input);
    assert_ne!(
        output.status.code(),
        Some(0),
        "expected failure: {output:?}"
    );
    let response = common::parse_json(&output);
    assert_error_code(&response, "INVALID_ARGUMENT");
    assert!(list(&db).is_empty());
}

/// 补充：非法 JSON 输入返回 INVALID_INPUT（退出码 2），且不写库。
#[test]
fn note_create_input_json_invalid_json_rejected() {
    let db = TestDb::new();
    let args = ["note", "create", "--input-json", "-"];
    let output = db.raw_json_with_stdin(&args, "{not valid json");
    let exit_code = output.status.code().unwrap_or(-1);
    assert_eq!(exit_code, 2, "unexpected exit code: {output:?}");
    let response = common::parse_json(&output);
    assert_error_code(&response, "INVALID_INPUT");
    assert!(list(&db).is_empty());
}

// ---------------------------------------------------------------------------
// 用户隔离（数据表结构设计 4.1 节）
// ---------------------------------------------------------------------------

/// 补充：笔记按 user_id 隔离——看不到、查不到、改不到、删不到其他用户的笔记。
#[test]
fn note_user_isolation() {
    let db = TestDb::new();
    let other_user = db.add_user("qq", "other-user");
    assert_ne!(other_user, db.user_id);

    // 默认用户创建一条笔记。
    let mine = create(&db, "只有我能看到的笔记");
    let id = id_of(&mine);

    // 其他用户列表为空，关键词也无法命中。
    assert!(list_as(&db, &other_user, &[]).is_empty());
    assert!(list_as(&db, &other_user, &["--keyword", "只有我"]).is_empty());

    // 其他用户按 ID 查询 / 修改 / 删除都表现为“不存在”。
    for args in [
        vec!["note", "get", "--id", id.as_str()],
        vec!["note", "update", "--id", id.as_str(), "--content", "篡改"],
        vec!["note", "delete", "--id", id.as_str()],
    ] {
        let (error, code) = db.fail_json_as(&other_user, &args);
        assert_error_code(&error, "NOTE_NOT_FOUND");
        assert_eq!(code, 1, "unexpected exit code for args={args:?}");
    }

    // 原用户的笔记没有被修改或删除。
    let stored = get(&db, &id);
    assert_eq!(stored["content"], "只有我能看到的笔记");

    // 其他用户创建自己的笔记后，两个用户各自只看得到自己的。
    let theirs = db.ok_json_as(&other_user, &["note", "create", "--content", "别人的笔记"]);
    assert_eq!(ids(&list(&db)), vec![id]);
    assert_eq!(ids(&list_as(&db, &other_user, &[])), vec![id_of(&theirs)]);
}

/// 补充：业务命令缺少 --user-id 返回 INVALID_ARGUMENT；未知用户返回 USER_NOT_FOUND。
#[test]
fn note_requires_user_id() {
    let db = TestDb::new();

    let output = db.raw_json_without_user(&["note", "list"]);
    let exit_code = output.status.code().unwrap_or(-1);
    assert_eq!(exit_code, 2, "unexpected output: {output:?}");
    let response = common::parse_json(&output);
    assert_error_code(&response, "INVALID_ARGUMENT");

    let unknown_user = "11111111-1111-4111-8111-111111111111";
    let (error, code) = db.fail_json_as(unknown_user, &["note", "list"]);
    assert_error_code(&error, "USER_NOT_FOUND");
    assert_eq!(code, 1);
}
