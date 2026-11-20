#!/bin/sh
# paadm 端到端冒烟测试（新权威表结构：users/items/reminders/transactions/notes）。
# 真实二进制 + 全新 SQLite 数据库 + jq/python3 断言；不保留任何旧版本数据。
#
# 用法：
#   sh scripts/server-smoke.sh                        # 默认用仓库内 target/debug/paadm
#   PAADM_BIN=/path/to/paadm sh scripts/server-smoke.sh
#   PAADM_WORK_DIR=/tmp/paadm-smoke sh scripts/server-smoke.sh
SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
REPO_DIR=$(dirname "$SCRIPT_DIR")
BIN=${PAADM_BIN:-"$REPO_DIR/target/debug/paadm"}
WORK_DIR=${PAADM_WORK_DIR:-/tmp/paadm-verify}
DB="$WORK_DIR/paa.db"
rm -rf "$WORK_DIR"
mkdir -p "$WORK_DIR"
PASS=0
FAIL=0

ok() {
  name=$1; expr=$2; json=$3; rc=$4
  if [ "$rc" -eq 0 ] && printf '%s' "$json" | jq -e "$expr" >/dev/null 2>&1; then
    echo "PASS  $name"; PASS=$((PASS+1))
  else
    echo "FAIL  $name (rc=$rc) -> $json"; FAIL=$((FAIL+1))
  fi
}

bad() {
  name=$1; expr=$2; json=$3; rc=$4
  if [ "$rc" -ne 0 ] && printf '%s' "$json" | jq -e "$expr" >/dev/null 2>&1; then
    echo "PASS  $name"; PASS=$((PASS+1))
  else
    echo "FAIL  $name (rc=$rc) -> $json"; FAIL=$((FAIL+1))
  fi
}

J()  { "$BIN" --db "$DB" --json --user-id "$USER_ID" "$@" 2>/dev/null; }
J2() { "$BIN" --db "$DB" --json --user-id "$USER2_ID" "$@" 2>/dev/null; }

echo "== 1. 用户与基础设施 =="
out=$("$BIN" --db "$DB" --json user create --provider QQ --external-user-id smoke-user 2>/dev/null); rc=$?
ok "user create（provider 转小写）" '.success == true and .data.provider == "qq" and .data.external_user_id == "smoke-user"' "$out" "$rc"
USER_ID=$(printf '%s' "$out" | jq -r '.data.id')
export USER_ID
out=$("$BIN" --db "$DB" --json user resolve --provider qq --external-user-id smoke-user 2>/dev/null); rc=$?
ok "user resolve 幂等" '.success == true and .data.created == false and .data.id == env.USER_ID' "$out" "$rc"
out=$("$BIN" --db "$DB" --json user create --provider qq --external-user-id smoke-user 2>/dev/null); rc=$?
bad "重复创建用户 -> DUPLICATE_USER" '.error.code == "DUPLICATE_USER"' "$out" "$rc"
out=$("$BIN" --db "$DB" --json user create --provider wechat --external-user-id second-user 2>/dev/null); rc=$?
USER2_ID=$(printf '%s' "$out" | jq -r '.data.id')
out=$("$BIN" --db "$DB" --json init 2>/dev/null); rc=$?
ok "init schema v2" '.success == true and .data.schema_version == 2' "$out" "$rc"

