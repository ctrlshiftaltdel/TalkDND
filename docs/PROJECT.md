# docs/PROJECT.md — Per-Project Configuration

> 这是 TalkDND 的项目适配层。通用规则留在 `AGENTS.md` 与 `kit/orchestration.md`，不要把本项目的路径和测试命令写进那些文件。

## 项目身份

- 项目名称：`TalkDND`
- 一句话目标：让 AI 在聊天里按玩家自备的 D&D 规则书主持冒险，并用 combat-sim 结算战斗；受版权保护的书和冒险、以及个人存档，永不进入仓库或公开记录。
- 现有项目 / 新项目：`现有项目`
- 本地主分支：`main`
- 语言 / 框架 / 运行方式：项目自己的程序、工具和脚本一律用 Rust（`tools/combat-sim` 战斗引擎，`tools/talkdnd` 游玩助手（开局摘要、建卡、同伴、存档点），`tools/talkdnd-dev` 开发工具）与 Markdown/YAML 规则数据。玩家在 `LetsPlay/` 里开局。**TECH STACK** 的接受例外只有下面三项，不得再扩大：
  - 作为本项目一部分维护的 AgentOrch Python 套件（只用标准库，经 `uv run --no-project` 运行）：`scripts/` 与 `tests/` 下的套件脚本和测试，包括 `orchestrator.py`、`validate_kit.py`、`check_drift.py`、`check_scope.py`、`report_metrics.py`、`run_delivery_scenarios.py`、`scripts/architecture/`、`scripts/self_improvement/`、`scripts/update_agentorch/`；
  - CI YAML（`.github/workflows/`，含其中的 shell 步骤）；
  - `LetsPlay/tools/pdf2md/`：游玩所需的 Python PDF → Markdown 转换工具。
  所有 Python 一律经 uv 运行（`uv sync` / `uv run`），不用裸 `pip`、`venv` 或 `python3`。套件文档里出现的裸 `python` 在本仓库一律按 `uv run --no-project python` 执行。
- 重要设计 / 需求文件：项目章程是 `docs/PROJECT-CHARTER.md`（决策记录与活章程）；设计目标是 `docs/PLAN.md`。套件入口（`kit/orchestration.md`）判断「章程缺失」时只看 `docs/PROJECT-CHARTER.md`：文件存在且没有占位符，即章程已建立。玩家说明是 `LetsPlay/README.md`；DM 手册是 `LetsPlay/AGENTS.md`（核心，≤150 行，开头是玩家指令表）与按需读的 `LetsPlay/docs/*.md`；引擎说明是 `tools/combat-sim/README.md` 与 `tools/combat-sim/docs/`。根目录 `README.md` 只给开发者。

## 范围与安全

- 允许改动路径：`tools/combat-sim/`、`tools/talkdnd/`、`tools/talkdnd-dev/`、`LetsPlay/Rules/combat/`、`LetsPlay/Rules/rulings*.md`、`LetsPlay/AGENTS.md`、`LetsPlay/docs/`、根目录与各目录 `README.md`、`docs/PROJECT-CHARTER.md`、`docs/PROJECT.md`、`docs/PLAN.md`、`AGENTS.md`、`kit/`、`editions/`、`templates/`、`scripts/`、`tests/`、`VERSION.md`、`LICENSE.md`、`NOTICE`、`MANIFEST.json`、`.github/`。具体任务可以再收窄。套件文件（含 `scripts/` 与 `tests/` 下的 Python）已并入本项目，可按任务修改；修改后要在同一 PR 说明本地差异。
- 永远禁止改动路径 / 文件：受版权保护的规则书与冒险书（`LetsPlay/Rules/2014/` 与 `LetsPlay/Rules/2024/` 除 `README.md` 外，`LetsPlay/Adventures/` 除 `README.md` 外，以及任何版权文本或其改编数据）；`LetsPlay/Saves/` 除 `README.md` 外；`**/dm-secrets.md`；`LetsPlay/Rules/combat/local/`；`LetsPlay/Rules/combat/2024/monsters/scarecrow.yaml`。
- 秘密与隐私文件模式：`.local/`（除 `README.md`）、`**/dm-secrets.md`、`LetsPlay/Saves/`（除 `README.md`）、GitHub 凭据、玩家规则书与冒险书文本。
- 破坏性操作和迁移政策：禁止改写 Git 历史、禁止强推。已有检出里的未跟踪本地书和存档用 `cargo run --manifest-path tools/talkdnd-dev/Cargo.toml -- migrate-layout`（默认 dry-run；真正移动时运行 `cargo run --manifest-path tools/talkdnd-dev/Cargo.toml -- migrate-layout --apply`），不得覆盖已有文件。主持游戏写入 `LetsPlay/Saves/` 豁免开发流程，但那些文件仍然不得提交。
- 新依赖政策：新增依赖（Rust crate、Python 第三方包、GitHub Action 以外的外部服务）必须先得到人工批准。`tools/talkdnd-dev` 只用 Rust 标准库。AgentOrch 套件保持上游 Python 标准库，不得改写为第三方依赖。
- 需要人工批准的变化：创建或删除 GitHub 仓库；改写历史或强推；新增依赖；合并到 `main`；规则版本语义变化（`ruleset` 分层、2014/2024 行为差异）；修改 `MANIFEST.json` 基线或重新生成清单。
- 游玩目录自洽：玩游戏需要的一切都在 `LetsPlay/`。根目录只放开发内容（源码、开发文档、CI、套件）。玩家运行的程序是不入库的 `LetsPlay/bin/combat-sim` 与 `LetsPlay/bin/talkdnd`（Windows 加 `.exe`）。`LetsPlay/` 里不得有指向目录外的启动脚本。`cargo run --manifest-path tools/talkdnd-dev/Cargo.toml -- check-letsplay` 会在跟踪的玩法文本（md、yaml、cmd、sh）引用了离开 `LetsPlay/` 的路径时失败。存档清单里从 `Saves/<存档>/combat/` 数起的 `../../../Rules` 仍在 `LetsPlay/` 内，必须通过。
- combat-sim 源码只读约定：修改 `tools/combat-sim/src` 下的文件之前先 `chmod u+w`，改完并验证之后 `chmod a-w`。`src` 没有变化时运行 `cargo run --manifest-path tools/talkdnd-dev/Cargo.toml -- fetch-binary`（需要已登录的 `gh`），不要在本机编译。只有 `src` 变化时才运行 `cargo run --manifest-path tools/talkdnd-dev/Cargo.toml -- install-local`。
- **PRIVACY RULE**: never put save contents, DM secrets, or copyrighted text into Issues, PRs, commit messages or CI logs.
- **LIVING CHARTER**: Master 必须在同一 PR 内更新 `docs/PROJECT-CHARTER.md`（Decision Log 与每一个受影响章节），只要该任务改变了范围、目标、技术栈、规则支持或一项用户决定。审查者在合并门核对这一点。缺了这次更新，审查不通过，PR 不得合并。

