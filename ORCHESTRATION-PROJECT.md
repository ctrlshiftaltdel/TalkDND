# ORCHESTRATION-PROJECT.md — Per-Project Configuration

> 这是 TalkDND 的项目适配层。通用规则留在 `AGENTS.md` 与 `AGENT-ORCHESTRATION.md`，不要把本项目的路径和测试命令写进那些文件。

## 项目身份

- 项目名称：`TalkDND`
- 一句话目标：让 AI 在聊天里按玩家自备的 D&D 规则书主持冒险，并用 combat-sim 结算战斗；受版权保护的书和冒险、以及个人存档，永不进入仓库或公开记录。
- 现有项目 / 新项目：`现有项目`
- 本地主分支：`main`
- 语言 / 框架 / 运行方式：Rust（`tools/combat-sim`）与 Markdown/YAML 规则数据；编排脚本是 Python 3 标准库。玩家在 `LetsPlay/` 里开局。
- 重要设计 / 需求文件：无独立 `docs/PROJECT-CHARTER.md`。玩家说明是根目录 `README.md`；DM 手册是 `LetsPlay/AGENTS.md`；引擎说明是 `tools/combat-sim/README.md` 与 `tools/combat-sim/docs/`。范围、非目标与隐私规则即本文件。

## 范围与安全

- 允许改动路径：`tools/combat-sim/`、`LetsPlay/Rules/combat/`、`LetsPlay/Rules/rulings*.md`、`LetsPlay/AGENTS.md`、根目录与各目录 `README.md`、编排文件（`AGENTS.md`、`AGENT-*.md`、`ORCHESTRATION-PROJECT.md`、`editions/`、`prompts/`、`templates/`、`VERSION.txt`）、`.github/`、`scripts/`。具体任务可以再收窄。
- 永远禁止改动路径 / 文件：受版权保护的规则书与冒险书（`LetsPlay/Rules/2014/` 与 `LetsPlay/Rules/2024/` 除 `README.md` 外，`LetsPlay/Adventures/` 除 `README.md` 外，以及任何版权文本或其改编数据）；`LetsPlay/Saves/` 除 `README.md` 外；`**/dm-secrets.md`；`LetsPlay/Rules/combat/local/`；`LetsPlay/Rules/combat/2024/monsters/scarecrow.yaml`。
- 秘密与隐私文件模式：`.local/`（除 `README.md`）、`**/dm-secrets.md`、`LetsPlay/Saves/`（除 `README.md`）、GitHub 凭据、玩家规则书与冒险书文本。
- 破坏性操作和迁移政策：禁止改写 Git 历史、禁止强推。已有检出里的未跟踪本地书和存档用 `scripts/migrate_local_layout.sh`（默认 dry-run，`--apply` 才移动），不得覆盖已有文件。主持游戏写入 `LetsPlay/Saves/` 豁免开发流程，但那些文件仍然不得提交。
- 新依赖政策：新增依赖（Rust crate、Python 第三方包、GitHub Action 以外的外部服务）必须先得到人工批准。编排脚本保持 Python 标准库。
- 需要人工批准的变化：创建或删除 GitHub 仓库；改写历史或强推；新增依赖；合并到 `main`；规则版本语义变化（`ruleset` 分层、2014/2024 行为差异）。
- combat-sim 源码只读约定：修改 `tools/combat-sim/src` 下的文件之前先 `chmod u+w`，改完并验证之后 `chmod a-w`。只有 `src` 变化时才重新 `cargo build --release`。
- **PRIVACY RULE**: never put save contents, DM secrets, or copyrighted text into Issues, PRs, commit messages or CI logs.

## 验证命令

下列命令已在接入时于本机执行。合并前和合并后都要重跑「合并后必须执行」里的命令。

- 快速测试：`python3 scripts/validate_kit.py` 与 `bash scripts/check_leaks.sh`
- 完整回归：`cargo test --manifest-path tools/combat-sim/Cargo.toml`
- 静态检查 / 类型检查：`python3 -m py_compile scripts/orchestrator.py scripts/validate_kit.py`
- 构建 / 打包：`cargo build --release --manifest-path tools/combat-sim/Cargo.toml`（`tools/bin/combat-sim` 是指向 `tools/combat-sim/target/release/combat-sim` 的符号链接，不把二进制提交进 Git）
- 手工 / UI 冒烟检查：`LetsPlay/bin/combat-sim roll 1d20+5 --dc 15`（可在任意工作目录调用该包装脚本）
- 合并后必须执行：`cargo test --manifest-path tools/combat-sim/Cargo.toml`；`python3 scripts/validate_kit.py`；`bash scripts/check_leaks.sh`

`tools/combat-sim/tests/real_content.rs` 在 `LetsPlay/Saves/2026-10-19-01/combat/campaign.yaml` 不存在时跳过并打印说明。CI 没有版权书，也没有存档。

## 运行配置

本项目的底座以本文件为准，不以 `scripts/orchestrator.py` 的内置缺省为准。

- `edition`：`github`
- 活跃底座是 `github`：任务单在 GitHub Issue，证据在 PR 与 CI。`editions/local/` 必须留在树里，因为 `scripts/validate_kit.py` 要求两套底座文件齐全；不得把 local 任务库当作第二套状态源，也不得双写。不要为本项目运行 `python3 scripts/orchestrator.py init`（该命令会按内置缺省写下 `edition=local` 的本地库）。
- 并发策略：按机器余量动态决定，不设固定上限。同一 worktree 同一时刻只允许一个测试进程。
- 本地私有目录：`.local/`（`.env`、机器信息）。永不入库。

## 本地工作流

- 分支命名：`task/<ID>-<name>`
- Worktree 根目录：`.worktrees/`
- 工作目录要求：一个执行实例一个 Worktree；不共享可写工作区。
- 默认审查修复上限：`retry_cap` 为 `3`。只有项目所有者显式修改才可改变。
- 本地证据保留方式：github 底座的证据在 Issue、PR 与 CI 日志中，并遵守上面的 PRIVACY RULE。`.orchestration/` 不入库。
- 备份策略：以 Git 历史与 GitHub 上的 Issue/PR 为准。不把存档、DM 秘密或版权文本复制进备份或日志。

## 新项目启动产物

本仓库是现有项目。播放布局在 `LetsPlay/`，开发契约在根目录。不另写章程文件。
