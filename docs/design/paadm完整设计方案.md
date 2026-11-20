# paadm 完整设计方案

- 文档版本：v1.0
- 项目定位：PAA 的本地个人事务数据管理 CLI
- 实现语言：Rust
- 数据存储：SQLite
- 主要调用方：PAA Agent
- 次要调用方：用户本人通过终端直接使用
- 设计原则：职责单一、接口稳定、可独立测试、先满足真实需求，不提前引入服务化复杂度

---

# 1. 项目定位

## 1.1 paadm 是什么

`paadm` 是 PAA 的数据管理入口，也是一个可以独立使用的命令行工具。

它负责把上层传入的明确操作，转化为经过校验的业务操作，并将结果持久化到 SQLite。

例如：

- 创建一条待办。
- 查询某一天的日程。
- 将一条待办标记为完成。
- 创建一条提醒记录。
- 记录一笔支出。
- 搜索此前保存的想法。

paadm 不负责理解用户的自然语言，也不负责决定用户真正想做什么。它接收的是已经明确表达出来的结构化操作。

## 1.2 paadm 不是什么

paadm 不是：

- 一个 AI Agent。
- 一个自然语言解析器。
- 一个聊天机器人。
- 一个消息平台适配器。
- 一个提醒推送服务。
- 一个通用数据库管理客户端。
- 一个需要长期运行的后台服务。
- 一个多用户 SaaS 后端。

它不应该因为某个上层功能需要，就不断增加与数据管理无关的职责。

## 1.3 核心定位

> paadm 是 PAA 的个人事务数据管理层。它提供稳定、明确、可验证的命令接口，负责业务数据的校验、增删改查、关联约束和持久化。

---

# 2. 系统整体架构与职责边界

## 2.1 系统关系

```text
用户
 │
 ▼
消息入口（QQ / 微信 / 其他入口）
 │
 ▼
Agent
 │
 │  理解意图、组织参数、决定调用哪个工具
 │
 ▼
paadm CLI（Rust）
 │
 ├── CLI 参数解析
 ├── 输入校验
 ├── 业务规则
 ├── 数据查询与修改
 ├── 事务管理
 └── JSON / 人类可读结果
 │
 ▼
SQLite（paa.db）
```

数据返回方向：

```text
SQLite
  │
  ▼
paadm
  │  返回成功结果或结构化错误
  ▼
Agent
  │  将执行结果转述给用户
  ▼
用户
```

## 2.2 各层职责表

| 层级 | 负责什么 | 不负责什么 |
|---|---|---|
| 消息入口 | 接收和发送平台消息 | 业务数据规则、数据库操作 |
| Agent | 理解自然语言、识别意图、收集缺失信息、选择工具、组织回复 | 直接读写 SQLite、伪造执行结果 |
| paadm CLI | 提供命令接口、参数解析、结构化响应 | 自然语言理解、聊天、消息收发 |
| paadm Service | 执行业务规则、校验操作合法性、协调多个数据操作 | 调用 LLM、主动联系用户 |
| paadm Repository | 执行数据库读写 | 决定用户意图、组织自然语言回复 |
| SQLite | 数据持久化、索引、事务、约束 | 业务意图识别、提醒推送 |

## 2.3 最重要的边界原则

### 原则 A：Agent 决定“做什么”，paadm 保证“怎么正确地做”

例如用户说：

> 明天下午三点提醒我交数学作业。

Agent 负责：

1. 理解用户想创建提醒。
2. 识别提醒内容。
3. 将“明天下午三点”解析成明确时间。
4. 如果信息不足，向用户追问。
5. 生成 paadm 所需的结构化参数。

paadm 负责：

1. 检查必要字段是否存在。
2. 检查时间格式是否合法。
3. 检查关联的待办是否存在（如果提供了关联 ID）。
4. 创建提醒记录。
5. 将结果写入 SQLite。
6. 返回实际执行结果。

paadm 不应接收“明天下午三点”这样的自然语言时间并自行猜测含义。

### 原则 B：Agent 不直接访问业务数据库

Agent 只能通过 paadm 的公开接口操作业务数据。

不允许 Agent：

- 自己拼 SQL。
- 直接修改 `paa.db`。
- 依赖 SQLite 表结构实现业务功能。
- 绕过 paadm 的业务校验。