echo "== 2. Task（items.type=task）=="
out=$(J task create --title "完成数学作业" --description "第 3 章习题" --priority 3 --due-at "2026-11-17T23:59:00+08:00"); rc=$?
ok "task create 完整字段" '.success == true and .data.type == "task" and .data.status == "pending" and .data.priority == 3 and .data.user_id == env.USER_ID and .data.all_day == false' "$out" "$rc"
TASK_ID=$(printf '%s' "$out" | jq -r '.data.id')
export TASK_ID
out=$(J task get --id "$TASK_ID"); rc=$?
ok "task get 按 ID" '.success == true and .data.title == "完成数学作业"' "$out" "$rc"
out=$(J task list --status pending); rc=$?
ok "task list 状态筛选" '.success == true and .data.count == 1' "$out" "$rc"
out=$(J task list --keyword "数学" --priority 3); rc=$?
ok "task list 关键词 + 优先级" '.success == true and .data.count == 1' "$out" "$rc"
out=$(J task update --id "$TASK_ID" --title "完成数学作业（修订）" --clear-due-at); rc=$?
ok "task update + 显式清空 due_at" '.data.title == "完成数学作业（修订）" and .data.due_at == null' "$out" "$rc"
out=$(J task complete --id "$TASK_ID"); rc=$?
ok "task complete 设置 completed_at" '.data.status == "completed" and .data.completed_at != null' "$out" "$rc"
out=$(J task complete --id "$TASK_ID"); rc=$?
ok "task complete 幂等" '.data.status == "completed"' "$out" "$rc"
out=$(J task create --title "  "); rc=$?
bad "空标题拒绝" '.error.code == "INVALID_ARGUMENT"' "$out" "$rc"
out=$(J task create --title 'x"; DROP TABLE items; --'); rc=$?
EVIL_ID=$(printf '%s' "$out" | jq -r '.data.id')
ok "特殊字符（SQL 安全）创建" '.success == true' "$out" "$rc"
out=$(J task get --id "$EVIL_ID"); rc=$?
ok "特殊字符原样保存" '.data.title == "x\"; DROP TABLE items; --"' "$out" "$rc"
out=$(J task get --id "no-such-task"); rc=$?
bad "task get 不存在 -> TASK_NOT_FOUND" '.error.code == "TASK_NOT_FOUND"' "$out" "$rc"

echo "== 3. Schedule（items.type=event）=="
out=$(J schedule create --title "组会" --description "周会" --start-at "2026-11-20T10:00:00+08:00" --end-at "2026-11-20T11:00:00+08:00" --location "实验室" --all-day 1); rc=$?
ok "schedule create 完整字段" '.success == true and .data.type == "event" and .data.all_day == true and .data.location == "实验室"' "$out" "$rc"
EVENT_ID=$(printf '%s' "$out" | jq -r '.data.id')
out=$(J schedule list --from "2026-11-20T09:00:00+08:00" --to "2026-11-20T12:00:00+08:00"); rc=$?
ok "schedule 区间相交查询" '.success == true and .data.count == 1' "$out" "$rc"
out=$(J schedule list --from "2026-11-21T00:00:00+08:00" --to "2026-11-22T00:00:00+08:00"); rc=$?
ok "schedule 区间外不返回" '.data.count == 0' "$out" "$rc"
out=$(J schedule create --title "bad" --start-at "2026-11-20T10:00:00+08:00" --end-at "2026-11-20T09:00:00+08:00"); rc=$?
bad "end 早于 start 拒绝" '.error.code == "INVALID_ARGUMENT"' "$out" "$rc"
out=$(J task get --id "$EVENT_ID"); rc=$?
bad "用 task 命令访问 event -> TASK_NOT_FOUND" '.error.code == "TASK_NOT_FOUND"' "$out" "$rc"
out=$(J schedule get --id "missing"); rc=$?
bad "schedule get 不存在 -> SCHEDULE_NOT_FOUND" '.error.code == "SCHEDULE_NOT_FOUND"' "$out" "$rc"

