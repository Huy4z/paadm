# paadm

PAA 的本地个人事务数据管理 CLI（Rust + SQLite）。

- 供 PAA Agent 以子进程 + JSON 协议调用，也可由用户本人直接在终端使用。
- 只负责“已经明确表达出来的结构化操作”：参数校验、业务规则、增删改查、事务与持久化。
- 不包含自然语言理解、LLM 调用、消息推送与后台调度。

设计与数据表依据：

- **数据表结构（权威）**：`docs/design/paadm数据表结构设计.md`
- 设计方案：`docs/design/paadm完整设计方案.md`
- 测试用例：`docs/design/PAA_paadm_第一版测试用例.md`
- 内部实现约定：`docs/paadm-impl-notes.md`

## 数据模型（第一版 6 张表）

| 表 | 用途 |
|---|---|
| `users` | PAA 用户与外部平台身份（provider + external_user_id）映射 |
| `items` | 待办与日程统一存储（`type='task'` / `type='event'`） |
| `reminders` | 提醒记录，`item_id` 可关联某个待办/日程，也可为独立提醒 |
| `transactions` | 收入与支出，金额为整数人民币分 |
| `notes` | 笔记 |
| `schema_migrations` | 数据库结构版本 |

约定：所有业务表都带 `user_id`，所有命令按用户隔离；时间用 ISO 8601 TEXT（带时区偏移，不另存 UTC 列）；金额用整数（¥18.50 → `1850`）；布尔用 0/1。

## 用户与 `--user-id`

paadm 的每个外部平台身份对应一个独立 PAA 用户：

```bash
# 创建用户（同一 provider + external_user_id 重复创建会返回 DUPLICATE_USER）
paadm --json user create --provider qq --external-user-id 123456

# 查找或创建（Agent 首次接触某平台用户时使用）
paadm --json user resolve --provider qq --external-user-id 123456

# 按 ID 查询
paadm --json user get --id "<user-id>"
```

之后所有业务命令都必须带 `--user-id <USER_ID>`：

```bash
paadm --user-id "<user-id>" --json task create --title "完成数学作业"
```

数据按用户隔离：跨用户查询/修改/删除表现为“记录不存在”。

## 技术栈

| 用途 | 依赖 |
|---|---|
| CLI 参数解析 | clap |
| JSON 输入输出 | serde / serde_json |
| SQLite | rusqlite（bundled，自带 SQLite） |
| ID | uuid v4 |
| 时间 | time（RFC 3339） |
| 错误 | thiserror（稳定错误码） |

> 本环境无法直连 crates.io，`.cargo/config.toml` 已配置 `rsproxy.cn` 镜像。

## 构建与测试

```bash
cd paadm
cargo build
cargo test          # 全部集成测试（每个用例使用独立临时数据库 + 独立测试用户）
```

## 数据库路径

优先级从高到低：

1. `--db <PATH>` 全局参数
2. 环境变量 `PAADM_DB_PATH`
3. 环境变量 `PAADM_DATA_DIR` 下的 `paa.db`
4. `$XDG_DATA_HOME/paa/paa.db`
5. `$HOME/.local/share/paa/paa.db`

任意命令首次运行都会自动创建数据库并执行迁移；`paadm init` 可显式初始化（幂等）。

## 命令一览

```text
paadm [--json] [--db <PATH>] [--user-id <USER_ID>] <command>

paadm init
paadm version
paadm user        create|get|resolve

paadm task        create|get|list|update|complete|cancel|delete
paadm schedule    create|get|list|update|delete
paadm reminder    create|get|list|update|trigger|cancel|delete
paadm transaction create|get|list|update|delete|summary
paadm note        create|get|list|update|delete
```

## 使用示例