这样未来替换 Agent、消息入口或数据库实现时，业务边界仍然稳定。

### 原则 C：paadm 不负责主动调度和推送

paadm 可以保存提醒的时间、内容、重复规则和状态，但不负责：

- 常驻等待提醒时间。
- 到时间主动唤醒 Agent。
- 发送 QQ 或微信消息。
- 管理消息发送失败后的重试。
- 维护后台定时任务。

这些属于未来的 Scheduler / Notification 层。

### 原则 D：数据库是持久化手段，不是业务接口

上层只依赖 paadm 的命令和 JSON 契约，不依赖具体 SQL、表名或 SQLite 文件结构。

---

# 3. 第一版功能范围

## 3.1 纳入第一版

| 领域 | 功能 | 第一版支持 |
|---|---|---|
| Task | 待办 | 创建、查询、修改、完成、取消、删除 |
| Schedule | 日程 | 创建、查询、修改、删除 |
| Reminder | 提醒记录 | 创建、查询、修改、取消、删除 |
| Transaction | 收支记录 | 创建、查询、修改、删除、基础汇总 |
| Note | 想法/临时记录 | 创建、查询、修改、删除 |
| Storage | 数据存储 | 初始化、迁移、事务、备份策略 |
| CLI | 命令行 | 人类可读模式、JSON 模式、错误码 |
| Agent Interface | Agent 调用接口 | 参数输入、JSON 输出、退出码 |

## 3.2 暂不纳入第一版

- 自然语言解析。
- LLM 调用。
- 对话历史与长期记忆。
- 主动提醒调度。
- QQ、微信等平台接入。
- 多用户和多租户。
- 用户权限系统。
- 多设备实时同步。
- 云端数据库。
- 复杂周期性日程计算。
- 语义搜索和向量数据库。
- 自动推断并修改用户数据。
- 复杂报表和可视化。
- 插件系统。
- 远程 HTTP API。

这些功能不是永远不做，而是目前不属于 paadm 的核心职责，或不属于第一版交付范围。

---

# 4. 领域模型设计

第一版使用五类核心业务实体。

## 4.1 Task：待办事项

用途：记录用户需要完成的事情。

建议字段：

| 字段 | 类型 | 必需 | 说明 |
|---|---|---:|---|
| id | TEXT | 是 | 稳定唯一 ID |
| title | TEXT | 是 | 待办标题 |
| description | TEXT | 否 | 详细描述 |
| status | TEXT | 是 | pending / completed / cancelled |
| priority | TEXT | 否 | low / medium / high |
| due_at | TEXT | 否 | 截止时间，ISO 8601 |
| created_at | TEXT | 是 | 创建时间 |
| updated_at | TEXT | 是 | 更新时间 |
| completed_at | TEXT | 否 | 完成时间 |

规则：

- `title` 去除首尾空白后不能为空。
- 新建时默认 `status=pending`。
- 完成时设置 `completed_at`。
- 取消与完成是不同状态。
- 修改任务时，不得意外覆盖未提供的字段。
- 第一版不支持复杂子任务、依赖关系和任务看板。

## 4.2 Schedule：日程

用途：记录有明确时间安排的事件。

建议字段：

| 字段 | 类型 | 必需 | 说明 |
|---|---|---:|---|
| id | TEXT | 是 | 唯一 ID |
| title | TEXT | 是 | 日程标题 |
| description | TEXT | 否 | 描述 |
| start_at | TEXT | 是 | 开始时间 |
| end_at | TEXT | 否 | 结束时间 |
| location | TEXT | 否 | 地点 |
| created_at | TEXT | 是 | 创建时间 |
| updated_at | TEXT | 是 | 更新时间 |

规则：

- `start_at` 必须合法。
- 如果提供 `end_at`，必须晚于 `start_at`。
- 第一版不自动判断日程冲突，不擅自拒绝时间重叠。
- 第一版不计算复杂周期性日程。
- 查询某一天时，按明确的时间区间规则筛选与该日相交的日程。

## 4.3 Reminder：提醒记录

用途：保存用户要求在某个时间收到提醒的信息。

建议字段：