echo "== 4. Reminder（item 关联 / trigger / cancel）=="
out=$(J reminder create --content "交数学作业" --remind-at "2026-11-17T20:00:00+08:00" --item-id "$TASK_ID"); rc=$?
ok "reminder create 关联 item" '.success == true and .data.item_id == env.TASK_ID and .data.status == "pending"' "$out" "$rc"
REM_ID=$(printf '%s' "$out" | jq -r '.data.id')
out=$(J reminder list --item-id "$TASK_ID"); rc=$?
ok "reminder list 按 item 筛选" '.success == true and .data.count == 1' "$out" "$rc"
out=$(J reminder trigger --id "$REM_ID"); rc=$?
ok "reminder trigger -> triggered" '.data.status == "triggered"' "$out" "$rc"
out=$(J reminder cancel --id "$REM_ID"); rc=$?
ok "reminder cancel -> cancelled（保留记录）" '.data.status == "cancelled"' "$out" "$rc"
out=$(J reminder create --content "坏关联" --remind-at "2026-11-18T09:00:00+08:00" --item-id "missing-item"); rc=$?
bad "关联不存在 item -> ITEM_NOT_FOUND" '.error.code == "ITEM_NOT_FOUND"' "$out" "$rc"
# 删除被关联的待办：提醒保留且 item_id 置 NULL
out=$(J task create --title "关联会被删的待办"); rc=$?
REL_TASK_ID=$(printf '%s' "$out" | jq -r '.data.id')
out=$(J reminder create --content "关联提醒" --remind-at "2026-11-18T10:00:00+08:00" --item-id "$REL_TASK_ID"); rc=$?
REL_REM_ID=$(printf '%s' "$out" | jq -r '.data.id')
J task delete --id "$REL_TASK_ID" >/dev/null 2>&1
out=$(J reminder get --id "$REL_REM_ID"); rc=$?
ok "删除待办后提醒保留且 item_id 为 NULL" '.success == true and .data.item_id == null' "$out" "$rc"

echo "== 5. Transaction（人民币分、无币种）=="
out=$(J transaction create --type expense --amount 1850 --category food --description "午饭" --occurred-at "2026-11-17T12:00:00+08:00"); rc=$?
ok "transaction create 支出" '.success == true and .data.amount == 1850 and .data.type == "expense" and (.data | has("currency") | not)' "$out" "$rc"
TX_ID=$(printf '%s' "$out" | jq -r '.data.id')
out=$(J transaction create --type income --amount 300000 --category salary --occurred-at "2026-11-17T09:00:00+08:00"); rc=$?
ok "transaction create 收入" '.success == true and .data.type == "income"' "$out" "$rc"
out=$(J transaction summary --from "2026-11-01T00:00:00+08:00" --to "2026-12-01T00:00:00+08:00"); rc=$?
ok "summary 收支/净额" '.success == true and .data.income_total == 300000 and .data.expense_total == 1850 and .data.net_total == 298150' "$out" "$rc"
ok "summary 按类别统计" '(.data.by_category[] | select(.category == "food") | .expense) == 1850' "$out" "$rc"
out=$(J transaction create --type expense --amount -100 --occurred-at "2026-11-17T12:00:00+08:00"); rc=$?
bad "负金额拒绝" '.error.code == "INVALID_ARGUMENT"' "$out" "$rc"
out=$(J transaction summary --from "2020-01-01T00:00:00+08:00" --to "2020-02-01T00:00:00+08:00"); rc=$?
ok "空范围 summary 为零值" '.success == true and .data.income_total == 0 and .data.expense_total == 0 and .data.by_category == []' "$out" "$rc"
out=$(J transaction delete --id "$TX_ID"); rc=$?
ok "transaction delete" '.success == true and .data.deleted == true' "$out" "$rc"

echo "== 6. Note（无标签）=="
out=$(J note create --title "项目点子" --content '做一个"自动整理下载文件夹"的小工具；先记下来。'); rc=$?
ok "note create 中文/引号/分号" '.success == true and .data.title == "项目点子"' "$out" "$rc"
NOTE_ID=$(printf '%s' "$out" | jq -r '.data.id')
out=$(J note list --keyword "下载文件夹"); rc=$?
ok "note 关键词查询" '.success == true and .data.count == 1' "$out" "$rc"
out=$(J note update --id "$NOTE_ID" --content "更新后的内容"); rc=$?
ok "note update 内容" '.data.content == "更新后的内容"' "$out" "$rc"
out=$(J note create --content ""); rc=$?
bad "note 空内容拒绝" '.error.code == "INVALID_ARGUMENT"' "$out" "$rc"
out=$(J note get --id "missing"); rc=$?
bad "note get 不存在 -> NOTE_NOT_FOUND" '.error.code == "NOTE_NOT_FOUND"' "$out" "$rc"

