# paadm 实现协作说明（Lead 维护）

> 内部开发约定，不是用户文档。**表结构以 `docs/design/paadm数据表结构设计.md` 为唯一权威来源**；
> 本文只补充代码组织、CLI 契约与协作规则。旧的 `paadm完整设计方案.md` 中与表结构冲突的部分以数据表设计为准。

## 1. 总体结构

```
paadm/
  Cargo.toml                 # Lead 所有，不要修改（依赖已固定）
  src/
    main.rs / lib.rs         # Lead
    error.rs                 # Lead（稳定错误码，勿改）
    output/                  # Lead（统一 JSON 响应）
    storage/                 # Lead（连接、PRAGMA、迁移、路径解析）
    domain/common.rs         # Lead（共享校验/时间工具，勿改）
    domain/user.rs           # Lead（已完成）
    domain/item.rs           # items-dev 所有（待办+日程统一实体）
    domain/reminder.rs       # reminder-dev 所有
    domain/transaction.rs    # transaction-dev 所有
    domain/note.rs           # note-dev 所有
    services/<domain>.rs     # 对应领域开发者所有
    repositories/<domain>.rs # 对应领域开发者所有
    cli/mod.rs               # Lead（命令树、--user-id、分发，勿改）
    cli/user.rs              # Lead（已完成）
    cli/{task,schedule}.rs   # items-dev 所有（参数已定义，实现 run）
    cli/{reminder,transaction,note}.rs  # 对应领域开发者所有（参数已定义，实现 run）
  tests/common/mod.rs        # Lead（测试工具，勿改）
  tests/{task,schedule,reminder,transaction,note}.rs  # 对应领域开发者所有
  tests/storage.rs / tests/integration.rs              # Lead 所有
```

**写作用域**：只改自己领域文件；需要公共基础设施改动时给 Lead 发消息。

## 2. 数据库 schema（权威，见数据表结构设计）

共 6 张表：`users`、`items`、`reminders`、`transactions`、`notes`、`schema_migrations`。
迁移在 `src/storage/migrations.rs`：v1 建表，v2 建索引。

- `users(id TEXT PK, provider TEXT, external_user_id TEXT, created_at TEXT, UNIQUE(provider, external_user_id))`
- `items(id TEXT PK, user_id TEXT NOT NULL, title, description, type TEXT CHECK(task|event), status TEXT CHECK(pending|completed|cancelled) DEFAULT pending, priority INTEGER, start_at TEXT, end_at TEXT, due_at TEXT, all_day INTEGER NOT NULL DEFAULT 0 CHECK(0|1), location TEXT, created_at, updated_at, completed_at, UNIQUE(user_id,id), FK user_id→users ON DELETE CASCADE, CHECK(start_at IS NULL OR end_at IS NULL OR end_at > start_at))`
- `reminders(id TEXT PK, user_id TEXT NOT NULL, item_id TEXT, content TEXT NOT NULL, remind_at TEXT NOT NULL, status TEXT CHECK(pending|triggered|cancelled) DEFAULT pending, created_at, updated_at, FK user_id→users ON DELETE CASCADE, FK (user_id,item_id)→items(user_id,id) ON DELETE RESTRICT)`
- `transactions(id TEXT PK, user_id TEXT NOT NULL, type TEXT CHECK(income|expense), amount INTEGER NOT NULL CHECK(amount>0), category TEXT, description TEXT, occurred_at TEXT NOT NULL, created_at, updated_at, FK user_id→users ON DELETE CASCADE)`
- `notes(id TEXT PK, user_id TEXT NOT NULL, title TEXT, content TEXT NOT NULL, created_at, updated_at, FK user_id→users ON DELETE CASCADE)`
- `schema_migrations(version INTEGER PK, applied_at TEXT NOT NULL)`

关键点：

- **没有 `*_utc` 列**。时间只保存一份 ISO 8601 字符串（保留时区偏移）。
- **所有业务表都有 `user_id`**；所有查询/更新/删除都必须同时按 `user_id` 过滤，跨用户访问表现为“不存在”。
- **时间范围查询/排序**统一用 SQLite 时间函数处理时区偏移：
  - 范围：`julianday(col) >= julianday(?1)` / `julianday(col) <= julianday(?2)`；
  - 排序：`ORDER BY julianday(col) ASC, id ASC`（无时间列可用时按 `created_at`）。
- **删除事项**（items）：应用层在**同一事务**中先 `UPDATE reminders SET item_id = NULL WHERE user_id = ? AND item_id = ?`，再 `DELETE FROM items WHERE user_id = ? AND id = ?`（复合外键是 `ON DELETE RESTRICT`；不要依赖 `SET NULL`，SQLite 复合外键置空会连 `user_id` 一起置空）。
- 布尔值用 INTEGER 0/1；`all_day` 用 0/1 存储，JSON 输出建议为 bool。

## 3. 用户与隔离

- CLI 全局参数 `--user-id <USER_ID>`；业务命令通过 `crate::cli::require_user(&db, cli)?` 取得并校验用户（缺失 INVALID_ARGUMENT，未知 USER_NOT_FOUND）。
- 测试工具 `TestDb` 会自动创建默认测试用户并给所有命令加 `--user-id`；跨用户测试用 `db.add_user(...)` + `db.raw_json_as(...)` / `db.ok_json_as(...)`。
- 写 SQL 时必须使用 `user_id` 参数，不允许只按 id 查询后再假设归属。

## 4. 领域模型与输出约定