| 字段 | 类型 | 必需 | 说明 |
|---|---|---:|---|
| id | TEXT | 是 | 唯一 ID |
| content | TEXT | 是 | 提醒内容 |
| remind_at | TEXT | 是 | 提醒时间 |
| repeat_rule | TEXT | 否 | 重复规则 |
| status | TEXT | 是 | active / cancelled |
| task_id | TEXT | 否 | 关联待办 ID |
| created_at | TEXT | 是 | 创建时间 |
| updated_at | TEXT | 是 | 更新时间 |

规则：

- 提醒记录不等于提醒已经发送。
- paadm 只管理提醒数据，不负责触发提醒。
- `task_id` 如果存在，必须引用有效待办。
- 第一版可以保存简单重复规则，但不负责展开未来每次触发时间。
- 取消提醒不等于删除提醒记录。
- 提醒发送状态、发送历史和重试记录暂不属于第一版。

## 4.4 Transaction：收支记录

用途：记录收入和支出。

建议字段：

| 字段 | 类型 | 必需 | 说明 |
|---|---|---:|---|
| id | TEXT | 是 | 唯一 ID |
| type | TEXT | 是 | income / expense |
| amount_minor | INTEGER | 是 | 最小货币单位的整数金额 |
| currency | TEXT | 是 | 货币代码，如 CNY |
| category | TEXT | 否 | 类别 |
| description | TEXT | 否 | 描述 |
| occurred_at | TEXT | 是 | 实际发生时间 |
| created_at | TEXT | 是 | 创建时间 |
| updated_at | TEXT | 是 | 更新时间 |

规则：

- 金额使用整数最小货币单位，避免浮点误差。
- `type` 只允许 `income` 和 `expense`。
- 金额必须大于零，收入/支出由 type 区分。
- 不同币种不能直接相加。
- 第一版只做基础分类和汇总，不做预算、账户、转账、投资或复杂财务分析。

例如：18.50 CNY 应存储为 `amount_minor=1850`、`currency=CNY`。

## 4.5 Note：想法与临时记录

用途：快速保存用户暂时不需要进一步处理的信息。

建议字段：

| 字段 | 类型 | 必需 | 说明 |
|---|---|---:|---|
| id | TEXT | 是 | 唯一 ID |
| title | TEXT | 否 | 标题 |
| content | TEXT | 是 | 原始内容 |
| created_at | TEXT | 是 | 创建时间 |
| updated_at | TEXT | 是 | 更新时间 |

规则：

- 保留用户原始内容，不擅自改写。
- 第一版不做自动总结、语义分类和树形记忆。
- 第一版支持简单关键词检索即可。
- Note 不等于聊天记录，不负责保存完整对话。

---

# 5. 数据库设计

## 5.1 选择 SQLite 的原因

第一版使用 SQLite 单文件数据库：

- 无需独立数据库服务。
- 无需 Docker 或数据库服务器。
- 支持事务、索引、约束和 SQL 查询。
- 适合单用户个人事务管理。
- 未来可以通过迁移工具导出到其他数据库。

建议文件布局：

```text
paa-data/
├── paa.db
├── config.json
└── backups/
```

数据库路径应通过配置或环境变量指定，不应依赖当前工作目录。

建议支持：

- `PAADM_DATA_DIR`：指定数据目录。
- `PAADM_DB_PATH`：直接指定数据库文件路径。
- 未指定时使用操作系统对应的用户数据目录。

测试时必须能够指定临时数据库路径。

## 5.2 数据表

第一版至少包含：

```text
tasks
schedules
reminders
transactions
notes
schema_migrations
```

建议为常用查询建立索引：

- tasks(status)
- tasks(due_at)
- schedules(start_at)
- reminders(remind_at, status)
- transactions(occurred_at)
- transactions(type, category, occurred_at)
- notes(created_at)

索引应根据实际查询需求建立，不要提前大量创建。

## 5.3 关联与外键

第一版只建立必要关联：

```text
tasks 1 ───── N reminders
```

一个待办可以关联多条提醒；一条提醒最多关联一个待办。

建议启用 SQLite 外键约束。

删除待办时，第一版建议采用 `ON DELETE SET NULL`：保留提醒记录，但解除其任务关联。这样不会因为删除待办而意外删除用户设置的提醒。

其他实体之间暂不建立不必要的关联。

## 5.4 时间处理

统一采用 ISO 8601 时间字符串，并保留时区偏移。

