//! Transaction（收支记录）领域集成测试：TX-001 ~ TX-014 + 边界用例。
//!
//! 权威表结构见 `docs/design/paadm数据表结构设计.md`：
//! `transactions(id, user_id, type, amount, category, description, occurred_at, created_at, updated_at)`；
//! 金额 `amount` 为整数人民币分（¥18.50 → 1850），第一版没有币种字段。
//!
//! 全部用例通过真实 `paadm` 子进程执行，使用独立临时 SQLite 数据库
//! （`tests/common/mod.rs` 的 `TestDb`，会自动创建测试用户并带上 `--user-id`）。

mod common;

use serde_json::Value;

use common::{TestDb, assert_error_code, id_of, parse_json};

/// 固定的测试时间（带时区偏移）。
const TS_1: &str = "2026-11-17T12:00:00+08:00";
const TS_2: &str = "2026-11-17T13:00:00+08:00";
const TS_3: &str = "2026-11-18T09:00:00+08:00";

// ---------------------------------------------------------------------------
// 辅助函数
// ---------------------------------------------------------------------------

fn as_str_args(args: &[String]) -> Vec<&str> {
    args.iter().map(String::as_str).collect()
}

fn run_ok_json(db: &TestDb, args: &[String]) -> Value {
    db.ok_json(&as_str_args(args))
}

fn run_ok_items(db: &TestDb, args: &[String]) -> Vec<Value> {
    db.ok_items(&as_str_args(args))
}

fn run_fail_json(db: &TestDb, args: &[String]) -> (Value, i32) {
    db.fail_json(&as_str_args(args))
}

fn run_fail_with_stdin(db: &TestDb, args: &[String], stdin: &str) -> (Value, i32) {
    let output = db.raw_json_with_stdin(&as_str_args(args), stdin);
    let code = output.status.code().unwrap_or(-1);
    assert_ne!(code, 0, "expected failure, stdin={stdin}, args={args:?}");
    let value = parse_json(&output);
    assert_eq!(
        value["success"],
        Value::Bool(false),
        "expected success=false, got {value}"
    );
    assert!(
        value["error"]["code"].is_string(),
        "expected error.code string, got {value}"
    );
    (value, code)
}

fn create_cmd(r#type: &str, amount: i64, occurred_at: &str) -> Vec<String> {
    vec![
        "transaction".to_string(),
        "create".to_string(),
        "--type".to_string(),
        r#type.to_string(),
        "--amount".to_string(),
        amount.to_string(),
        "--occurred-at".to_string(),
        occurred_at.to_string(),
    ]
}

fn get_cmd(id: &str) -> Vec<String> {
    vec![
        "transaction".to_string(),
        "get".to_string(),
        "--id".to_string(),
        id.to_string(),
    ]
}

fn delete_cmd(id: &str) -> Vec<String> {
    vec![
        "transaction".to_string(),
        "delete".to_string(),
        "--id".to_string(),
        id.to_string(),
    ]
}

fn update_cmd(id: &str) -> Vec<String> {
    vec![
        "transaction".to_string(),
        "update".to_string(),
        "--id".to_string(),
        id.to_string(),
    ]
}

fn list_cmd(filters: &[&str]) -> Vec<String> {
    let mut args = vec!["transaction".to_string(), "list".to_string()];
    args.extend(filters.iter().map(|s| s.to_string()));
    args
}

fn summary_cmd(filters: &[&str]) -> Vec<String> {
    let mut args = vec!["transaction".to_string(), "summary".to_string()];
    args.extend(filters.iter().map(|s| s.to_string()));
    args
}

/// 创建一条记录并附加额外的 CLI 参数（如 `--category food`）。
fn create_with(db: &TestDb, r#type: &str, amount: i64, occurred_at: &str, extra: &[&str]) -> Value {
    let mut args = create_cmd(r#type, amount, occurred_at);
    args.extend(extra.iter().map(|s| s.to_string()));
    run_ok_json(db, &args)
}

/// 从汇总的 `by_category` 中取指定类别的行。
fn category_row<'a>(summary: &'a Value, category: &str) -> &'a Value {
    let rows = summary["by_category"]
        .as_array()
        .unwrap_or_else(|| panic!("by_category must be an array: {summary}"));
    rows.iter()
        .find(|row| row["category"] == category)
        .unwrap_or_else(|| panic!("missing category {category}: {summary}"))
}

fn amounts_of(items: &[Value]) -> Vec<i64> {
    items
        .iter()
        .map(|item| item["amount"].as_i64().expect("amount"))
        .collect()
}