- ID：`uuid::Uuid::new_v4().to_string()`。
- `created_at` / `updated_at` / `completed_at`：`crate::domain::common::now_iso()`（UTC RFC3339）。
- 时间输入：`common::validate_datetime(field, s)?`（必须带偏移；返回 trim 后原串）。旧 `parse_datetime` 仅为兼容保留。
- 文本：`validate_required_text` / `validate_optional_text`；金额：`common::validate_amount(i64)`（>0，人民币分）；`all_day`：`common::validate_all_day(i64)`。
- **items 统一实体**：字段 `id,user_id,title,description,type,status,priority,start_at,end_at,due_at,all_day,location,created_at,updated_at,completed_at`。JSON 输出 item 对象；`priority` 为整数（可 null）；`all_day` 输出 bool。
  - task 命令组只操作 `type='task'`，schedule 命令组只操作 `type='event'`；按 ID 取/改/删时要同时校验 type，否则分别返回 TASK_NOT_FOUND / SCHEDULE_NOT_FOUND。
  - task：`title` 必填、默认 `status=pending`、可用 `priority`(int)、`due_at`；`complete` 设 `status=completed` + `completed_at`（重复完成幂等）；`cancel` 设 `cancelled`（清空 `completed_at`）。
  - event：`start_at` 必填；`end_at` 可选且必须**严格晚于** start_at（应用层按时间点比较，同时数据库 CHECK 兜底）；`all_day` 0/1；`location` 可选。
  - 列表排序：task `created_at ASC, id ASC`；event `julianday(start_at) ASC, id ASC`（跨时区正确）。
  - event 区间查询固定 `[from, to)` 相交：`julianday(start_at) < julianday(to)` 且 `julianday(COALESCE(end_at, start_at)) > julianday(from)`。
  - 修改只覆盖提供字段，`--clear-*` 显式清空；任何成功修改刷新 `updated_at`。
- **reminder**：`content`/`remind_at` 必填；`item_id` 可选，提供时必须属于当前用户且存在（否则按关联错误返回；item 不存在 → 建议 ITEM 相关错误，用 `AppError::invalid_argument` 或 `TASK_NOT_FOUND` 需固定并测试）；`status` 默认 `pending`；`trigger` → `triggered`；`cancel` → `cancelled`（保留记录）；删除物理删除。排序 `julianday(remind_at) ASC, id ASC`；`--from/--to` 闭区间。
- **transaction**：`type` ∈ income/expense；`amount` >0 整数（人民币分）；`occurred_at` 必填；`category/description` 可选；排序 `julianday(occurred_at) ASC, id ASC`；summary 输出建议：
  ```json
  {"income_total": 300000, "expense_total": 1850, "net_total": 298150,
   "by_category": [{"category": "food", "income": 0, "expense": 1850}]}
  ```
  （category 为 NULL 的记录只计入总额，不生成 by_category 行；by_category 按 category 升序。）
- **note**：`content` 必填，`title` 可选；无标签/分类；排序 `created_at ASC, id ASC`；关键词匹配 title 或 content。
- 通用输出：单条 data 为记录对象；list `{items, count}`；delete `{id, deleted: true}`；空列表是成功。`--input-json` 用 `crate::cli::load_input_json::<对应 Args>(src)?`，字段名与长参数一致（snake_case）。

## 5. 错误码

`INVALID_ARGUMENT`(2)/`INVALID_INPUT`(2) 参数与输入；`USER_NOT_FOUND`/`TASK_NOT_FOUND`/`SCHEDULE_NOT_FOUND`/`REMINDER_NOT_FOUND`/`TRANSACTION_NOT_FOUND`/`NOTE_NOT_FOUND`/`DUPLICATE_USER`/`DATABASE_*`(1)。不要暴露 SQL；不要 panic。

## 6. TDD 工作方式（必须遵守）

1. 先写自己领域的测试（`tests/<domain>.rs`，`mod common;`）。
2. 运行 `cargo test --test <domain>` 确认失败（Red）。
3. 实现 domain/service/repository/cli 到全绿（Green）。
4. 最后 `cargo build` 确认整体可编译。
5. 并行开发时其他领域文件可能短暂编译失败：错误不在自己文件时重试或继续，必要时报告 Lead。

测试要求覆盖新数据表语义：用户隔离（跨用户不可见）、items 的 task/event 区分与状态流转、reminder 的 item 关联/trigger/cancel、transaction 金额规则与汇总、note 增删改查。用 `TestDb` 驱动真实二进制，禁止 mock / sleep / 网络。

## 7. 已知限制（与权威表结构相关）

1. **不迁移旧表结构**：本次重构直接使用全新的 v1（6 张表）与 v2（8 个索引）迁移；
   旧的 `tasks/schedules/...` 数据库不兼容，按需求直接删除重建，不提供旧库迁移。
2. **`items` 的 `CHECK (end_at > start_at)` 是 TEXT 字典序比较**：应用层按真实时间点
   校验 `end_at` 严格晚于 `start_at`，一般与数据库 CHECK 一致；但如果调用方给
   start_at/end_at 混用不同时区偏移写法，可能出现“应用层判断合法、数据库 CHECK 拒绝”
   （返回 INVALID_ARGUMENT）。第一版建议同一事项的起止时间使用同一时区偏移写法。
3. `schema_migrations` 只有 `version` / `applied_at`，与权威表结构一致。

## 8. 其他

- 不要新增依赖、不要修改 Cargo.toml。
- 不要 git commit；服务器由 Lead 统一验证。
- 完成时把「测试数量 + 通过结果 + 改动文件 + 遗留问题」send_message 给 Lead。
