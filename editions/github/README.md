# github 底座 — Issue + Draft PR + CI

`github` 底座把任务单放在 Issue、把实现过程放在 PR、把检查交给 CI，适合需要公开可核验证据的协作。核心契约仍以根目录 [AGENTS.md](../../AGENTS.md) 与 [AGENT-ORCHESTRATION.md](../../AGENT-ORCHESTRATION.md) 为准，本文件只说明底座操作方式。

## 适用场景

- 多人、多地点协作，或需要跨机器接手。
- 需要公开可核验的过程证据：谁在何时开工、改了什么、CI 是否通过。
- 项目变大，任务数量与并行度超出单机人工协调能力。
- 需要平台强制的合并门（分支保护、必需检查）。

## 任务载体

- 每个任务一个 Issue，正文固定使用 [ISSUE-TEMPLATE.md](ISSUE-TEMPLATE.md) 的五节：目标 / 验收标准 / 可改路径 / 禁改路径 / 依赖。
- Issue 标签至少表达状态（`status:ready`、`status:in-progress`、`status:review`、`status:blocked`）与类型。
- Issue 正文是任务的唯一权威描述；聊天与 PR 描述不能替代或悄悄修改验收标准。

## 锁机制

- 开工前在 Issue 内写入 [ACTIVE-TASK-REGISTRY.md](ACTIVE-TASK-REGISTRY.md) 的 `ORCH-TASK-REGISTRY` 登记块，声明 `locked_paths`、分支、PR、执行者与时间戳。
- 同时列出所有开放 PR 的变更文件，检查是否与 `locked_paths` 相交；相交即冲突，整个派发失败，不允许“先开工再协调”。
- 路径相同或父子重叠即冲突。活动期间扩大 `locked_paths` 必须在 Issue 评论说明并重新做冲突检查。
- 释放条件与核心契约一致：只有 `VERIFIED` / `CANCELED` 才把登记块标记为释放；失败或状态未知保持锁定并写 `blocked_reason`。

## 状态载体

- 任务状态写入 Issue 登记块与标签；实现状态写入 PR。
- `IN_PROGRESS` 后尽早创建 **Draft PR**，使工作可见；此时状态为 `PR_OPEN`。
- 实现完成、CI 在当前 head 全绿后把 PR 转为 **Ready for review**，状态进入 `REVIEW_READY`。
- PR 时间线、评审评论与 CI 运行记录共同构成状态证据；不要只在聊天里说“已完成”。

## 证据

- CI 运行结果，必须对应当前 head SHA。
- PR 评论中的逐条验收结论（四行格式）。
- PR Diff 与 changed files，用于范围核对。
- 合并后的主分支验证记录；平台合并记录单独保存。

## 已知边界

- **依赖平台可用性**：Issue / PR / CI 服务不可用时无法推进；不得临时改用本地任务库并声称等价。
- **公开仓库需脱敏**：Issue、PR 评论、提交信息与 CI 日志都可能公开；不得写入内部路径、客户数据、凭据或未公开的业务信息。
- **PR 数量大时需度量**：开放 PR 过多会导致冲突与审查积压；需要定期统计开放 PR 数量、存活时间与冲突率，并据此限制并行度。
- **平台检查不等于验收**：CI 绿灯只满足 G4，不替代 G1 的逐条验收。
- **通知噪音**：评论与状态变更会通知所有关注者，避免无意义的批量操作。
