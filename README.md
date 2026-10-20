# TalkDND

在聊天里玩 D&D 的开发仓库。要玩，进入 `LetsPlay/`，读 `LetsPlay/README.md`，在那个目录启动 Agent。DM 手册是 `LetsPlay/AGENTS.md`。

本文件给开发者。玩游戏需要的书、存档、裁定、战斗数据和 combat-sim 程序都在 `LetsPlay/` 里。根目录只放源码、开发文档、CI 和 AgentOrch 套件。

开发契约是 `AGENTS.md`。项目章程是 `docs/PROJECT-CHARTER.md`。

## 程序放在哪

玩家运行的文件是 `LetsPlay/bin/combat-sim`（Windows 是 `LetsPlay/bin/combat-sim.exe`）。不入库。GitHub Releases 的资产 `talkdnd-letsplay-<标签>-<target>`（Windows `.zip`，其余 `.tar.gz`，各带 `.sha256`）是整个已入库的 `LetsPlay/` 加上 `LetsPlay/bin/` 里的程序，由 CI 从 `git archive` 打包，并检查包内只有入库文件，没有 PDF、书或存档。`build-artifacts` 制品的根上只有程序，解压到 `LetsPlay/bin/`。发版：在 `main` 上打 `v*` 标签并推送。

`tools/combat-sim/src` 没改时，在仓库根目录拉取 `main` 上最近一次成功构建（需要已登录的 [GitHub CLI](https://cli.github.com/)）：

```bash
cargo run --manifest-path tools/talkdnd-dev/Cargo.toml -- fetch-binary
```

`--dry-run` 只打印 `gh run list` / `gh run download`。`--target` 可以是 `x86_64-unknown-linux-gnu`、`x86_64-pc-windows-msvc` 或 `aarch64-apple-darwin`。制品大约保留 30 天。

`src` 改了才在本机做 release 构建并复制过去：

```bash
cargo run --manifest-path tools/talkdnd-dev/Cargo.toml -- install-local
```

这条命令等价于 `cargo build --release --manifest-path tools/combat-sim/Cargo.toml`，再把 `tools/combat-sim/target/release/combat-sim`（Windows 为 `combat-sim.exe`）复制到 `LetsPlay/bin/`，并在 Linux / macOS 上加上可执行位。`--dry-run` 只打印命令。装好的文件不要提交。

源码平时只读：改某个 `tools/combat-sim/src` 文件之前 `chmod u+w`，验证之后 `chmod a-w`。

放好之后：

```bash
LetsPlay/bin/combat-sim roll 1d20+5
```

Windows 是 `LetsPlay\bin\combat-sim.exe roll 1d20+5`。macOS 第一次运行若被隔离属性拦住：`xattr -d com.apple.quarantine LetsPlay/bin/combat-sim`。

玩家向的解压说明、规则书位置和开局步骤在 `LetsPlay/README.md`。

## 开发检查

```bash
cargo fmt --manifest-path tools/combat-sim/Cargo.toml --check
cargo fmt --manifest-path tools/talkdnd-dev/Cargo.toml --check
cargo clippy --manifest-path tools/combat-sim/Cargo.toml --all-targets -- -D warnings
cargo clippy --manifest-path tools/talkdnd-dev/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path tools/combat-sim/Cargo.toml
cargo test --manifest-path tools/talkdnd-dev/Cargo.toml
python3 scripts/validate_kit.py
cargo run --manifest-path tools/talkdnd-dev/Cargo.toml -- check-leaks
cargo run --manifest-path tools/talkdnd-dev/Cargo.toml -- check-letsplay
```

`check-letsplay` 在 `LetsPlay/` 里跟踪的 md、yaml、cmd、sh 引用了目录外路径时失败。存档清单模板 `LetsPlay/example/save/combat/campaign.yaml` 里的库路径仍在 `LetsPlay/` 内。

## 版权

- 代码（`tools/`）、开发文档、`LetsPlay/Rules/rulings*.md`：MIT，见 `LICENSE`。
- `LetsPlay/Rules/combat/` 只取自 SRD 5.1 与 5.2.1，CC-BY-4.0，署名见 `NOTICE`。
- 「Dungeons & Dragons」「D&D」是 Wizards of the Coast 的商标。本项目是非官方爱好者作品。