示例：

```text
2026-11-17T15:00:00+08:00
```

约定：

- Agent 负责把自然语言时间解析为明确时间。
- paadm 负责校验时间格式。
- 数据库保存可无歧义解析的时间值。
- 查询时间范围时使用统一的时间比较规则。
- 展示时区由上层决定，或使用用户配置的默认时区。
- 不得把没有时区的本地时间悄悄当成 UTC。

如果需要统一存 UTC，必须在输入和输出边界明确转换，并保留用户原始时区语义所需的信息。

## 5.5 ID 设计

使用稳定的唯一 ID，不使用数组下标或数据库行号作为外部 ID。

建议使用 UUID（如 UUID v4 或 v7），以字符串形式返回给 Agent。

第一版不要求用户记忆 ID；ID 主要用于 Agent 精确引用记录。

## 5.6 数据库迁移

使用 `schema_migrations` 记录已执行迁移版本。

要求：

- 首次启动自动初始化。
- 已执行的迁移不得重复执行。
- 新版本启动时按顺序执行未完成迁移。
- 迁移应在事务中执行。
- 迁移失败时不得标记为成功。
- 不得静默删除或覆盖旧数据。

数据库结构变化必须通过迁移完成，不允许运行时随意修改表结构。

---

# 6. paadm CLI 设计

## 6.1 命令树

建议采用统一的领域命令结构：

```text
paadm
├── init
├── version
├── task
│   ├── create
│   ├── get
│   ├── list
│   ├── update
│   ├── complete
│   ├── cancel
│   └── delete
├── schedule
│   ├── create
│   ├── get
│   ├── list
│   ├── update
│   └── delete
├── reminder
│   ├── create
│   ├── get
│   ├── list
│   ├── update
│   ├── cancel
│   └── delete
├── transaction
│   ├── create
│   ├── get
│   ├── list
│   ├── update
│   ├── delete
│   └── summary
└── note
    ├── create
    ├── get
    ├── list
    ├── update
    └── delete
```

命令名应保持一致，避免同一种操作在不同领域出现不必要的命名差异。

## 6.2 命令示例

创建待办：

```bash
paadm task create \
  --title "完成数学作业" \
  --due-at "2026-11-17T23:59:00+08:00" \
  --priority high \
  --json
```

查询待办：

```bash
paadm task list --status pending --json
```

查询单条：

```bash
paadm task get --id "<task-id>" --json
```

完成待办：

```bash
paadm task complete --id "<task-id>" --json
```

创建日程：

```bash
paadm schedule create \
  --title "组会" \
  --start-at "2026-11-20T10:00:00+08:00" \
  --end-at "2026-11-20T11:00:00+08:00" \
  --json
```

记录支出：

```bash
paadm transaction create \
  --type expense \
  --amount-minor 1850 \
  --currency CNY \
  --category food \
  --description "午饭" \
  --json
```

## 6.3 人类模式与 JSON 模式

paadm 有两种输出模式：

### 人类模式

供用户直接在终端使用，输出简洁易读的信息。

### JSON 模式

供 Agent 调用，输出稳定、机器可读的结果。

JSON 模式约定：

- stdout 只输出一个 JSON 对象。
- 日志、调试信息写入 stderr。
- 成功时退出码为 0。
- 失败时退出码非 0。
- JSON 字段结构保持稳定。
- Agent 依赖错误码，而不是解析自然语言错误描述。

## 6.4 JSON 响应格式

成功：

```json
{
  "success": true,
  "data": {
    "id": "..."
  },
  "error": null
}
```

失败：

```json
{
  "success": false,
  "data": null,
  "error": {
    "code": "TASK_NOT_FOUND",
    "message": "Task does not exist"
  }
}
```

列表查询：

```json
{
  "success": true,
  "data": {
    "items": [],
    "count": 0
  },
  "error": null
}
```

约定：

- `success` 表示本次命令是否成功执行。
- `data` 表示业务结果。
- `error.code` 是稳定机器接口。
- `error.message` 主要用于诊断和展示。
- 空列表是成功，不是错误。
- 不返回 SQL、堆栈或内部文件路径等不必要的内部细节。

## 6.5 输入方式

第一版优先支持 CLI 参数。

对于长文本、复杂内容，可以增加 stdin JSON 输入，例如：

