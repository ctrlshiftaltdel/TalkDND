# talkdnd-dev

`tools/` 的开发命令集。只用标准库，没有第三方依赖。在仓库根目录运行：

```bash
cargo run --manifest-path tools/talkdnd-dev/Cargo.toml -- <命令>
```

| 命令 | 作用 |
|------|------|
| `check-leaks` | 扫描 `git ls-files`：书籍文件（pdf/epub/docx）、书页图片、`Saves/` 等私有目录、pdf2md 生成目录、超过 300 KiB 的正文都算命中；逐行打印并退出 1 |
| `check-letsplay` | 检查 `LetsPlay/` 里跟踪的 md/yaml/yml/cmd/sh：不得引用目录外路径，骰子只能用 `combat-sim roll`；有命中时退出 1 |
| `migrate-layout` | 把旧的根目录游玩布局移进 `LetsPlay/`；默认 dry-run，`--apply` 才移动 |
| `fetch-binary` | 从 `main` 上最近一次成功的 `build-artifacts` 下载当前平台的 `combat-sim` 与 `talkdnd`，装到 `LetsPlay/bin/`；`--dry-run` 只打印，`--target` 指定三元组；需要已登录的 `gh` |
| `install-local` | release 构建 `tools/combat-sim` 与 `tools/talkdnd`，把两个二进制装到 `LetsPlay/bin/`；`--dry-run` 只打印 |
| `release-notes <tag>` | 取 `CHANGELOG.md` 里该 tag 的小节，附下载说明后打印；`--changelog` 可换文件 |

退出码：0 通过或完成，1 检查有命中，2 用法错误或执行失败。

`fetch-binary` / `install-local` 装出来的文件只留在本机，不要提交。