```bash
export PAADM_DB_PATH="$HOME/.local/share/paa/paa.db"
USER_ID=$(paadm --json user resolve --provider qq --external-user-id 123456 | jq -r '.data.id')

# 创建待办（priority 为整数；due_at 必须带时区偏移）
paadm --user-id "$USER_ID" --json task create \
  --title "完成数学作业" \
  --due-at "2026-11-17T23:59:00+08:00" \
  --priority 3

# 查询待处理待办 / 完成待办
paadm --user-id "$USER_ID" --json task list --status pending
paadm --user-id "$USER_ID" --json task complete --id "<task-id>"

# 创建日程（items.type=event）
paadm --user-id "$USER_ID" --json schedule create \
  --title "组会" \
  --start-at "2026-11-20T10:00:00+08:00" \
  --end-at "2026-11-20T11:00:00+08:00" \
  --location "实验室" --all-day 0

# 创建提醒（可关联待办/日程，也可独立）
paadm --user-id "$USER_ID" --json reminder create \
  --content "交数学作业" \
  --remind-at "2026-11-17T20:00:00+08:00" \
  --item-id "<task-id>"

# 记录支出（整数人民币分：18.50 元 → 1850）与本月汇总
paadm --user-id "$USER_ID" --json transaction create \
  --type expense --amount 1850 --category food \
  --description "午饭" --occurred-at "2026-11-17T12:00:00+08:00"

paadm --user-id "$USER_ID" --json transaction summary \
  --from "2026-11-01T00:00:00+08:00" \
  --to "2026-12-01T00:00:00+08:00"

# 笔记
paadm --user-id "$USER_ID" --json note create --content "记一个项目点子..."
```

长文本可用 stdin JSON 输入（字段名与 CLI 长参数一致，snake_case）：

```bash
echo '{"title":"完成数学作业","priority":3}' | \
  paadm --user-id "$USER_ID" --json task create --input-json -
```

## JSON 契约

成功：

```json
{"success": true, "data": {"id": "..."}, "error": null}
```

失败：

```json
{"success": false, "data": null, "error": {"code": "TASK_NOT_FOUND", "message": "..."}}
```

- 单条记录：`data` 为记录对象（含 `id`、`user_id` 等）。
- 列表：`data = {"items": [...], "count": N}`；空列表是成功。
- 删除：`data = {"id": "...", "deleted": true}`。
- 汇总：`{"income_total": N, "expense_total": N, "net_total": N, "by_category": [{"category": "food", "income": 0, "expense": 1850}]}`。
- stdout 只输出一个 JSON 对象；日志只写 stderr。
- 退出码：成功 0；参数/输入错误 2；业务/存储错误 1。

### 错误码

`INVALID_ARGUMENT`、`INVALID_INPUT`、`USER_NOT_FOUND`、`DUPLICATE_USER`、`TASK_NOT_FOUND`、
`SCHEDULE_NOT_FOUND`、`ITEM_NOT_FOUND`、`REMINDER_NOT_FOUND`、`TRANSACTION_NOT_FOUND`、`NOTE_NOT_FOUND`、
`DUPLICATE_ID`、`DATABASE_BUSY`、`DATABASE_ERROR`、`MIGRATION_FAILED`、`INTERNAL_ERROR`。

## 实现结构

```text
src/
  main.rs / lib.rs
  cli/           参数解析与命令分发（user/task/schedule/reminder/transaction/note）
  domain/        实体、状态枚举、输入校验、共享时间/文本工具
  services/      业务规则与跨字段校验（所有操作带 user_id）
  repositories/  参数化 SQL 读写（所有业务查询按 user_id 过滤）
  storage/       连接、PRAGMA（WAL、外键、busy_timeout）、迁移
  output/        统一 JSON 响应
  error.rs       稳定错误码
tests/
  common/        集成测试工具（真实子进程 + 临时 SQLite + 默认测试用户）
  integration.rs CLI/USER/ITEM/DB/SYS/AGENT 契约测试
  storage.rs     存储层测试
  task.rs schedule.rs reminder.rs transaction.rs note.rs   各领域测试
```

时间与范围查询约定：输入 ISO 8601 必须带时区偏移；范围过滤与排序在 SQL 中用
`julianday()` 处理时区偏移；事件区间相交规则固定为 `[from, to)`。
删除事项时，先在同一事务中把关联提醒的 `item_id` 置空，再删除事项（复合外键为 `ON DELETE RESTRICT`）。

验证结果详见 `docs/paadm-verification.md`（本地与服务器测试、端到端冒烟）。