```bash
echo '{"title":"完成数学作业"}' |
  paadm task create --input-json -
```

不建议一开始同时实现多套复杂输入协议。先固定一种清晰的 stdin 约定即可。

---

# 7. Rust 内部架构

## 7.1 推荐目录结构

```text
paadm/
├── Cargo.toml
├── src/
│   ├── main.rs
│   ├── cli/
│   │   ├── mod.rs
│   │   ├── task.rs
│   │   ├── schedule.rs
│   │   ├── reminder.rs
│   │   ├── transaction.rs
│   │   └── note.rs
│   │
│   ├── domain/
│   │   ├── mod.rs
│   │   ├── task.rs
│   │   ├── schedule.rs
│   │   ├── reminder.rs
│   │   ├── transaction.rs
│   │   └── note.rs
│   │
│   ├── services/
│   │   ├── mod.rs
│   │   ├── task.rs
│   │   ├── schedule.rs
│   │   ├── reminder.rs
│   │   ├── transaction.rs
│   │   └── note.rs
│   │
│   ├── repositories/
│   │   ├── mod.rs
│   │   ├── task.rs
│   │   ├── schedule.rs
│   │   ├── reminder.rs
│   │   ├── transaction.rs
│   │   └── note.rs
│   │
│   ├── storage/
│   │   ├── mod.rs
│   │   ├── sqlite.rs
│   │   └── migrations.rs
│   │
│   ├── output/
│   │   ├── mod.rs
│   │   └── response.rs
│   │
│   └── error.rs
│
└── tests/
    ├── cli/
    ├── task/
    ├── schedule/
    └── storage/
```

这是逻辑组织建议，不要求第一天就创建全部文件。

## 7.2 各模块职责

### CLI

- 使用 clap 解析参数。
- 将输入转换为明确的命令参数。
- 调用 Service。
- 根据输出模式打印结果。
- 不直接写 SQL。

### Domain

- 定义业务实体。
- 定义状态和枚举。
- 定义必要的领域约束。
- 不依赖 CLI 和具体数据库实现。

### Service

- 实现业务操作。
- 执行跨字段校验。
- 协调 Repository。
- 管理需要原子执行的多个数据操作。
- 不处理自然语言。

### Repository

- 实现数据查询和写入。
- 使用参数化 SQL。
- 将数据库记录转换为领域对象。
- 不负责自然语言或 CLI 输出。

### Storage

- 打开 SQLite 连接。
- 初始化数据库。
- 执行迁移。
- 设置外键和连接参数。
- 提供事务所需能力。

### Output

- 定义统一 JSON 响应结构。
- 将成功结果和错误转换为 CLI 输出。
- 保持 stdout 纯净。

### Error

- 定义业务错误、存储错误、参数错误。
- 映射为稳定错误码。
- 隔离 SQLite 原始错误信息。

## 7.3 是否一定要使用 Repository Trait？

不必为了架构形式而提前抽象。

如果第一版只有 SQLite，可以先使用清晰的 Repository 结构和普通 Rust 类型。

只有当确实需要替换存储实现、在 Service 单元测试中注入 Mock，或者需要多个 Repository 实现时，再引入 Trait。

不要为了“未来可能切换数据库”提前制造大量抽象。

---

# 8. Agent 与 paadm 的进程交互

## 8.1 第一版采用子进程调用

Agent 通过进程 API 启动 paadm：

```text
Agent
  │
  ├── 创建 Command
  ├── 设置参数
  ├── 启动 paadm
  ├── 捕获 stdout / stderr
  ├── 获取退出码
  └── 解析 JSON
```

第一版不需要：

- HTTP 服务。
- gRPC。
- Unix Socket。
- 常驻 daemon。
- 数据库连接池服务。
- 复杂 IPC 框架。

## 8.2 安全调用约定

- Agent 使用参数数组传递参数。
- 不拼接 Shell 命令字符串。
- 对用户文本正确处理空格和特殊字符。
- 设置合理的进程超时。
- 限制 stdout / stderr 的最大读取量。
- 检查退出码和 JSON 响应是否一致。
- 对异常退出、超时和无法解析的输出返回明确错误。

## 8.3 调用结果的可信边界

Agent 只有在以下条件满足时，才能向用户报告操作成功：

