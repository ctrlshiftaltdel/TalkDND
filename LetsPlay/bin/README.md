# bin

本目录放两个程序（Windows 加 `.exe`），文件不入库：

- `combat-sim`：所有掷骰与战斗结算。
- `talkdnd`：开局摘要（`start`）、新存档与建卡（`new`、`chargen`）、随机同伴（`npc`）、存档点与日志（`checkpoint`、`log`）。它的掷骰用的是 combat-sim 同一套骰子引擎。

GitHub Releases 的压缩包已经把两个程序放在这里。`main` 分支的 `build-artifacts` 有两个制品 `combat-sim-<平台>` 与 `talkdnd-<平台>`，各自解压到这里。Linux 和 macOS 若缺少执行权限：

```bash
chmod +x combat-sim talkdnd
```

在上一级目录调用 `bin/combat-sim`、`bin/talkdnd`（Windows 是 `bin\combat-sim.exe`、`bin\talkdnd.exe`）。放法见上一级的 `README.md`。