// ---------------------------------------------------------------------------
// TX-001 创建支出
// ---------------------------------------------------------------------------

#[test]
fn tx_001_create_expense() {
    let db = TestDb::new();
    let data = create_with(
        &db,
        "expense",
        1850,
        TS_1,
        &["--category", "food", "--description", "午饭"],
    );

    assert_eq!(data["type"], "expense", "type 必须是 expense: {data}");
    assert_eq!(data["amount"], 1850, "金额为人民币分: {data}");
    assert_eq!(data["category"], "food");
    assert_eq!(data["description"], "午饭");
    assert_eq!(data["occurred_at"], TS_1);
    assert_eq!(data["user_id"], db.user_id, "记录必须归属当前用户");
    assert!(data["id"].as_str().is_some_and(|s| !s.is_empty()));
    assert!(data["created_at"].as_str().is_some_and(|s| !s.is_empty()));
    assert!(data["updated_at"].as_str().is_some_and(|s| !s.is_empty()));
    // 旧字段（最小货币单位 / 币种）已删除。
    assert!(
        data.get("amount_minor").is_none(),
        "不应再有 amount_minor: {data}"
    );
    assert!(data.get("currency").is_none(), "第一版没有币种字段: {data}");

    // 记录可通过 get 读回，且列表计数为 1。
    let id = id_of(&data);
    let fetched = run_ok_json(&db, &get_cmd(&id));
    assert_eq!(fetched["id"], id);
    assert_eq!(fetched["type"], "expense");
    assert_eq!(fetched["amount"], 1850);

    let items = run_ok_items(&db, &list_cmd(&[]));
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["id"], id);
}

// ---------------------------------------------------------------------------
// TX-002 创建收入
// ---------------------------------------------------------------------------

#[test]
fn tx_002_create_income() {
    let db = TestDb::new();
    let data = create_with(&db, "income", 500_000, TS_2, &["--category", "salary"]);

    assert_eq!(data["type"], "income", "type 必须是 income: {data}");
    assert_eq!(data["amount"], 500_000);
    assert_eq!(data["category"], "salary");

    let id = id_of(&data);
    let fetched = run_ok_json(&db, &get_cmd(&id));
    assert_eq!(fetched["type"], "income");
    assert_eq!(fetched["amount"], 500_000);
}

// ---------------------------------------------------------------------------
// TX-003 金额为零：规则固定为拒绝（amount 必须 > 0）
// ---------------------------------------------------------------------------

#[test]
fn tx_003_zero_amount_rejected() {
    let db = TestDb::new();
    let args = create_cmd("expense", 0, TS_1);
    let (value, code) = run_fail_json(&db, &args);
    assert_eq!(code, 2, "参数错误退出码应为 2");
    assert_error_code(&value, "INVALID_ARGUMENT");
    assert!(
        run_ok_items(&db, &list_cmd(&[])).is_empty(),
        "校验失败不得写入数据"
    );
}

// ---------------------------------------------------------------------------
// TX-004 负金额：校验失败，金额符号不与 type 混用
// ---------------------------------------------------------------------------

#[test]
fn tx_004_negative_amount_rejected() {
    let db = TestDb::new();
    for r#type in ["expense", "income"] {
        let args = create_cmd(r#type, -1650, TS_1);
        let (value, code) = run_fail_json(&db, &args);
        assert_eq!(code, 2, "type={type}");
        assert_error_code(&value, "INVALID_ARGUMENT");
    }
    assert!(run_ok_items(&db, &list_cmd(&[])).is_empty());
}

// ---------------------------------------------------------------------------
// TX-005 非法类型
// ---------------------------------------------------------------------------

#[test]
fn tx_005_invalid_type_rejected() {
    let db = TestDb::new();
    for invalid_type in ["transfer", "expenses", "", "INCOME", "Income"] {
        let args = create_cmd(invalid_type, 100, TS_1);
        let (value, code) = run_fail_json(&db, &args);
        assert_eq!(code, 2, "type={invalid_type:?}");
        assert_error_code(&value, "INVALID_ARGUMENT");
    }
    assert!(run_ok_items(&db, &list_cmd(&[])).is_empty());
}

// ---------------------------------------------------------------------------
// TX-006 缺少金额 / 发生时间 / 类型（参数错误）
// ---------------------------------------------------------------------------

