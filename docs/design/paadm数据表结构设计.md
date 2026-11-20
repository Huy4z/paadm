# PAA 数据库表结构（第一版）

> 适用范围：PAA 工具子系统的数据存储层。  
> 数据库：SQLite。  
> 设计原则：优先满足当前使用场景，保持表结构简单；不为尚未确定的需求预留复杂抽象。

## 1. 设计约定

### 1.1 数据范围

第一版包含以下业务表：

| 表名 | 用途 |
|---|---|
| `users` | PAA 用户及其外部平台身份 |
| `items` | 待办与日程 |
| `reminders` | 独立提醒，以及关联待办/日程的提醒 |
| `transactions` | 收入与支出记录 |
| `notes` | 普通笔记 |
| `schema_migrations` | 数据库结构版本记录 |

不建立独立的周期规则表、分类表、账户表、标签表或笔记关联表。

### 1.2 通用约定

- 主键使用 UUID，存储为 `TEXT`。
- 时间使用 ISO 8601 格式的 `TEXT`，例如 `2026-11-17T15:00:00+08:00`。
- 所有业务数据表都包含 `user_id`，所有业务查询都必须按 `user_id` 过滤。
- SQLite 连接建立后启用外键约束：`PRAGMA foreign_keys = ON;`。
- 布尔值使用 `INTEGER`，只允许 `0` 或 `1`。
- 枚举值使用 `TEXT`，由数据库 `CHECK` 约束和应用层共同校验。
- 金额使用整数，单位为人民币分，避免浮点数精度问题。
- `created_at` 和 `updated_at` 由应用层维护，更新时间在每次修改记录时更新。

### 1.3 用户身份

每个外部平台身份对应一个独立的 PAA 用户。

例如，同一个自然人通过 QQ 和微信联系 PAA，会被视为两个独立用户，不共享业务数据。第一版不提供跨平台身份绑定或数据合并。

`provider` 使用平台标识，例如 `qq`、`wechat`；`external_user_id` 保存对应平台提供的稳定用户标识。

---

## 2. 表结构

以下 SQL 以 SQLite 为目标。建表顺序已考虑外键依赖。

### 2.1 `users` — 用户

保存 PAA 内部用户 ID 与外部平台身份的映射。

| 字段 | 类型 | 约束 | 说明 |
|---|---|---|---|
| `id` | TEXT | PRIMARY KEY | PAA 内部用户 ID，UUID |
| `provider` | TEXT | NOT NULL | 外部平台，例如 `qq` |
| `external_user_id` | TEXT | NOT NULL | 外部平台用户 ID |
| `created_at` | TEXT | NOT NULL | 创建时间 |

同一平台的外部用户 ID 不允许重复。

```sql
CREATE TABLE users (
    id TEXT PRIMARY KEY,
    provider TEXT NOT NULL,
    external_user_id TEXT NOT NULL,
    created_at TEXT NOT NULL,

    UNIQUE (provider, external_user_id),
    UNIQUE (id)
);
```

### 2.2 `items` — 待办与日程

待办和日程统一存储，通过 `type` 区分。

| 字段 | 类型 | 约束 | 说明 |
|---|---|---|---|
| `id` | TEXT | PRIMARY KEY | 记录 ID，UUID |
| `user_id` | TEXT | NOT NULL | 所属用户 |
| `title` | TEXT | NOT NULL | 标题 |
| `description` | TEXT | NULL | 详细描述 |
| `type` | TEXT | NOT NULL | `task` / `event` |
| `status` | TEXT | NOT NULL | `pending` / `completed` / `cancelled` |
| `priority` | INTEGER | NULL | 优先级，可按应用约定取值 |
| `start_at` | TEXT | NULL | 开始时间 |
| `end_at` | TEXT | NULL | 结束时间 |
| `due_at` | TEXT | NULL | 最晚完成时间 |
| `all_day` | INTEGER | NOT NULL DEFAULT 0 | 是否全天事项，`0` / `1` |
| `location` | TEXT | NULL | 地点 |
| `created_at` | TEXT | NOT NULL | 创建时间 |
| `updated_at` | TEXT | NOT NULL | 更新时间 |
| `completed_at` | TEXT | NULL | 完成时间 |

时间字段语义：

- `start_at`：事项开始时间。
- `end_at`：事项结束时间。
- `due_at`：待办最晚完成时间。
- 普通待办通常只需要 `due_at`；有明确安排时间的日程通常使用 `start_at` 和 `end_at`。
- `type` 由调用方明确指定，不根据时间字段是否为空推断。