1. paadm 进程正常完成。
2. 退出码为 0。
3. JSON 能正常解析。
4. `success=true`。

如果进程超时或中途退出，Agent 不得直接重试有副作用的命令，因为第一次操作可能已经写入数据库。

对于不确定的创建结果，应先通过业务查询确认是否已创建，再决定是否重试。

## 8.4 为什么第一版不常驻？

每次调用启动一个短生命周期进程：

1. 打开数据库。
2. 执行操作。
3. 输出结果。
4. 退出。

优点：

- 部署简单。
- 生命周期清楚。
- 容易测试。
- 无需管理常驻进程。
- Agent 崩溃不会直接导致数据服务不可用。

如果未来确实遇到启动开销、批量操作效率或长连接需求，再考虑增加常驻模式；不要现在就做。

---

# 9. 并发、事务与数据可靠性

## 9.1 并发模型

第一版定位为单用户本地数据管理。

允许：

- 多个 paadm 进程同时读取。
- 多个进程尝试写入。
- Agent 和用户终端同时使用。

SQLite 同一时刻仍然只有一个写事务能够执行，因此需要：

- 启用 WAL 模式。
- 设置合理的 busy timeout。
- 尽量缩短写事务。
- 不在数据库事务中执行网络请求或等待用户输入。

## 9.2 事务边界

单条记录的创建、修改和删除使用数据库原子操作。

涉及多个数据修改的操作必须放在同一事务中。

例如：

```text
创建一个任务
  +
创建关联提醒
  ↓
同一个数据库事务
```

如果提醒创建失败，整个操作回滚，避免留下只有任务、没有预期提醒的半成品。

但不要为了简单 CRUD 过度使用多层事务封装。

## 9.3 删除策略

第一版区分：

- `complete`：任务已完成。
- `cancel`：任务不再需要执行。
- `delete`：从业务数据中删除。

删除前应明确目标 ID。

提醒取消与删除也要区分：

- 取消：保留记录，只改变状态。
- 删除：移除记录。

关联删除规则必须明确，并通过外键约束实现，而不是依赖调用方记住所有关联关系。

## 9.4 备份

第一版提供简单可靠的备份方案即可：

- 支持手动备份。
- 备份时使用 SQLite 支持的安全备份方式，不能简单地在数据库写入期间随意复制单个 DB 文件。
- 恢复前保留当前文件副本。
- 恢复后执行完整性检查。

自动备份计划可以由未来的运维或调度层负责，不必让 paadm 自己成为后台任务管理器。

---

# 10. 错误体系

建议第一版至少区分以下错误类别：

| 错误码示例 | 含义 |
|---|---|
| INVALID_ARGUMENT | 参数不合法 |
| INVALID_INPUT | 输入数据无法解析 |
| TASK_NOT_FOUND | 待办不存在 |
| SCHEDULE_NOT_FOUND | 日程不存在 |
| REMINDER_NOT_FOUND | 提醒不存在 |
| TRANSACTION_NOT_FOUND | 账目不存在 |
| NOTE_NOT_FOUND | 记录不存在 |
| DUPLICATE_ID | ID 冲突 |
| DATABASE_BUSY | 数据库暂时被占用 |
| DATABASE_ERROR | 数据库操作失败 |
| MIGRATION_FAILED | 数据库迁移失败 |
| INTERNAL_ERROR | 未预期内部错误 |

要求：

- 错误码稳定。
- 错误信息可读。
- 不向 Agent 暴露不必要的内部堆栈。
- 不将失败伪装成成功。
- 错误响应本身也必须符合 JSON 协议。

---

# 11. 安全边界

第一版虽然是本地单用户工具，也需要基本安全约束：

1. 所有 SQL 使用参数绑定，禁止字符串拼接用户输入。
2. 数据库路径来自明确配置，不允许用户输入意外覆盖系统文件。
3. 不接受 Agent 传入任意 SQL。
4. 不提供任意文件读写命令。
5. 不在 stdout 暴露调试信息或敏感路径。
6. 对输入长度设置合理上限。
7. 数据库文件和备份文件使用适当的本地文件权限。
8. 不把 Agent 提供的字段直接当作 SQL 标识符或表名。
9. 如果以后增加远程调用，需要另行设计身份认证、授权和审计；本地 CLI 的安全假设不能直接用于远程服务。