#[test]
fn tx_006_missing_required_fields_rejected() {
    let db = TestDb::new();

    // 缺少 --amount：CLI 参数缺失，退出码 2、INVALID_ARGUMENT。
    let args = vec![
        "transaction".to_string(),
        "create".to_string(),
        "--type".to_string(),
        "expense".to_string(),
        "--occurred-at".to_string(),
        TS_1.to_string(),
    ];
    let (value, code) = run_fail_json(&db, &args);
    assert_eq!(code, 2);
    assert_error_code(&value, "INVALID_ARGUMENT");

    // 经 --input-json 提交时，由 Service 校验各必填字段缺失。
    let input_args = vec![
        "transaction".to_string(),
        "create".to_string(),
        "--input-json".to_string(),
        "-".to_string(),
    ];
    let cases = [
        // 缺少 amount
        r#"{"type":"expense","occurred_at":"2026-11-17T12:00:00+08:00"}"#,
        // 缺少 occurred_at
        r#"{"type":"expense","amount":100}"#,
        // 缺少 type
        r#"{"amount":100,"occurred_at":"2026-11-17T12:00:00+08:00"}"#,
        // 空对象
        r#"{}"#,
    ];
    for stdin in cases {
        let (value, code) = run_fail_with_stdin(&db, &input_args, stdin);
        assert_eq!(code, 2, "stdin={stdin}");
        assert_error_code(&value, "INVALID_ARGUMENT");
    }

    assert!(
        run_ok_items(&db, &list_cmd(&[])).is_empty(),
        "校验失败不得写入数据"
    );
}

// ---------------------------------------------------------------------------
// TX-007 查询时间范围：只统计区间内记录（闭区间 [from, to]，按 julianday 比较）
// ---------------------------------------------------------------------------