```sql
CREATE TABLE items (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,

    title TEXT NOT NULL,
    description TEXT,
    type TEXT NOT NULL CHECK (type IN ('task', 'event')),
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'completed', 'cancelled')),
    priority INTEGER,

    start_at TEXT,
    end_at TEXT,
    due_at TEXT,
    all_day INTEGER NOT NULL DEFAULT 0 CHECK (all_day IN (0, 1)),
    location TEXT,

    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    completed_at TEXT,

    UNIQUE (user_id, id),
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE,
    CHECK (
        start_at IS NULL
        OR end_at IS NULL
        OR end_at > start_at
    )
);
```

### 2.3 周期性日程的处理方式

第一版不建立周期规则表。周期性日程由 Agent 解析后，展开为多条普通 `items` 记录。

例如，用户要求“每周一、三、五晚上 8 点训练，持续一个月”，Agent 计算出对应日期，然后逐条创建日程。

约定：

- 用户指定结束日期时，按结束日期生成全部记录。
- 用户没有指定结束日期、只说“以后每周”时，第一版默认生成未来 90 天的记录。
- 第一版不自动续期；未来确有需要时，再增加续期能力。
- 每次生成的日程都是独立记录，可以单独修改、完成或删除。
- `items` 表本身不区分普通日程和由周期描述展开的日程。

### 2.4 `reminders` — 提醒

提醒独立于待办和日程存在。`item_id` 可以为空：

- `item_id` 为空：独立提醒。
- `item_id` 不为空：关联某个待办或日程。

| 字段 | 类型 | 约束 | 说明 |
|---|---|---|---|
| `id` | TEXT | PRIMARY KEY | 提醒 ID，UUID |
| `user_id` | TEXT | NOT NULL | 所属用户 |
| `item_id` | TEXT | NULL | 关联事项 ID，可为空 |
| `content` | TEXT | NOT NULL | 提醒内容 |
| `remind_at` | TEXT | NOT NULL | 触发时间 |
| `status` | TEXT | NOT NULL | `pending` / `triggered` / `cancelled` |
| `created_at` | TEXT | NOT NULL | 创建时间 |
| `updated_at` | TEXT | NOT NULL | 更新时间 |

```sql
CREATE TABLE reminders (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    item_id TEXT,

    content TEXT NOT NULL,
    remind_at TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'triggered', 'cancelled')),

    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,

    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE,
    FOREIGN KEY (user_id, item_id)
        REFERENCES items(user_id, id)
        ON DELETE RESTRICT
);
```

删除事项时的处理：

- 如果事项没有关联提醒，可以直接删除。
- 如果事项有关联提醒，应用层在同一个事务中先将这些提醒的 `item_id` 更新为 `NULL`，再删除事项。
- 这样既保留提醒，又避免提醒错误地关联到其他用户的事项。
- 不要直接使用 `ON DELETE SET NULL` 处理复合外键，因为 SQLite 会尝试同时将复合外键中的 `user_id` 也置空。

第一版只保存一次性提醒。提醒的实际触发、定时扫描和消息平台推送由后续 Scheduler / 消息发送模块负责，不属于数据库表的职责。

### 2.5 `transactions` — 收入与支出

收入和支出统一存储，通过 `type` 区分。

第一版只使用人民币，不保存币种字段；分类为一级分类，直接以文本保存在流水记录中，不建立独立分类表。

| 字段 | 类型 | 约束 | 说明 |
|---|---|---|---|
| `id` | TEXT | PRIMARY KEY | 记录 ID，UUID |
| `user_id` | TEXT | NOT NULL | 所属用户 |
| `type` | TEXT | NOT NULL | `income` / `expense` |
| `amount` | INTEGER | NOT NULL | 金额，单位为人民币分 |
| `category` | TEXT | NULL | 一级分类 |
| `description` | TEXT | NULL | 备注 |
| `occurred_at` | TEXT | NOT NULL | 实际发生时间 |
| `created_at` | TEXT | NOT NULL | 创建时间 |
| `updated_at` | TEXT | NOT NULL | 更新时间 |

```sql
CREATE TABLE transactions (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,

    type TEXT NOT NULL CHECK (type IN ('income', 'expense')),
    amount INTEGER NOT NULL CHECK (amount > 0),
    category TEXT,
    description TEXT,

    occurred_at TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,

    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
);
```

金额示例：

| 实际金额 | 数据库存储值 |
|---:|---:|
| ¥18.50 | `1850` |
| ¥12.00 | `1200` |
| ¥5000.00 | `500000` |

