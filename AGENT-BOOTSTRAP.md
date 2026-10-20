# AGENT-BOOTSTRAP.md — 会话自动引导入口

本文件定义每次 Agent 会话开始的确定性入口。无论会话由谁发起、上下文是否丢失，都要按本流程恢复状态并自动路由，不依赖聊天记忆或人工提醒。

## 1. 引导流程

### 第一步：读取权威文件与底座

按顺序读取：

1. `ORCHESTRATION-PROJECT.md` — 读取 `edition`，取值必须是 `local` 或 `github`。
2. `AGENTS.md` — 核心协作契约（edition-neutral）。
3. `AGENT-ORCHESTRATION.md` — 状态机、合并门与能力契约。
4. `editions/<edition>/MASTER.md` — 当前底座的 Master 手册。

**底座判定规则**：

- 从 `ORCHESTRATION-PROJECT.md` 的 `edition` 键读取取值，允许值只有 `local`、`github`。
- 文件缺失、`edition` 键缺失、值为空或不在允许集合内：输出明确错误并**停止**，不得猜测、不得按默认值继续、不得自行切换底座。错误信息必须包含缺失项、实际读到的值和建议修复位置。
- 读取成功后，后续所有路由、命令和证据路径都以该底座为准。

### 第二步：恢复状态

用真实输出恢复状态，不从聊天记忆重建。先执行两个底座共用的 Git 检查：

```bash
git status --short
git branch --show-current
git log -1 --oneline
```

再按底座执行任务载体查询：

`local`：

```bash
python scripts/orchestrator.py task list
```

`github`：

```bash
gh issue list --state open --limit 100
gh pr list --state open --limit 100
```

必要时可补充 `git worktree list`、任务库事件查询、Issue 详情 / PR 时间线与最近提交记录。

### 第三步：按状态自动路由

根据第一步与第二步的事实，按顺序匹配下列分支，命中即加载对应提示词并继续；都不命中时落入 d。`<edition>` 指第一步读到的底座。

| # | 条件 | 动作 |
|---|------|------|
| a | `ORCHESTRATION-PROJECT.md` 存在 `FILL_IN` 占位符，或章程缺失 | 加载 `prompts/START-CHARTER.md`（两个底座共享），先补齐章程再进入编排 |
| b | 章程已填写完整，且当前底座没有任何任务（新项目） | 加载 `editions/<edition>/START-MASTER-NEW-PROJECT.md`，进入初始规划 |
| c | 当前底座已有活动任务（`DISPATCHED` / `IN_PROGRESS` / `PR_OPEN` 等），或项目已有推进记录 | 加载 `editions/<edition>/START-MASTER-EXISTING-PROJECT.md`，先汇报当前状态、活动任务与阻塞，再继续调度 |
| d | 当前底座没有任务，且用户本轮未给出目标 | 简要汇报当前状态（底座、分支、工作区、最近提交、章程情况）后等待用户目标，不自行造任务 |

判断以底座载体与 Git 的真实状态为准；多种条件同时命中时，按 a → b → c 优先级处理（例如章程缺失时先补章程，即使已有任务）。

共享提示词固定为 `prompts/START-CHARTER.md` 与 `prompts/START-REVIEWER.md`；其余启动与 Master 提示词一律从 `editions/<edition>/` 读取，不要跨底座加载。

## 2. 引导纪律

- 引导产出的是“状态与路由”汇报，它不替代 Master 的任务汇报模板（结论、用户可感知变化、验收结果、风险与下一步）。
- 不得用聊天记忆代替任务载体状态；所有“有任务 / 有阻塞 / 有活动锁”的结论都必须有命令输出或文件证据支撑。
- 引导本身不改产品代码、不改编排状态；发现的冲突、缺失或异常先登记为任务或标记 `NEEDS_HUMAN_DECISION`，再进入对应提示词流程。
- 底座不一致（例如项目声明 `github` 但只有本地任务库）属于阻断问题：停止派发，报告实际观察与所需决定，不得自行迁移或双写。