#[test]
fn tx_007_list_by_time_range() {
    let db = TestDb::new();
    create_with(&db, "expense", 100, "2026-10-31T23:59:59+08:00", &[]);
    create_with(&db, "expense", 200, "2026-11-01T00:00:00+08:00", &[]);
    create_with(&db, "expense", 300, "2026-11-15T12:00:00+08:00", &[]);
    create_with(&db, "expense", 400, "2026-11-30T23:59:59+08:00", &[]);
    create_with(&db, "expense", 500, "2026-12-01T00:00:00+08:00", &[]);

    let items = run_ok_items(
        &db,
        &list_cmd(&[
            "--from",
            "2026-11-01T00:00:00+08:00",
            "--to",
            "2026-11-30T23:59:59+08:00",
        ]),
    );
    assert_eq!(
        amounts_of(&items),
        vec![200, 300, 400],
        "只应返回 11 月内的记录，且按 julianday(occurred_at) ASC, id ASC 排序"
    );

    // 边界改用 UTC 偏移表达，结果必须一致（证明比较基于时间点而非字符串）。
    let items_utc = run_ok_items(
        &db,
        &list_cmd(&[
            "--from",
            "2026-10-31T16:00:00Z",
            "--to",
            "2026-11-30T15:59:59Z",
        ]),
    );
    let ids: Vec<&str> = items.iter().map(|i| i["id"].as_str().unwrap()).collect();
    let ids_utc: Vec<&str> = items_utc
        .iter()
        .map(|i| i["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ids_utc);

    // summary 使用同一时间范围时只统计这些记录。
    let summary = run_ok_json(
        &db,
        &summary_cmd(&[
            "--from",
            "2026-11-01T00:00:00+08:00",
            "--to",
            "2026-11-30T23:59:59+08:00",
        ]),
    );
    assert_eq!(summary["expense_total"], 900);
    assert_eq!(summary["income_total"], 0);

    // 区间外：空列表是成功而不是错误。
    let empty = run_ok_items(&db, &list_cmd(&["--from", "2027-01-01T00:00:00+08:00"]));
    assert!(empty.is_empty());
}

// ---------------------------------------------------------------------------
// TX-008 按类别统计/筛选：只统计对应类别和支出
// ---------------------------------------------------------------------------

#[test]
fn tx_008_list_and_summary_by_category() {
    let db = TestDb::new();
    create_with(&db, "expense", 1850, TS_1, &["--category", "food"]);
    create_with(&db, "expense", 2000, TS_2, &["--category", "food"]);
    create_with(&db, "expense", 300, TS_3, &["--category", "transport"]);
    create_with(&db, "income", 500_000, TS_3, &["--category", "salary"]);

    let items = run_ok_items(&db, &list_cmd(&["--category", "food"]));
    assert_eq!(items.len(), 2);
    assert!(
        items
            .iter()
            .all(|item| item["category"] == "food" && item["type"] == "expense")
    );

    let summary = run_ok_json(&db, &summary_cmd(&["--category", "food"]));
    assert_eq!(summary["income_total"], 0);
    assert_eq!(summary["expense_total"], 3850);
    assert_eq!(summary["net_total"], -3850);
    let rows = summary["by_category"].as_array().expect("by_category");
    assert_eq!(rows.len(), 1, "只应包含 food 一个类别: {summary}");
    let food = category_row(&summary, "food");
    assert_eq!(food["income"], 0);
    assert_eq!(food["expense"], 3850);

    // 按类型筛选：只有一条收入记录。
    let incomes = run_ok_items(&db, &list_cmd(&["--type", "income"]));
    assert_eq!(incomes.len(), 1);
    assert_eq!(incomes[0]["category"], "salary");
}

// ---------------------------------------------------------------------------
// TX-009 收支汇总：收入与支出不混淆，净额正确
// ---------------------------------------------------------------------------

#[test]
fn tx_009_summary_income_expense_not_mixed() {
    let db = TestDb::new();
    create_with(&db, "income", 500_000, TS_1, &["--category", "salary"]);
    create_with(&db, "expense", 1850, TS_2, &["--category", "food"]);
    create_with(&db, "expense", 500, TS_2, &["--category", "food"]);

    let summary = run_ok_json(&db, &summary_cmd(&[]));
    assert_eq!(summary["income_total"], 500_000);
    assert_eq!(summary["expense_total"], 2350);
    assert_eq!(summary["net_total"], 497_650);
    // 新结构没有按币种分组的 groups 字段。
    assert!(
        summary.get("groups").is_none(),
        "不应再按币种分组: {summary}"
    );

    // by_category 按 category 升序，收入与支出分别累计。
    let rows = summary["by_category"].as_array().expect("by_category");
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["category"], "food");
    assert_eq!(rows[0]["income"], 0);
    assert_eq!(rows[0]["expense"], 2350);
    assert_eq!(rows[1]["category"], "salary");
    assert_eq!(rows[1]["income"], 500_000);
    assert_eq!(rows[1]["expense"], 0);

    // 按类型筛选汇总：只统计支出。
    let expense_only = run_ok_json(&db, &summary_cmd(&["--type", "expense"]));
    assert_eq!(expense_only["income_total"], 0);
    assert_eq!(expense_only["expense_total"], 2350);
    assert_eq!(expense_only["net_total"], -2350);
    let (value, code) = run_fail_json(&db, &summary_cmd(&["--type", "transfer"]));
    assert_eq!(code, 2);
    assert_error_code(&value, "INVALID_ARGUMENT");
}

// ---------------------------------------------------------------------------
// TX-010 修改金额：新金额生效，其它字段不变
// ---------------------------------------------------------------------------

#[test]
fn tx_010_update_amount() {
    let db = TestDb::new();
    let created = create_with(&db, "expense", 1850, TS_1, &["--category", "food"]);
    let id = id_of(&created);

    let mut args = update_cmd(&id);
    args.extend(["--amount".to_string(), "2000".to_string()]);
    let updated = run_ok_json(&db, &args);
    assert_eq!(updated["id"], id);
    assert_eq!(updated["amount"], 2000);
    assert_eq!(updated["category"], "food", "未提供字段不得被覆盖");
    assert_eq!(updated["occurred_at"], TS_1);

    let fetched = run_ok_json(&db, &get_cmd(&id));
    assert_eq!(fetched["amount"], 2000);

    let summary = run_ok_json(&db, &summary_cmd(&[]));
    assert_eq!(summary["expense_total"], 2000);

    // 非法金额拒绝，且原值保持不变。
    let mut bad = update_cmd(&id);
    bad.extend(["--amount".to_string(), "0".to_string()]);
    let (value, code) = run_fail_json(&db, &bad);
    assert_eq!(code, 2);
    assert_error_code(&value, "INVALID_ARGUMENT");
    assert_eq!(run_ok_json(&db, &get_cmd(&id))["amount"], 2000);

    // 不存在记录返回 TRANSACTION_NOT_FOUND（退出码 1）。
    let mut missing = update_cmd("no-such-transaction");
    missing.extend(["--amount".to_string(), "100".to_string()]);
    let (value, code) = run_fail_json(&db, &missing);
    assert_eq!(code, 1);
    assert_error_code(&value, "TRANSACTION_NOT_FOUND");
}

// ---------------------------------------------------------------------------
// TX-011 修改类别：新类别生效
// ---------------------------------------------------------------------------

#[test]
fn tx_011_update_category() {
    let db = TestDb::new();
    let created = create_with(&db, "expense", 1850, TS_1, &["--category", "food"]);
    let id = id_of(&created);

    let mut args = update_cmd(&id);
    args.extend(["--category".to_string(), "transport".to_string()]);
    let updated = run_ok_json(&db, &args);
    assert_eq!(updated["category"], "transport");
    assert_eq!(updated["amount"], 1850);

    let fetched = run_ok_json(&db, &get_cmd(&id));
    assert_eq!(fetched["category"], "transport");

    // 旧类别不再匹配，新类别统计正确。
    assert!(run_ok_items(&db, &list_cmd(&["--category", "food"])).is_empty());
    assert_eq!(
        run_ok_items(&db, &list_cmd(&["--category", "transport"])).len(),
        1
    );

    let summary = run_ok_json(&db, &summary_cmd(&[]));
    let transport = category_row(&summary, "transport");
    assert_eq!(transport["expense"], 1850);
    assert!(
        summary["by_category"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["category"] != "food"),
        "旧类别不应出现在汇总中: {summary}"
    );
}

// ---------------------------------------------------------------------------
// TX-012 删除账目：删除后统计结果同步变化
// ---------------------------------------------------------------------------

#[test]
fn tx_012_delete_syncs_summary() {
    let db = TestDb::new();
    let first = create_with(&db, "expense", 1850, TS_1, &["--category", "food"]);
    let second = create_with(&db, "expense", 500, TS_2, &["--category", "transport"]);

    let summary = run_ok_json(&db, &summary_cmd(&[]));
    assert_eq!(summary["expense_total"], 2350);

    let first_id = id_of(&first);
    let deleted = run_ok_json(&db, &delete_cmd(&first_id));
    assert_eq!(deleted["id"], first_id);
    assert_eq!(deleted["deleted"], true);

    // 删除后无法再查询到，且列表只剩一条。
    let (value, code) = run_fail_json(&db, &get_cmd(&first_id));
    assert_eq!(code, 1);
    assert_error_code(&value, "TRANSACTION_NOT_FOUND");

    let items = run_ok_items(&db, &list_cmd(&[]));
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["id"], id_of(&second));

    // 汇总同步变化。
    let summary = run_ok_json(&db, &summary_cmd(&[]));
    assert_eq!(summary["expense_total"], 500);
    assert_eq!(summary["net_total"], -500);
    assert!(
        summary["by_category"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["category"] != "food"),
        "已删除记录不应参与汇总: {summary}"
    );

    // 重复删除返回 TRANSACTION_NOT_FOUND。
    let (value, code) = run_fail_json(&db, &delete_cmd(&first_id));
    assert_eq!(code, 1);
    assert_error_code(&value, "TRANSACTION_NOT_FOUND");
}

