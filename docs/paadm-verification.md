# paadm 第一版验证报告（按权威数据表结构重构后）

- 验证时间：2026-11-17
- 代码位置：`paadm/`
- 表结构依据（权威）：`docs/design/paadm数据表结构设计.md`
- 其他依据：`docs/design/paadm完整设计方案.md`、`docs/design/PAA_paadm_第一版测试用例.md`
- 验证方式：TDD（先写失败测试，再实现至通过）+ 本地全量测试 + 独立 Linux 服务器全新数据库构建/测试/端到端冒烟

## 1. 结论

| 项目 | 结果 |
|---|---|
| 本地 `cargo test` | **183 passed, 0 failed**（9 个测试目标） |
| 本地 `cargo fmt --check` | 通过 |
| 本地 `cargo clippy --all-targets` | 无警告 |
| 服务器 `cargo test` | **183 passed, 0 failed** |
| 服务器 release 构建 | 通过（10.81s），release 二进制冒烟正常 |
| 服务器端到端冒烟（全新空数据库） | **51 项全部通过（PASS=51 / FAIL=0）** |

服务器信息：Ubuntu 22.04.5，2 vCPU；Rust 1.100.0（rsproxy.cn 镜像安装），
通过 SSH 远程执行构建与测试。

## 2. 按权威数据表结构落地情况

第一版共 **6 张表**，与设计文档一致：

| 表 | 字段（实现） |
|---|---|
| `users` | `id, provider, external_user_id, created_at`，`UNIQUE(provider, external_user_id)` |
| `items` | `id, user_id, title, description, type(task/event), status(pending/completed/cancelled), priority(INTEGER), start_at, end_at, due_at, all_day(0/1), location, created_at, updated_at, completed_at`，`UNIQUE(user_id,id)` |
| `reminders` | `id, user_id, item_id(可空), content, remind_at, status(pending/triggered/cancelled), created_at, updated_at` |
| `transactions` | `id, user_id, type(income/expense), amount(INTEGER>0，人民币分), category, description, occurred_at, created_at, updated_at` |
| `notes` | `id, user_id, title, content, created_at, updated_at` |
| `schema_migrations` | `version, applied_at` |

其他约定落实情况：

- **用户隔离**：所有业务表带 `user_id`；所有读写 SQL 均按 `user_id` 过滤，跨用户访问表现为对应记录 `*_NOT_FOUND`。
- **外键**：`items/reminders/transactions/notes.user_id → users(id) ON DELETE CASCADE`；
  `reminders (user_id, item_id) → items(user_id, id) ON DELETE RESTRICT`。
- **删除事项**：应用层在同一事务中先 `UPDATE reminders SET item_id = NULL`，再按 `type` 删除事项。
- **时间**：ISO 8601 TEXT，保留时区偏移；范围过滤与排序在 SQL 中用 `julianday()`，跨时区正确；无 `*_utc` 列。
- **金额**：`amount` 为整数人民币分（¥18.50 → 1850），无币种字段；汇总按人民币给出收入/支出/净额与类别统计。
- **索引**：8 个索引与设计文档一致（`idx_items_user_status/start/due`、`idx_reminders_user_status_time/item`、`idx_transactions_user_time/category`、`idx_notes_user_created`）。
- **迁移**：v1 建 6 张表（`schema_migrations` 由迁移器引导创建），v2 建 8 个索引；**不迁移旧表结构**（按需求直接使用全新数据库）。
- `users` 支持：`paadm user create / get / resolve`；业务命令通过全局 `--user-id` 指定用户。

## 3. TDD 过程证据

1. 先重写公共骨架（新 schema/迁移、users 与 `--user-id`、CLI 根、测试工具），各领域 `run` 暂返回 `INTERNAL_ERROR`。
2. 各领域先重写测试并运行，得到 Red：
   - Task（items/task）：`FAILED. 0 passed; 38 failed`
   - Schedule（items/event）：`FAILED. 0 passed; 26 failed`
   - Reminder：`FAILED. 0 passed; 19 failed`
   - Transaction：`FAILED. 0 passed; 24 failed`
   - Note：`FAILED. 0 passed; 25 failed`
   - Lead 集成测试在重构中途为 `10 passed; 25 failed`（依赖领域实现）