`amount` 始终存正数，收入或支出由 `type` 表示。

### 2.6 `notes` — 笔记

第一版只提供笔记的增删改查，不加入分类、标签、关联关系、树形结构或向量检索。

| 字段 | 类型 | 约束 | 说明 |
|---|---|---|---|
| `id` | TEXT | PRIMARY KEY | 笔记 ID，UUID |
| `user_id` | TEXT | NOT NULL | 所属用户 |
| `title` | TEXT | NULL | 标题 |
| `content` | TEXT | NOT NULL | 正文 |
| `created_at` | TEXT | NOT NULL | 创建时间 |
| `updated_at` | TEXT | NOT NULL | 更新时间 |

```sql
CREATE TABLE notes (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,

    title TEXT,
    content TEXT NOT NULL,

    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,

    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
);
```

### 2.7 `schema_migrations` — 数据库版本

记录已经执行过的数据库迁移。它由 paadm 自身维护，不属于用户业务数据。

```sql
CREATE TABLE schema_migrations (
    version INTEGER PRIMARY KEY,
    applied_at TEXT NOT NULL
);
```

---

## 3. 索引

为常用查询建立索引。第一版建议使用以下索引，后续根据实际查询和 `EXPLAIN QUERY PLAN` 的结果调整。

```sql
CREATE INDEX idx_items_user_status
    ON items(user_id, status);

CREATE INDEX idx_items_user_start
    ON items(user_id, start_at);

CREATE INDEX idx_items_user_due
    ON items(user_id, due_at);

CREATE INDEX idx_reminders_user_status_time
    ON reminders(user_id, status, remind_at);

CREATE INDEX idx_reminders_user_item
    ON reminders(user_id, item_id);

CREATE INDEX idx_transactions_user_time
    ON transactions(user_id, occurred_at);

CREATE INDEX idx_transactions_user_category
    ON transactions(user_id, category);

CREATE INDEX idx_notes_user_created
    ON notes(user_id, created_at);
```

---

## 4. 数据隔离与删除规则

### 4.1 用户数据隔离

所有业务表都包含 `user_id`。所有读取、修改和删除操作都必须同时指定当前用户 ID 和目标记录 ID。

例如：

```sql
SELECT *
FROM items
WHERE user_id = ? AND id = ?;
```

不要只根据 `id` 查询业务记录，再假设它一定属于当前用户。

对于 `reminders.item_id`，使用 `(user_id, item_id)` 复合外键，确保提醒不能关联到其他用户的事项。

### 4.2 删除规则

- 删除用户：业务数据通过外键 `ON DELETE CASCADE` 一并删除。
- 删除事项：先将关联提醒的 `item_id` 置为 `NULL`，再删除事项；两步必须处于同一个事务。
- 删除提醒：直接删除对应提醒。
- 删除记账记录：直接删除对应流水。
- 删除笔记：直接删除对应笔记。

### 4.3 修改规则

- 修改任何业务记录时更新 `updated_at`。
- 将事项状态改为 `completed` 时，由应用层设置 `completed_at`。
- 事项重新变为未完成时，由应用层按统一规则清空 `completed_at`。
- 取消记录使用状态字段，不等同于物理删除。

---

## 5. SQLite 初始化注意事项

每个 SQLite 连接都应启用外键约束：

```sql
PRAGMA foreign_keys = ON;
```

建议设置合理的忙等待时间：

```sql
PRAGMA busy_timeout = 5000;
```

迁移、创建事项及其提醒、删除事项及解除提醒关联等多步操作，应使用事务保证原子性。

---

## 6. 第一版明确不实现的内容

为避免数据层过早复杂化，第一版不实现：

- 周期规则表、RRULE 或无限周期实例。
- 周期日程自动续期。
- 日程单次实例之外的系列管理。
- QQ、微信等渠道间的身份绑定和数据互通。
- 多币种、账户、银行卡和转账。
- 独立分类管理或多级分类。
- 笔记分类、标签、笔记关联和树形记忆。
- 提醒的定时调度和消息推送。
- 复杂的权限系统。

这些能力只有在实际需求出现时再增加。

## 7. 第一版表结构清单

最终第一版共 **6 张表**：

1. `users`
2. `items`
3. `reminders`
4. `transactions`
5. `notes`
6. `schema_migrations`

核心原则：paadm 负责结构化数据的校验、业务规则、CRUD、事务和持久化；Agent 负责自然语言理解、意图判断、时间解析和必要的追问。数据库只保存已经被明确解析并确认的结构化结果。