// ---------------------------------------------------------------------------
// TX-013 小数金额：16.50 元记为 1650，整数运算无浮点误差
// ---------------------------------------------------------------------------

#[test]
fn tx_013_fractional_amount_no_float_error() {
    let db = TestDb::new();
    let data = create_with(&db, "expense", 1650, TS_1, &["--category", "food"]);
    assert!(data["amount"].is_i64(), "金额必须序列化为整数: {data}");
    assert_eq!(data["amount"].as_i64(), Some(1650));

    create_with(&db, "expense", 3350, TS_2, &["--category", "food"]);
    let summary = run_ok_json(&db, &summary_cmd(&[]));
    assert_eq!(summary["expense_total"].as_i64(), Some(5000));
    assert_eq!(summary["net_total"].as_i64(), Some(-5000));
    let food = category_row(&summary, "food");
    assert_eq!(food["expense"].as_i64(), Some(5000));
}

// ---------------------------------------------------------------------------
// TX-014 单币种（人民币）：旧的 --currency / --amount-minor 参数已删除
// ---------------------------------------------------------------------------

#[test]
fn tx_014_single_currency_rmb_no_currency_field() {
    let db = TestDb::new();

    // 旧参数被 clap 拒绝：参数错误，不写入数据。
    let mut args = create_cmd("expense", 100, TS_1);
    args.extend(["--currency".to_string(), "CNY".to_string()]);
    let (value, code) = run_fail_json(&db, &args);
    assert_eq!(code, 2);
    assert_error_code(&value, "INVALID_ARGUMENT");

    let mut args = create_cmd("expense", 100, TS_1);
    args.extend(["--amount-minor".to_string(), "100".to_string()]);
    let (value, code) = run_fail_json(&db, &args);
    assert_eq!(code, 2);
    assert_error_code(&value, "INVALID_ARGUMENT");
    assert!(run_ok_items(&db, &list_cmd(&[])).is_empty());

    // 合法记录的输出只有 amount（人民币分），没有币种字段。
    let data = create_with(&db, "expense", 1850, TS_1, &[]);
    assert_eq!(data["amount"], 1850);
    assert!(data.get("currency").is_none());
    assert!(data.get("amount_minor").is_none());
}