## 验证命令

下列命令已在接入时于本机执行。合并前和合并后都要重跑「合并后必须执行」里的命令。

- 快速测试：`uv run --no-project python scripts/validate_kit.py --project-ready`、`cargo run --manifest-path tools/talkdnd-dev/Cargo.toml -- check-leaks` 与 `cargo run --manifest-path tools/talkdnd-dev/Cargo.toml -- check-letsplay`
- 完整回归：`cargo test --manifest-path tools/combat-sim/Cargo.toml`、`cargo test --manifest-path tools/talkdnd/Cargo.toml` 与 `cargo test --manifest-path tools/talkdnd-dev/Cargo.toml`
- 静态检查 / 类型检查：`uv run --no-project python -m compileall -q scripts tests`（零依赖语法检查）
- 构建 / 打包：`src` 未改时运行 `cargo run --manifest-path tools/talkdnd-dev/Cargo.toml -- fetch-binary`。它用 `gh run list` / `gh run download` 取 `main` 上最近一次成功的 `build-artifacts` 制品 `combat-sim-<target>` 与 `talkdnd-<target>`（制品根上就是二进制），安装到 `LetsPlay/bin/combat-sim`、`LetsPlay/bin/talkdnd`（Windows 加 `.exe`），并在 unix 上加上可执行位。`--dry-run` 只打印命令。`--target` 可选 `x86_64-unknown-linux-gnu`、`x86_64-pc-windows-msvc`、`aarch64-apple-darwin`。`src` 改了才运行 `cargo run --manifest-path tools/talkdnd-dev/Cargo.toml -- install-local`：它对 `tools/combat-sim` 与 `tools/talkdnd` 各执行 `cargo build --release`，再把两个程序（Windows 加 `.exe`）复制到 `LetsPlay/bin/`。`--dry-run` 只打印命令。装好的文件只留在本机，不要提交。
- GitHub：`.github/workflows/ci.yml` 在 push 与 pull request 上跑 fmt、clippy、ubuntu / windows / macos-14 测试、cargo-deny、2014 与 2024 的 `content check`（只读仓库里的 `LetsPlay/Rules/combat`）、`validate_kit.py --project-ready`、完整套件 unittest、`compileall`、`check-leaks`、`check-letsplay`、yamllint 和离线相对链接检查。`main` 上的 `.github/workflows/build-artifacts.yml` 上传三个平台的 release 二进制，制品根上就是该文件，解压到 `LetsPlay/bin/`。标签发布仍走 `.github/workflows/release.yml`，压缩包根上同样只有该二进制。
- 发布与版本号：版本号规则见 `docs/versioning.md`（补丁 0.0.x = 小修复；次版本 0.x.0 = 增删功能或较大改动、存档结构变化；主版本 x.0.0 只用于大版本；每段是整数，0.9 之后是 0.10）。每次发布前先在 `CHANGELOG.md` 顶部写该版本的中文小节（新功能 / 变更 / 修复 / 更新说明·迁移 / 已知限制）；`release.yml` 用 `talkdnd-dev release-notes <tag>` 从中生成 GitHub Release 说明，缺小节就失败。三个 crate 的版本与 tag 一致。
- 手工 / UI 冒烟检查：程序已在 `LetsPlay/bin/` 时，于任意工作目录运行 `LetsPlay/bin/combat-sim roll 1d20+5 --dc 15`（Windows 为 `LetsPlay\bin\combat-sim.exe`）
- 套件测试：`uv run --no-project python -m unittest discover -s tests`
- pdf2md 测试：`uv run --project LetsPlay/tools/pdf2md --group dev pytest -q LetsPlay/tools/pdf2md/tests`
- 合并后必须执行：`cargo test --manifest-path tools/combat-sim/Cargo.toml`；`cargo test --manifest-path tools/talkdnd/Cargo.toml`；`cargo test --manifest-path tools/talkdnd-dev/Cargo.toml`；`uv run --no-project python scripts/validate_kit.py --project-ready`；`uv run --no-project python -m unittest discover -s tests`；`cargo run --manifest-path tools/talkdnd-dev/Cargo.toml -- check-leaks`；`cargo run --manifest-path tools/talkdnd-dev/Cargo.toml -- check-letsplay`