3. 实现/重构到 Green；最后全量回归：183 passed。

## 4. 测试目标与用例覆盖

| 测试目标 | 数量 | 覆盖 |
|---|---:|---|
| `tests/task.rs` | 38 | TASK-001..028 + priority 整数筛选、用户隔离、task/event 类型隔离、删除事务等 |
| `tests/schedule.rs` | 26 | SCH-001..014 + all_day、跨时区排序、区间边界、用户隔离、--input-json 等 |
| `tests/reminder.rs` | 19 | REM-001..016 + REL-001/002/003：item 关联/解除、pending/triggered/cancelled 状态机、跨用户 |
| `tests/transaction.rs` | 24 | TX-001..014 + summary 新结构（人民币、无币种）、类别统计、用户隔离 |
| `tests/note.rs` | 25 | NOTE-001..008 + 长文本/中文特殊字符/关键词/范围/用户隔离（无标签） |
| `tests/storage.rs` | 13 | 新 schema 建表、迁移 v2 补齐索引、WAL、外键 CASCADE/RESTRICT、删除事项事务、回滚、备份恢复、损坏库 |
| `tests/integration.rs` | 35 | CLI-001..010、USER-001..004、ITEM-001、DB-001..006、SYS-001..008、AGENT-001..010、REL-005/006 |
| `src/lib.rs` 单测 | 3 | 领域枚举/校验单元测试 |
| **合计** | **183** | |

## 5. 服务器端到端冒烟覆盖（51 项）

`paadm/scripts/server-smoke.sh` 在服务器上使用**全新空数据库** + 真实二进制 + jq/python3 断言：

1. 用户与基础设施：user create（provider 转小写）、user resolve 幂等、重复创建 `DUPLICATE_USER`、init（schema v2）；
2. Task：完整字段（`type=task`、priority 整数、`user_id`、`all_day`）、查询/筛选、修改 + 显式清空、完成与幂等、空标题拒绝、SQL 注入文本安全、`TASK_NOT_FOUND`；
3. Schedule：`type=event`、all_day、区间相交/区间外、end 早于 start 拒绝、用错命令组返回 `TASK_NOT_FOUND`、`SCHEDULE_NOT_FOUND`；
4. Reminder：关联 item、`trigger`、`cancel` 保留记录、关联不存在 item → `ITEM_NOT_FOUND`、删除被关联待办后提醒保留且 `item_id=NULL`；
5. Transaction：支出/收入创建、`amount` 人民币分、无 `currency` 字段、汇总（收入 300000/支出 1850/净额 298150）、按类别统计、负金额拒绝、空范围零值汇总；
6. Note：中文/引号/分号、关键词、修改、空内容拒绝、`NOTE_NOT_FOUND`；
7. 用户隔离：第二个用户看不到第一个用户的数据，跨用户 get → `TASK_NOT_FOUND` / `NOTE_NOT_FOUND`；
8. Agent 契约：stdin JSON 创建、非法 JSON → `INVALID_INPUT`、stdout 单行 JSON、人类可读模式；
9. 可靠性：重复 init 保留数据、跨进程持久化、WAL、表结构核对（6 表 / 8 索引 / `schema_migrations(version, applied_at)`）。

结果：`PASS=51 FAIL=0`，`SCRIPT_RC=0`。

## 6. 服务器数据清理

按用户要求，验证完成后已删除服务器上的全部测试数据与测试产物（测试 SQLite 库、临时库、测试日志、旧源码包/冒烟脚本）；
服务器只保留 Rust 工具链与源码目录（如 `~/paadm`），不保留任何业务测试数据。
旧表结构不迁移：新版本只支持全新 schema。

## 7. 复现方式

```bash
# 本地
cd paadm && cargo test

# 服务器（把 <SERVER> / <SERVER_WORKDIR> 换成实际值）
ssh <SERVER> "cd <SERVER_WORKDIR> && cargo test"
ssh <SERVER> "sh <SERVER_WORKDIR>/scripts/server-smoke.sh"
```