// ---------------------------------------------------------------------------
// 额外边界用例
// ---------------------------------------------------------------------------

/// 空数据库：列表为空、汇总为零值且 by_category 为空（不是错误）。
#[test]
fn tx_extra_empty_database_summary_and_list() {
    let db = TestDb::new();

    let data = run_ok_json(&db, &list_cmd(&[]));
    assert_eq!(data["items"].as_array().map(Vec::len), Some(0));
    assert_eq!(data["count"], 0);

    let summary = run_ok_json(&db, &summary_cmd(&[]));
    assert_eq!(summary["income_total"], 0);
    assert_eq!(summary["expense_total"], 0);
    assert_eq!(summary["net_total"], 0);
    assert_eq!(summary["by_category"].as_array().map(Vec::len), Some(0));

    // 人类可读模式同样成功输出。
    let output = db.raw(&["transaction", "list"]);
    assert!(
        output.status.success(),
        "human mode failed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!output.stdout.is_empty());
}

/// 查询不存在 / 非法 ID。
#[test]
fn tx_extra_get_not_found() {
    let db = TestDb::new();

    let (value, code) = run_fail_json(&db, &get_cmd("does-not-exist"));
    assert_eq!(code, 1);
    assert_error_code(&value, "TRANSACTION_NOT_FOUND");

    let (value, code) = run_fail_json(&db, &get_cmd(""));
    assert_eq!(code, 2);
    assert_error_code(&value, "INVALID_ARGUMENT");
}

/// 修改可选字段：未提供不覆盖；--clear-* 显式清空。
#[test]
fn tx_extra_update_optional_fields_and_clear() {
    let db = TestDb::new();
    let created = create_with(
        &db,
        "expense",
        1850,
        TS_1,
        &["--category", "food", "--description", "午饭"],
    );
    let id = id_of(&created);

    // 只改描述，类别与金额保留。
    let mut args = update_cmd(&id);
    args.extend(["--description".to_string(), "晚饭".to_string()]);
    let updated = run_ok_json(&db, &args);
    assert_eq!(updated["description"], "晚饭");
    assert_eq!(updated["category"], "food");
    assert_eq!(updated["amount"], 1850);

    // 显式清空类别和描述。
    let mut args = update_cmd(&id);
    args.extend([
        "--clear-category".to_string(),
        "--clear-description".to_string(),
    ]);
    let updated = run_ok_json(&db, &args);
    assert_eq!(updated["category"], Value::Null);
    assert_eq!(updated["description"], Value::Null);
    assert_eq!(updated["amount"], 1850);
    assert_eq!(updated["type"], "expense");

    let fetched = run_ok_json(&db, &get_cmd(&id));
    assert_eq!(fetched["category"], Value::Null);
    assert_eq!(fetched["description"], Value::Null);
}

/// 创建时的时间校验：必须带时区偏移。
#[test]
fn tx_extra_create_validation_time() {
    let db = TestDb::new();

    for occurred_at in [
        "2026-11-17 12:00:00",
        "2026-11-17",
        "2026-11-17T12:00:00",
        "not-a-date",
        "",
    ] {
        let args = create_cmd("expense", 100, occurred_at);
        let (value, code) = run_fail_json(&db, &args);
        assert_eq!(code, 2, "occurred_at={occurred_at:?}");
        assert_error_code(&value, "INVALID_ARGUMENT");
    }

    assert!(run_ok_items(&db, &list_cmd(&[])).is_empty());
}

/// 列表/汇总筛选参数校验。
#[test]
fn tx_extra_filter_validation() {
    let db = TestDb::new();

    let (value, code) = run_fail_json(&db, &list_cmd(&["--type", "transfer"]));
    assert_eq!(code, 2);
    assert_error_code(&value, "INVALID_ARGUMENT");

    let (value, code) = run_fail_json(&db, &summary_cmd(&["--type", "transfer"]));
    assert_eq!(code, 2);
    assert_error_code(&value, "INVALID_ARGUMENT");

    let (value, code) = run_fail_json(&db, &list_cmd(&["--from", "not-a-date"]));
    assert_eq!(code, 2);
    assert_error_code(&value, "INVALID_ARGUMENT");

    let (value, code) = run_fail_json(&db, &summary_cmd(&["--to", "2026-11-17"]));
    assert_eq!(code, 2);
    assert_error_code(&value, "INVALID_ARGUMENT");
}

/// --input-json 创建与修改；非法 JSON 返回 INVALID_INPUT 且不写数据。
#[test]
fn tx_extra_input_json_create_and_update() {
    let db = TestDb::new();

    let create_args = vec![
        "transaction".to_string(),
        "create".to_string(),
        "--input-json".to_string(),
        "-".to_string(),
    ];
    let payload = r#"{"type":"expense","amount":1850,"category":"food","description":"午饭","occurred_at":"2026-11-17T12:00:00+08:00"}"#;
    let output = db.raw_json_with_stdin(&as_str_args(&create_args), payload);
    common::assert_success(&output, &as_str_args(&create_args));
    let data = parse_json(&output)["data"].clone();
    let id = id_of(&data);
    assert_eq!(data["amount"], 1850);
    assert_eq!(data["category"], "food");
    assert_eq!(data["user_id"], db.user_id);

    // 通过 stdin JSON 部分更新：只改 amount/category，其它字段保留。
    let update_args = vec![
        "transaction".to_string(),
        "update".to_string(),
        "--input-json".to_string(),
        "-".to_string(),
    ];
    let update_payload = format!(r#"{{"id":"{id}","amount":2400,"category":"transport"}}"#);
    let output = db.raw_json_with_stdin(&as_str_args(&update_args), &update_payload);
    common::assert_success(&output, &as_str_args(&update_args));
    let updated = parse_json(&output)["data"].clone();
    assert_eq!(updated["amount"], 2400);
    assert_eq!(updated["category"], "transport");
    assert_eq!(updated["type"], "expense");
    assert_eq!(updated["occurred_at"], "2026-11-17T12:00:00+08:00");

    // 非法 JSON：INVALID_INPUT，且不产生新记录。
    let (value, code) = run_fail_with_stdin(&db, &create_args, r#"{"type":"expense","#);
    assert_eq!(code, 2);
    assert_error_code(&value, "INVALID_INPUT");
    assert_eq!(run_ok_items(&db, &list_cmd(&[])).len(), 1);
}

/// 列表排序：julianday(occurred_at) ASC，同一时刻按 id ASC。
#[test]
fn tx_extra_list_ordering_by_utc() {
    let db = TestDb::new();
    // 三个不同偏移的等价时刻，按时间点而不是字符串排序。
    create_with(&db, "expense", 300, "2026-11-17T23:00:00+08:00", &[]);
    create_with(&db, "expense", 200, "2026-11-17T10:00:00Z", &[]);
    create_with(&db, "expense", 100, "2026-11-17T08:00:00+08:00", &[]);

    let items = run_ok_items(&db, &list_cmd(&[]));
    assert_eq!(amounts_of(&items), vec![100, 200, 300]);

    // 同一 UTC 时刻的两条记录按 id ASC 排序。
    let a = create_with(&db, "expense", 1000, "2026-11-18T08:00:00+08:00", &[]);
    let b = create_with(&db, "expense", 1000, "2026-11-18T00:00:00Z", &[]);
    let items = run_ok_items(&db, &list_cmd(&[]));
    let id_a = id_of(&a);
    let id_b = id_of(&b);
    let pos_a = items.iter().position(|i| i["id"] == id_a).unwrap();
    let pos_b = items.iter().position(|i| i["id"] == id_b).unwrap();
    let ordered = if id_a < id_b {
        pos_a < pos_b
    } else {
        pos_a > pos_b
    };
    assert!(ordered, "同一时刻应按 id ASC 排序: {items:?}");
}

/// 数据跨进程持久化：创建后新进程仍能查询到。
#[test]
fn tx_extra_persistence_across_invocations() {
    let db = TestDb::new();
    let created = create_with(&db, "income", 1234, TS_3, &["--category", "gift"]);
    let id = id_of(&created);

    // 每次调用都是新的 paadm 进程，等价于重启后查询。
    let fetched = run_ok_json(&db, &get_cmd(&id));
    assert_eq!(fetched["amount"], 1234);
    assert_eq!(fetched["occurred_at"], TS_3);

    let summary = run_ok_json(&db, &summary_cmd(&[]));
    assert_eq!(summary["income_total"], 1234);
}

/// 汇总：未分类（category=NULL）只计入总额，不生成 by_category 行；by_category 升序。
#[test]
fn tx_extra_summary_category_null_and_ordering() {
    let db = TestDb::new();
    create_with(&db, "income", 1000, TS_1, &[]);
    create_with(&db, "expense", 500, TS_1, &[]);
    create_with(&db, "expense", 1850, TS_2, &["--category", "food"]);
    create_with(&db, "expense", 300, TS_3, &["--category", "transport"]);

    let summary = run_ok_json(&db, &summary_cmd(&[]));
    assert_eq!(summary["income_total"], 1000);
    assert_eq!(summary["expense_total"], 2650);
    assert_eq!(summary["net_total"], -1650);

    let rows = summary["by_category"].as_array().expect("by_category");
    assert_eq!(
        rows.len(),
        2,
        "未分类记录不应生成 by_category 行: {summary}"
    );
    assert_eq!(rows[0]["category"], "food");
    assert_eq!(rows[0]["expense"], 1850);
    assert_eq!(rows[1]["category"], "transport");
    assert_eq!(rows[1]["expense"], 300);
}

/// 用户隔离：所有读取/修改/删除都必须按 user_id 过滤，跨用户表现为“不存在”。
#[test]
fn tx_extra_user_isolation() {
    let db = TestDb::new();
    let other = db.add_user("qq", "other-user");

    // 默认用户创建一条记录。
    let mine = create_with(&db, "expense", 1850, TS_1, &["--category", "food"]);
    let mine_id = id_of(&mine);
    assert_eq!(mine["user_id"], db.user_id);

    // 另一个用户的列表与汇总只统计自己的数据。
    let other_list = db.ok_json_as(&other, &["transaction", "list"]);
    assert_eq!(other_list["count"], 0);
    let other_summary = db.ok_json_as(&other, &["transaction", "summary"]);
    assert_eq!(other_summary["income_total"], 0);
    assert_eq!(other_summary["expense_total"], 0);

    // 另一个用户创建自己的记录。
    let mut args = create_cmd("income", 500_000, TS_2);
    args.extend(["--category".to_string(), "salary".to_string()]);
    let theirs = db.ok_json_as(&other, &as_str_args(&args));
    let theirs_id = id_of(&theirs);
    assert_eq!(theirs["user_id"], other);
    assert_ne!(mine_id, theirs_id);

    // 默认用户看不到对方记录；按 ID get/update/delete 均视为不存在。
    let my_items = run_ok_items(&db, &list_cmd(&[]));
    assert_eq!(my_items.len(), 1);
    assert_eq!(my_items[0]["id"], mine_id);

    let (value, code) = run_fail_json(&db, &get_cmd(&theirs_id));
    assert_eq!(code, 1);
    assert_error_code(&value, "TRANSACTION_NOT_FOUND");

    let mut update_args = update_cmd(&theirs_id);
    update_args.extend(["--amount".to_string(), "1".to_string()]);
    let (value, code) = run_fail_json(&db, &update_args);
    assert_eq!(code, 1);
    assert_error_code(&value, "TRANSACTION_NOT_FOUND");

    let (value, code) = run_fail_json(&db, &delete_cmd(&theirs_id));
    assert_eq!(code, 1);
    assert_error_code(&value, "TRANSACTION_NOT_FOUND");

    // 对方记录保持原样且对对方仍可见。
    let theirs_fetched = db.ok_json_as(&other, &as_str_args(&get_cmd(&theirs_id)));
    assert_eq!(theirs_fetched["amount"], 500_000);

    // 默认用户汇总只统计自己的记录。
    let summary = run_ok_json(&db, &summary_cmd(&[]));
    assert_eq!(summary["expense_total"], 1850);
    assert_eq!(summary["income_total"], 0);

    // 缺少 --user-id 的业务命令返回 INVALID_ARGUMENT（退出码 2）。
    let output = db.raw_json_without_user(&["transaction", "list"]);
    let code = output.status.code().unwrap_or(-1);
    assert_eq!(code, 2, "缺少 --user-id 应返回参数错误");
    assert_error_code(&parse_json(&output), "INVALID_ARGUMENT");
}
