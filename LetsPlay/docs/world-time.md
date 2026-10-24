# 世界时间与预定事件（`talkdnd time`）

> 本文件是 `AGENTS.md` 的参考章节，按需查阅。数据契约见 `docs/campaign-continuity.md` §2.4/§4.5/§5.4。

## 这是什么

`talkdnd time` 给存档加一个**可选**的游戏内时钟与预定事件表，数据存在 `Saves/<slot>/world-time.yaml`（version 1）。

- **默认关闭**：没有 `world-time.yaml` 或 `enabled: false` 时，功能就是关的；写命令一律退出码 1，文件不会被创建或改动。
- **只按显式命令推进**：只有 `time set` / `time advance` 改时间。叙述、场景切换、战斗、休息结算、`start` 都不推进，也绝不写 `world-time.yaml`。
- **不做历法算术**：日期和时段是自由文本标签（如 `1 Hammer`、`黄昏`），程序不推断月长、闰年或标签先后。唯一的天数计数是 `elapsed_days`（`time advance` 累加，`--hours` 按 h/24 折算）。
- **到期只按标签相等**：事件的 `due` 与当前 `date` 相同且状态是 `pending`，就算「今天到期」；`done` / `skipped` 后不再提示。事件不自动改数值或剧情。

## 命令

| 命令 | 说明 |
|---|---|
| `time show` | 显示启用状态、历法、日期、时段、已过天数与事件计数（未启用也可用） |
| `time enable` | 开启（幂等；重复执行不写文件、不记日志） |
| `time disable` | 关闭；日期/时段/事件/已过天数保留（幂等） |
| `time set --date D [--phase P]` | 设当前日期标签；`--phase` 可选，`--phase ""` 清空时段；未启用拒绝 |
| `time advance --days N 或 --hours N` | 推进时钟（至少一个，合计必须 >0）；纯加法 |
| `time event add --due 标签 --title T [--dm-only] [--note N]` | 新增事件，id 从 EV-001 递增、永不复用；`--note` 可文本引用实体 id（不做外键校验） |
| `time event list [--all]` | 列事件；`due` 等于当前日期且 pending 的标「今天到期」 |
| `time event done <id>` / `time event skip <id>` | 标记完成/跳过；未知 id 或已是终态时退出码 1 |

可见性（2026-10-25 项目所有者决定）：`time event list` 默认只列非 `dm_only` 事件（玩家视图）；`--all` 是 DM 视图，全部事件都列出，`dm_only` 行首标 `[DM]`。`start` 摘要只提示玩家可见的今天到期事件。`--all` 的输出是 DM 视图，不要进入玩家可看材料（见 `docs/campaign-continuity.md` §6.2 与 guard）。

## start 摘要

`talkdnd start` 只在 `enabled: true` 时出现「世界时间」节：当前日期、时段（设置了才显示）、已过天数，以及玩家可见的今天到期事件列表。关闭或没有到期事件时相应内容不出现。

## 边界与错误

- 未启用时 `set` / `advance` / `event add` / `event done` / `event skip` 一律退出码 1，且不创建、不改动 `world-time.yaml`。
- `advance --days 0 --hours 0` 退出码 1；负值是命令行用法错误（退出码 2）；超出 32 位整数范围退出码 1。
- `elapsed_days` 每次写入按 9 位小数四舍五入；显示最多 3 位小数并去尾零（24 次 `--hours 1` 显示 `1`）。
- `world-time.yaml` 非法 YAML 或 `version` 大于 1 时，命令退出码 1 并点名文件与版本。
- 程序写入会自动更新校验清单；手改会被 `talkdnd check` 报告为「changed outside the tools」，按桌规处理（见 `docs/table-integrity.md`）。

## 推进纪律

开团时由 DM 决定何时 `time advance`：长休、赶路、过夜这类「时间过去了」的时刻才推进；场景内推进（一场战斗、一段对话）不推进。事件到期只是提示——按剧情处置，处置结果（改数值、触发遭遇、改场景）由 DM 显式操作，程序不自动做任何事。
