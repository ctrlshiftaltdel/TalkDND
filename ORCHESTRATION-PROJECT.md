# ORCHESTRATION-PROJECT.md — Per-Project Configuration

> 这是 TalkDND 的项目适配层。通用规则留在 `AGENTS.md` 与 `AGENT-ORCHESTRATION.md`，不要把本项目的路径和测试命令写进那些文件。

## 项目身份

- 项目名称：`TalkDND`
- 一句话目标：让 AI 在聊天里按玩家自备的 D&D 规则书主持冒险，并用 combat-sim 结算战斗；受版权保护的书和冒险、以及个人存档，永不进入仓库或公开记录。
- 现有项目 / 新项目：`现有项目`
- 本地主分支：`main`
- 语言 / 框架 / 运行方式：项目自己的程序、工具和脚本一律用 Rust（`tools/combat-sim` 战斗引擎，`tools/talkdnd-dev` 开发工具）与 Markdown/YAML 规则数据。玩家在 `LetsPlay/` 里开局。**TECH STACK** 的接受例外只有下面两项，不得再扩大：
  - 随上游原样保留、不得修改的 AgentOrch Python 套件脚本：`scripts/orchestrator.py`、`scripts/validate_kit.py`、`tests/test_orchestrator.py`；
  - CI YAML（`.github/workflows/`，含其中的 shell 步骤）。
- 重要设计 / 需求文件：项目章程是 `docs/PROJECT-CHARTER.md`。AGENT-BOOTSTRAP 判断「章程缺失」时只看这一路径：文件存在且没有占位符，即章程已建立。不要改套件文件来改查找路径。玩家说明是 `LetsPlay/README.md`；DM 手册是 `LetsPlay/AGENTS.md`；引擎说明是 `tools/combat-sim/README.md` 与 `tools/combat-sim/docs/`。根目录 `README.md` 只给开发者。

## 范围与安全

- 允许改动路径：`tools/combat-sim/`、`tools/talkdnd-dev/`、`LetsPlay/Rules/combat/`、`LetsPlay/Rules/rulings*.md`、`LetsPlay/AGENTS.md`、根目录与各目录 `README.md`、`docs/PROJECT-CHARTER.md`、编排文件（`AGENTS.md`、`AGENT-*.md`、`ORCHESTRATION-PROJECT.md`、`editions/`、`prompts/`、`templates/`、`VERSION.txt`）、`.github/`、`scripts/`。具体任务可以再收窄。套件脚本 `scripts/orchestrator.py`、`scripts/validate_kit.py`、`tests/test_orchestrator.py` 即使落在 `scripts/` 下也不得修改。
- 永远禁止改动路径 / 文件：受版权保护的规则书与冒险书（`LetsPlay/Rules/2014/` 与 `LetsPlay/Rules/2024/` 除 `README.md` 外，`LetsPlay/Adventures/` 除 `README.md` 外，以及任何版权文本或其改编数据）；`LetsPlay/Saves/` 除 `README.md` 外；`**/dm-secrets.md`；`LetsPlay/Rules/combat/local/`；`LetsPlay/Rules/combat/2024/monsters/scarecrow.yaml`。
- 秘密与隐私文件模式：`.local/`（除 `README.md`）、`**/dm-secrets.md`、`LetsPlay/Saves/`（除 `README.md`）、GitHub 凭据、玩家规则书与冒险书文本。
- 破坏性操作和迁移政策：禁止改写 Git 历史、禁止强推。已有检出里的未跟踪本地书和存档用 `cargo run --manifest-path tools/talkdnd-dev/Cargo.toml -- migrate-layout`（默认 dry-run；真正移动时运行 `cargo run --manifest-path tools/talkdnd-dev/Cargo.toml -- migrate-layout --apply`），不得覆盖已有文件。主持游戏写入 `LetsPlay/Saves/` 豁免开发流程，但那些文件仍然不得提交。
- 新依赖政策：新增依赖（Rust crate、Python 第三方包、GitHub Action 以外的外部服务）必须先得到人工批准。`tools/talkdnd-dev` 只用 Rust 标准库。三份 AgentOrch 套件脚本保持上游 Python 标准库，不得改写。
- 需要人工批准的变化：创建或删除 GitHub 仓库；改写历史或强推；新增依赖；合并到 `main`；规则版本语义变化（`ruleset` 分层、2014/2024 行为差异）。
- 游玩目录自洽：玩游戏需要的一切都在 `LetsPlay/`。根目录只放开发内容（源码、开发文档、CI、套件）。玩家运行的程序是不入库的 `LetsPlay/bin/combat-sim`（Windows 为 `LetsPlay/bin/combat-sim.exe`）。`LetsPlay/` 里不得有指向目录外的启动脚本。`cargo run --manifest-path tools/talkdnd-dev/Cargo.toml -- check-letsplay` 会在跟踪的玩法文本（md、yaml、cmd、sh）引用了离开 `LetsPlay/` 的路径时失败。存档清单里从 `Saves/<存档>/combat/` 数起的 `../../../Rules` 仍在 `LetsPlay/` 内，必须通过。
- combat-sim 源码只读约定：修改 `tools/combat-sim/src` 下的文件之前先 `chmod u+w`，改完并验证之后 `chmod a-w`。`src` 没有变化时运行 `cargo run --manifest-path tools/talkdnd-dev/Cargo.toml -- fetch-binary`（需要已登录的 `gh`），不要在本机编译。只有 `src` 变化时才运行 `cargo run --manifest-path tools/talkdnd-dev/Cargo.toml -- install-local`。
- **PRIVACY RULE**: never put save contents, DM secrets, or copyrighted text into Issues, PRs, commit messages or CI logs.
- **LIVING CHARTER**: Master 必须在同一 PR 内更新 `docs/PROJECT-CHARTER.md`（Decision Log 与每一个受影响章节），只要该任务改变了范围、目标、技术栈、规则支持或一项用户决定。审查者在合并门核对这一点。缺了这次更新，审查不通过，PR 不得合并。