---

# 12. 技术选型

建议第一版保持依赖精简：

| 领域 | 建议 | 用途 |
|---|---|---|
| CLI | clap | 命令行解析 |
| JSON | serde + serde_json | 输入输出序列化 |
| SQLite | rusqlite | SQLite 访问 |
| ID | uuid | 唯一标识 |
| 时间 | time 或 chrono | 时间解析和格式处理 |
| 错误处理 | thiserror | 定义错误类型 |
| 测试数据库 | tempfile | 临时目录和数据库 |
| CLI 集成测试 | assert_cmd / predicates | 测试真实命令行为 |

不需要为了 paadm 引入：

- Tokio（第一版没有异步需求）。
- Web 框架。
- ORM（第一版 SQL 规模小，直接 SQL 更透明）。
- Redis。
- MySQL / PostgreSQL。
- 消息队列。
- 微服务框架。
- 容器编排系统。

依赖版本以项目创建时稳定兼容的版本为准。

---

# 13. 测试策略与 TDD 流程

## 13.1 测试层次

### 单元测试

测试：

- 领域状态转换。
- 字段校验。
- 时间区间规则。
- 金额规则。
- 错误码映射。

### Repository 测试

使用临时 SQLite 数据库，测试：

- 创建和查询。
- 修改和删除。
- 索引相关查询。
- 外键约束。
- 事务回滚。

### Service 测试

测试：

- 业务规则。
- 跨 Repository 操作。
- 关联处理。
- 错误传播。

### CLI 集成测试

启动真实 paadm 进程，测试：

- 命令参数。
- JSON 输出。
- 退出码。
- 数据持久化。
- 错误处理。

## 13.2 TDD 实施顺序

### Milestone 1：最小可运行闭环

- `paadm --help`
- 数据库首次初始化。
- 创建最小 Task。
- 按 ID 查询 Task。
- 空列表查询。
- 进程重启后数据仍存在。
- JSON 成功与失败响应。

### Milestone 2：Task 完整闭环

- 创建。
- 查询。
- 修改。
- 完成。
- 取消。
- 删除。
- 输入校验和错误处理。

### Milestone 3：数据库可靠性

- 事务。
- 外键。
- WAL。
- 并发读写。
- 迁移。
- 备份恢复。

### Milestone 4：其他业务实体

按 Schedule、Reminder、Transaction、Note 的顺序逐个实现。

### Milestone 5：Agent 调用接入

- 参数传递。
- JSON 协议。
- 错误码。
- 退出码。
- 超时与异常退出处理。

每个 Milestone 都应有可运行、可验证的交付结果。

---

# 14. 明确的设计边界清单

这一节是本方案最重要的部分。新增需求时，应先判断它属于哪一层。

| 需求 | 应属于 | paadm 是否负责 |
|---|---|---:|
| 用户说“明天提醒我” | Agent | 否 |
| 将“明天”解析成具体日期 | Agent | 否 |
| 判断用户到底想创建提醒还是日程 | Agent | 否 |
| 用户缺少必要时间时追问 | Agent | 否 |
| 根据明确参数创建提醒记录 | paadm | 是 |
| 校验提醒时间格式 | paadm | 是 |
| 保存提醒的重复规则 | paadm | 是 |
| 计算复杂重复提醒的每次触发时间 | Scheduler | 否 |
| 到时间唤醒任务 | Scheduler | 否 |
| 发送 QQ / 微信消息 | 消息入口 / Notification | 否 |
| 发送失败后重试 | Notification | 否 |
| 将“已经完成了”映射到某个任务 | Agent | 否 |
| 将指定任务状态更新为 completed | paadm | 是 |
| 直接执行 SQL 查询 | paadm 内部 Repository | 是 |
| 让 Agent 自己拼 SQL | 不允许 | 否 |
| 保存用户完整聊天记录 | 对话系统 | 否 |
| 保存用户明确要求记录的想法 | paadm | 是 |
| 自动总结和构建长期记忆 | 记忆系统 | 否 |
| 计算本月支出总额 | paadm | 是 |
| 分析用户消费习惯并给建议 | Agent / 分析层 | 否 |
| 多平台账号接入 | 消息入口 | 否 |
| 多用户身份认证 | 未来服务端 | 第一版否 |