`tools/combat-sim/tests/real_content.rs` 在 `LetsPlay/Saves/2026-10-19-01/combat/campaign.yaml` 不存在时跳过并打印说明。CI 没有版权书，也没有存档。

## 运行配置

本项目的底座以本文件为准，不以 `scripts/orchestrator.py` 的内置缺省为准。

- `edition`：`github`
- 活跃底座是 `github`：任务单在 GitHub Issue，证据在 PR 与 CI。`editions/local/` 必须留在树里，因为 `scripts/validate_kit.py` 要求两套底座文件齐全；不得把 local 任务库当作第二套状态源，也不得双写。**不要运行** `uv run --no-project python scripts/orchestrator.py init`：它即使声明 `github` 也会写出 `.orchestration/state.sqlite3` 的本地任务库。`docs/PLAN.md` 由本文件所描述的项目内容手工维护，不通过 `init` 创建。
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

本仓库是现有项目。播放布局在 `LetsPlay/`，开发契约在根目录。项目章程是已填写的 `docs/PROJECT-CHARTER.md`；设计目标是 `docs/PLAN.md`。两者都随项目演进：维护规则见上面的 **LIVING CHARTER**。套件入口（`kit/orchestration.md`）的章程查找以 `docs/PROJECT-CHARTER.md` 为准。

任务状态只在 GitHub Issues；本仓库**不建立** `docs/implementation-plan.md` 或其它第二套任务清单。阶段计划只在未来确有需要时由 Master 明确决定，并必须把状态留给 Issue 载体，不得双写。`templates/implementation-plan.md` 只是套件模板，不是本项目的计划文件。

## AgentOrch 本地差异

本树是 **AgentOrch 1.6.3（Format 1.6）**，安装官方 `AgentOrch-1.6.3.zip` 的 92 个 `REQUIRED` 文件与官方 `MANIFEST.json` 基线。1.6.3 已包含上游对下游 `docs/`、manifest drift、路径大小写和 `check_drift` 缓存跳过的修复；本树只保留以下有意差异：

- `scripts/validate_kit.py`：链接扫描跳过 gitignored 玩家书籍/存档（保留其 `README.md`）、pdf2md `.md/` 输出、`.worktrees`、`target` 与本地缓存；被跟踪文档的真实断链仍会报告。上游 1.6.3 的下游模式（只检查 `docs/PROJECT.md` / `docs/PLAN.md` 和清单结构，core drift 交给 `check_drift.py`）保持原样。
- `tests/test_validate_kit.py`：在 1.6.3 上游测试上增加本地书/生成目录跳过的回归测试；没有删除断言。
- `AGENTS.md`：TalkDND 第 8 节、顶部 PLAY 提示与合并门说明，不属于上游通用文件；1.6.3 的「阅读口径（唯一权威）」段已保留。

1.6.3 新增的 `scripts/check_scope.py` 与 `tests/test_check_scope.py` 随包安装。

预期 `check_drift.py .` 报告 **3 个 modified 文件**（`AGENTS.md`、`scripts/validate_kit.py`、`tests/test_validate_kit.py`）与 **2 个 project-profile 文件**（`docs/PROJECT.md`、`docs/PLAN.md`）。这是公开、可解释的漂移，不是失败。**不要**用本树重新生成 `MANIFEST.json`：那会把本地修改记成上游基线。后续更新先运行 `uv run --no-project python -m scripts.update_agentorch.entry --project . --request "更新 AgentOrch" --mode PLAN`；出现 `THREE_WAY_CONFLICT` 时停下交人工决定，不得自动覆盖。

本树未内置上游仓库专用的 `docs/README.md`、`docs/charter.md`、`docs/upgrade.md`、`docs/drift.md`、`docs/metrics.md`、`docs/design/resource-locks.md`、`docs/testing/delivery-regression-scenarios.md`、`docs/self-improvement/*`。需要时：自我完善读 `templates/self-improvement/README.md`；资源锁语义读 `kit/orchestration.md` 第 3.3 节与 `templates/` 下的说明；更新安全语义以 `templates/self-improvement/README.md` 和引擎报告为准。`github` 底座不使用本地资源锁，资源占用只以 Issue 登记块协作记录。