## 验证命令

下列命令已在接入时于本机执行。合并前和合并后都要重跑「合并后必须执行」里的命令。

- 快速测试：`python3 scripts/validate_kit.py`、`cargo run --manifest-path tools/talkdnd-dev/Cargo.toml -- check-leaks` 与 `cargo run --manifest-path tools/talkdnd-dev/Cargo.toml -- check-letsplay`
- 完整回归：`cargo test --manifest-path tools/combat-sim/Cargo.toml` 与 `cargo test --manifest-path tools/talkdnd-dev/Cargo.toml`
- 静态检查 / 类型检查：`python3 -m py_compile scripts/orchestrator.py scripts/validate_kit.py`
- 构建 / 打包：`src` 未改时运行 `cargo run --manifest-path tools/talkdnd-dev/Cargo.toml -- fetch-binary`。它用 `gh run list` / `gh run download` 取 `main` 上最近一次成功的 `build-artifacts` 制品 `combat-sim-<target>`（制品根上就是二进制），安装到 `LetsPlay/bin/combat-sim`（Windows 为 `LetsPlay/bin/combat-sim.exe`），并在 unix 上加上可执行位。`--dry-run` 只打印命令。`--target` 可选 `x86_64-unknown-linux-gnu`、`x86_64-pc-windows-msvc`、`aarch64-apple-darwin`。`src` 改了才运行 `cargo run --manifest-path tools/talkdnd-dev/Cargo.toml -- install-local`：它执行 `cargo build --release --manifest-path tools/combat-sim/Cargo.toml`，再把 `tools/combat-sim/target/release/combat-sim`（Windows 为 `combat-sim.exe`）复制到 `LetsPlay/bin/`。`--dry-run` 只打印命令。装好的文件只留在本机，不要提交。
- GitHub：`.github/workflows/ci.yml` 在 push 与 pull request 上跑 fmt、clippy、ubuntu / windows / macos-14 测试、cargo-deny、2014 与 2024 的 `content check`（只读仓库里的 `LetsPlay/Rules/combat`）、`validate_kit.py`、`check-leaks`、`check-letsplay`、yamllint 和离线相对链接检查。`main` 上的 `.github/workflows/build-artifacts.yml` 上传三个平台的 release 二进制，制品根上就是该文件，解压到 `LetsPlay/bin/`。标签发布仍走 `.github/workflows/release.yml`，压缩包根上同样只有该二进制。
- 手工 / UI 冒烟检查：程序已在 `LetsPlay/bin/` 时，于任意工作目录运行 `LetsPlay/bin/combat-sim roll 1d20+5 --dc 15`（Windows 为 `LetsPlay\bin\combat-sim.exe`）
- 合并后必须执行：`cargo test --manifest-path tools/combat-sim/Cargo.toml`；`cargo test --manifest-path tools/talkdnd-dev/Cargo.toml`；`python3 scripts/validate_kit.py`；`cargo run --manifest-path tools/talkdnd-dev/Cargo.toml -- check-leaks`；`cargo run --manifest-path tools/talkdnd-dev/Cargo.toml -- check-letsplay`

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

本仓库是现有项目。播放布局在 `LetsPlay/`，开发契约在根目录。项目章程是已填写的 `docs/PROJECT-CHARTER.md`。它是活文档：维护规则见上面的 **LIVING CHARTER**。AGENT-BOOTSTRAP 的章程查找以该路径为准，不修改套件文件。
