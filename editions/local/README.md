# local 底座 — 离线单机

`local` 底座把任务状态、路径锁和证据全部留在本机，不依赖任何托管平台。核心契约仍以根目录 [AGENTS.md](../../AGENTS.md) 与 [AGENT-ORCHESTRATION.md](../../AGENT-ORCHESTRATION.md) 为准，本文件只说明底座操作方式。

## 适用场景

- 一位操作者、一台机器完成开发与审查。
- 需要完全离线，或所在环境不允许连接 Issue / PR / CI 服务。
- 项目规模较小、任务数量有限，人工清理分支和 Worktree 可接受。
- 只需要本地 Git 与 Worktree，不需要公开可核验的协作记录。

## 任务载体

- `.orchestration/state.sqlite3` 是唯一任务状态源，由 `scripts/orchestrator.py` 读写。
- 事件与状态变更记录在同一数据库中；不要手工 `DELETE` / `UPDATE` 绕过 CLI。
- `init` 会把 `.orchestration/` 加入当前克隆的 `.git/info/exclude`；没有 Git 时需自行加入 `.gitignore`。
- 备份：`python scripts/orchestrator.py backup`，阶段结束与每日收尾各执行一次。

## 锁机制

- 原子路径锁：`task dispatch <ID>` 在 SQLite `BEGIN IMMEDIATE` 事务中一次性检查并占用全部允许路径。
- 路径相同或父子重叠即冲突；多路径任务任一冲突则整体失败，不产生半套锁。
- 只有 `VERIFIED` / `CANCELED` 自动释放；失败、超时、状态不明保持锁定，需显式 `task unlock` 并记录理由。

## 命令入口

```bash
python scripts/orchestrator.py init
python scripts/orchestrator.py task list
python scripts/orchestrator.py task add --id TASK-0001 --title "示例任务" --paths src/ --acceptance "验收条目"
python scripts/orchestrator.py task ready TASK-0001
python scripts/orchestrator.py task dispatch TASK-0001
python scripts/orchestrator.py task status TASK-0001 REVIEW_READY --head-sha <sha>
python scripts/orchestrator.py backup
```

`python` 指代当前环境的 Python 3 解释器；本套件不引入第三方依赖。完整流程见本目录 [MASTER.md](MASTER.md) 与 [START-EXECUTION.md](START-EXECUTION.md)。

## 证据

- 本地命令的完整命令、退出码与必要输出。
- 当前 head SHA，以及 `git status --short` / `git diff --name-only`。
- 任务库事件记录与备份位置。
- 证据必须对应当前 head；合并后在新主分支上重新运行并记录。

## 已知边界

- **状态只在本机**：换机器、换克隆或任务库丢失后无法自动恢复；必须依赖备份。
- **跨机协作需迁移**：多人或跨地点协作应迁移到 `github` 底座，迁移是单向的，按 `AGENT-ORCHESTRATION.md` 的“底座切换”执行。
- **没有平台级 CI**：检查是否执行完全依赖执行者与 Master 的真实运行，无法用平台强制。
- **Worktree / 分支需人工清理**：`VERIFIED` 不会自动删除分支或 Worktree，长期项目需要定期清理，清理失败不影响已验证状态。
- **无公开可核验记录**：外部人员无法独立核对审查过程，只适合内部或单人项目。
