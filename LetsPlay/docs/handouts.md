# 资料图（handout）

给玩家看冒险书里的图：地图、信件、告示、重要物品。图来自本地用 pdf2md 转换的书（`Adventures/…/.md/<书名>/img/`），只在本地使用，绝不提交。

## 数据

- **登记表**：`Adventures/…/.md/<书名>/handouts.yaml`，和转换后的书放在一起（已被 gitignore）。第一次用 `handout` 命令时由 pdf2md 的 `index.json`（地图图注、插图、整页图）和 `img/` 自动生成；`$T handout build` 重新扫描，保留已有标记。
- 每条：`id`（图片文件名，如 `p0012-00`）、`img`、`page`、`kind`（map / art / figure）、`caption`（地图图注）、`area`（该页之前最近的章节或区域标题）、`visibility`（`dm` 或 `player`）、可选 `crop`（玩家可见区域）、`note`。
- **默认全部「仅城主」**。只有图注写明玩家版（Player Version / 玩家）的地图自动标为 `player`。
- **已展示**：`Saves/<存档>/handouts.yaml` 和 `Saves/<存档>/handouts/` 下导出的图（存档目录本来就不提交）。

## 命令

| 命令 | 作用 |
| --- | --- |
| `handout show <id\|关键词> [--crop x,y,w,h] [--png]` | 导出玩家可见的图，打印 `file:` 路径和 `markdown:` 图片行；写进已揭示账本（`资料：<图注>`，标签 `handout:<id>`）和会话日志。城主图直接拒绝 |
| `handout list` | 已经给玩家看过的图 |
| `handout list --dm` | 整张登记表、可见性和图片路径（城主自己看） |
| `handout mark <id> --player\|--dm [--crop …] [--caption …] [--note …]` | 改可见性；`--crop` 设了以后只导出这块，`show` 不能再用别的区域 |
| `handout build` | 从索引重建登记表 |

都可以加 `--book <文件夹或书名>`；默认用存档设置的冒险书。`--crop` 的四个数都 ≤1 时按比例算，否则按像素；裁剪结果存成 PNG。

## 防剧透

- 城主图（暗门、陷阱、怪物或宝藏编号、未揭示的名字）和含剧透的整页插图不展示，只描述角色看到的部分。
- 标 `--player` 之前必须亲自看过图；只有一部分安全就用 `--crop`。拿不准就保持 `dm`。
- 同一张图再次展示不会重复记账，只写一行日志。

## 在不同宿主里展示

- 支持 Markdown 图片的聊天界面：贴 `markdown:` 那一行。
- 能附文件的宿主：附 `file:` 的路径。
- 纯终端：给出路径让玩家自己打开。
- 不显示 WebP：加 `--png`。

## 局限

- 图注和区域来自 pdf2md 的启发式结果；整页图（figure）常常带着正文，多半要裁剪。
- 程序不会识别图里的内容，玩家可见与否全靠城主判断；默认仅城主就是为此。
- 裁剪按矩形，不能挖掉中间的编号。