## 14.1 一个判断新需求归属的简单方法

问三个问题：

**问题一：它是否已经是一个明确的结构化数据操作？**

例如：

- 创建待办。
- 修改提醒时间。
- 查询本月支出。

如果是，通常属于 paadm。

**问题二：它是否需要理解用户表达、上下文或意图？**

例如：

- “那个任务”指什么？
- “晚点”具体是几点？
- 用户是在取消日程还是取消提醒？

如果是，属于 Agent。

**问题三：它是否需要在未来某个时间主动执行，或者与外部平台通信？**

例如：

- 到时间发送提醒。
- 主动推送 QQ 消息。
- 消息发送失败后重试。

如果是，属于 Scheduler / Notification，而不是 paadm。

## 14.2 防止 paadm 膨胀的规则

以下设计应当明确禁止或推迟：

- 不在 paadm 中接入 LLM。
- 不在 paadm 中实现自然语言分词和日期理解。
- 不让 paadm 管理聊天会话。
- 不让 paadm 直接发送消息。
- 不让 paadm 常驻等待提醒。
- 不让 Agent 绕过 paadm 修改数据库。
- 不为尚未出现的多用户需求提前实现权限体系。
- 不为“可能切换数据库”提前设计复杂插件架构。
- 不为了架构完整而引入微服务、消息队列和分布式锁。

---

# 15. 第一版验收标准

paadm v1.0 至少满足以下条件：

## 基础能力

- [ ] 可以独立安装或运行。
- [ ] 可以显示帮助和版本。
- [ ] 可以指定数据库路径。
- [ ] 首次启动能初始化数据库。
- [ ] 可以执行数据库迁移。

## 业务能力

- [ ] Task 基本 CRUD 完整。
- [ ] Schedule 基本 CRUD 完整。
- [ ] Reminder 基本 CRUD 完整。
- [ ] Transaction 基本 CRUD 和基础汇总完整。
- [ ] Note 基本 CRUD 完整。

## 可靠性

- [ ] 参数校验有效。
- [ ] 错误码稳定。
- [ ] SQL 参数化。
- [ ] 事务行为正确。
- [ ] 数据重启后保留。
- [ ] 外键约束有效。
- [ ] 并发访问不会静默丢失成功写入的数据。
- [ ] 数据库迁移不会破坏旧数据。

## Agent 接口

- [ ] JSON 输出符合统一结构。
- [ ] stdout 不混入日志。
- [ ] 退出码符合约定。
- [ ] 错误可被 Agent 稳定识别。
- [ ] Agent 不需要访问 SQLite 文件。

## 边界

- [ ] paadm 不包含 LLM 调用。
- [ ] paadm 不包含自然语言理解。
- [ ] paadm 不包含消息平台适配。
- [ ] paadm 不负责主动提醒推送。
- [ ] paadm 可独立于 Agent 使用和测试。

---

# 16. 推荐开发路线

不要按目录结构从上到下开发，也不要一次性实现全部领域。

建议按以下路线：

```text
第一步：CLI 骨架
    ↓
第二步：SQLite 初始化与迁移
    ↓
第三步：Task Domain / Service / Repository
    ↓
第四步：Task CLI 与 JSON 响应
    ↓
第五步：Task 集成测试
    ↓
第六步：Schedule
    ↓
第七步：Reminder
    ↓
第八步：Transaction
    ↓
第九步：Note
    ↓
第十步：Agent 调用适配
```

每一步都遵循：

```text
写测试
  ↓
确认测试失败（Red）
  ↓
实现最少代码
  ↓
测试通过（Green）
  ↓
重构（Refactor）
  ↓
运行回归测试
```

## 最终设计结论

第一版 paadm 应当是一个：

- Rust 编写的独立 CLI。
- 通过子进程方式供 Agent 调用。
- 以 JSON 作为机器交互协议。
- 以 SQLite 作为持久化存储。
- 采用简单分层结构组织业务代码。
- 专注个人事务数据管理。
- 不包含 AI、聊天、消息推送和后台调度职责。

**paadm 的价值不在于它拥有多少功能，而在于它为 PAA 提供一个稳定、可信、可独立使用的数据操作边界。**

先把这个边界做好，再让 Agent 和其他入口围绕它扩展。