echo "== 7. 用户数据隔离 =="
out=$(J2 task list); rc=$?
ok "第二个用户看不到第一个用户的待办" '.success == true and .data.count == 0' "$out" "$rc"
out=$(J2 task get --id "$TASK_ID"); rc=$?
bad "跨用户 get -> TASK_NOT_FOUND" '.error.code == "TASK_NOT_FOUND"' "$out" "$rc"
out=$(J2 note get --id "$NOTE_ID"); rc=$?
bad "跨用户 note get -> NOTE_NOT_FOUND" '.error.code == "NOTE_NOT_FOUND"' "$out" "$rc"

echo "== 8. Agent 调用契约 =="
out=$(printf '%s' '{"title":"来自 stdin 的待办","priority":2}' | "$BIN" --db "$DB" --json --user-id "$USER_ID" task create --input-json - 2>/dev/null); rc=$?
ok "stdin JSON 输入创建" '.success == true and .data.title == "来自 stdin 的待办" and .data.priority == 2' "$out" "$rc"
out=$(printf '%s' '{"title": ' | "$BIN" --db "$DB" --json --user-id "$USER_ID" task create --input-json - 2>/dev/null); rc=$?
bad "非法 JSON -> INVALID_INPUT" '.error.code == "INVALID_INPUT"' "$out" "$rc"
out=$(J task list); rc=$?
ok "stdout 单个 JSON 对象" '.success == true and (.data.items | length) == (.data.count)' "$out" "$rc"
STDOUT=$("$BIN" --db "$DB" --json --user-id "$USER_ID" task list 2>/dev/null)
if [ "$(printf '%s\n' "$STDOUT" | wc -l)" -eq 1 ]; then
  echo "PASS  stdout 只有一行 JSON"; PASS=$((PASS+1))
else
  echo "FAIL  stdout 行数不为 1"; FAIL=$((FAIL+1))
fi
"$BIN" --db "$DB" --user-id "$USER_ID" task list >/dev/null 2>&1 && { echo "PASS  人类可读模式可运行"; PASS=$((PASS+1)); } || { echo "FAIL  人类可读模式"; FAIL=$((FAIL+1)); }

echo "== 9. 数据库可靠性 =="
out=$(J init); rc=$?
ok "重复 init 不破坏数据" '.success == true and .data.schema_version == 2' "$out" "$rc"
out=$(J task list); rc=$?
ok "重启后数据仍在（每次命令都是新进程）" '.success == true and .data.count >= 2' "$out" "$rc"
WAL=$(python3 - "$DB" <<'PY'
import sqlite3, sys
conn = sqlite3.connect(sys.argv[1])
print(conn.execute("PRAGMA journal_mode").fetchone()[0])
PY
)
[ "$WAL" = "wal" ] && { echo "PASS  WAL 模式已启用"; PASS=$((PASS+1)); } || { echo "FAIL  WAL=$WAL"; FAIL=$((FAIL+1)); }
python3 - "$DB" <<'PY' > /tmp/paadm-schema-check.txt 2>&1
import sqlite3, sys
conn = sqlite3.connect(sys.argv[1])
tables = sorted(r[0] for r in conn.execute("select name from sqlite_master where type='table' and name not like 'sqlite_%'"))
indexes = sorted(r[0] for r in conn.execute("select name from sqlite_master where type='index' and name like 'idx_%'"))
migration_cols = [r[1] for r in conn.execute("pragma table_info(schema_migrations)")]
users = conn.execute("select count(*) from users").fetchone()[0]
items = conn.execute("select count(*) from items").fetchone()[0]
assert tables == ['items', 'notes', 'reminders', 'schema_migrations', 'transactions', 'users'], tables
assert len(indexes) == 8, indexes
assert migration_cols == ['version', 'applied_at'], migration_cols
assert users == 2, users
assert items >= 2, items
print("SCHEMA_OK")
PY
if [ "$(cat /tmp/paadm-schema-check.txt)" = "SCHEMA_OK" ]; then
  echo "PASS  表结构核对（6 表 / 8 索引 / schema_migrations.version,applied_at）"; PASS=$((PASS+1))
else
  echo "FAIL  表结构核对 -> $(cat /tmp/paadm-schema-check.txt)"; FAIL=$((FAIL+1))
fi

echo ""
echo "==== 冒烟测试结果: PASS=$PASS FAIL=$FAIL ===="
[ "$FAIL" -eq 0 ] || exit 1
