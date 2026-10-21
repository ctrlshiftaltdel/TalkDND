# 书籍文本：pdf2md

> 本文件是 `AGENTS.md` 的参考章节，按需查阅。路径相对于 `LetsPlay/`。


玩家只需把 PDF 放进 `Rules/2014/`、`Rules/2024/`、`Adventures/`（可放子目录）。`tools/pdf2md` 会在后台把 PDF 逐段转成便于搜索的 Markdown，放在 PDF 旁边的隐藏目录里，文件名与 PDF 相同：

```
Rules/2024/Player's Handbook 2024.pdf
Rules/2024/.md/Player's Handbook 2024/Player's Handbook 2024.md   # 全书，<a id="pN"></a> = PDF 第 N 页
Rules/2024/.md/Player's Handbook 2024/index.md                    # 标题 → 页码、数据块页、地图页、低质量页
Rules/2024/.md/Player's Handbook 2024/img/p0123.webp              # 地图/插图页面图（链接，不嵌入）
```

**首次使用（每台机器一次）**。所有 Python 都通过 [uv](https://docs.astral.sh/uv/) 运行：

1. 没有 `uv` 时先安装（官方安装器）：
   - Linux / macOS：`curl -LsSf https://astral.sh/uv/install.sh | sh`
   - Windows（PowerShell）：`powershell -ExecutionPolicy ByPass -c "irm https://astral.sh/uv/install.ps1 | iex"`
2. 在本目录执行：`uv sync --project tools/pdf2md`（自动下载合适的 Python 和依赖，装进 `tools/pdf2md/.venv/`）。
3. 再执行：`uv run --project tools/pdf2md python -m pdf2md setup`（下载英文词表、检查 OCR 引擎）。出现 `setup OK` 即可。

依赖 PyMuPDF / pymupdf4llm（AGPL）与 RapidOCR 由这一步从网上安装，不随 TalkDND 分发。可选的 PaddleOCR 引擎：`uv sync --project tools/pdf2md --extra paddle`，运行时加 `--ocr-engine paddle`（体积约 1 GB、CPU 上明显更慢，一般不需要）。以后去掉 paddle 时要执行 `uv sync --project tools/pdf2md --reinstall-package opencv-python`，否则两个 OpenCV 包共用的目录会被删坏。

**每次开局**（`talkdnd start` 已自动在后台启动 `pdf2md run`；没有 uv 时跳过）：

- 先看进度：`uv run --project tools/pdf2md python -m pdf2md status`。
- **能开后台任务 / 子代理时**：在后台启动 `uv run --project tools/pdf2md python -m pdf2md run`，不要等待它，继续主持。它自动低优先级运行，顺序是：当前存档的冒险书 → 当前规则版本的 SRD → 该版其他书 → 其他冒险 → 另一版规则书。中断后再次运行会接着做；同一时间只会有一个转换进程。
- **不能开后台任务时**：只在场景之间（不在战斗或关键剧情中）跑一小段：`uv run --project tools/pdf2md python -m pdf2md run --budget 60`（最多 60 秒）。
- 马上要用某几页而它们还没转：`uv run --project tools/pdf2md python -m pdf2md run --book "<书名的一部分>" --pages 40-52 --budget 120`。
- 输出不入库，也不要贴进任何 issue、PR 或日志。

**可选：质量审核与逐页修正**（任何代理都能担任审核角色，不是开局必需）：

- `uv run --project tools/pdf2md python -m pdf2md audit [--book 名称]`：逐页对照 PDF 文字层打分（召回率、未知词/千词、可疑标题、表格结构、空页/重复/串栏、低质量标记、`inferred`/`check PDF` 标记数），结果按最差在前写入 `.md/<书名>/audit.json`；`pdf2md fix-queue` 列出待审页。
- `pdf2md review --book 名称 [--page 12,30-32 | --top 10] --engine grok|cmd:<命令> --jobs 2`：把该页 Markdown、PDF 文字层和页面图交给审核代理改写。`grok` 用 Grok Build；`cmd:<命令>` 可接任何代理（提示文件路径作为最后一个参数，修正后的 Markdown 输出到 stdout）。拿不准的地方先查页面图、SRD 与其他本地书、本书交叉引用；仍不确定就按上下文合理补全并标 `<!-- inferred: 原因 -->`，只有完全无法辨认才标 `<!-- check PDF -->`。
- 只有召回率不降、未知词不升时才采用，保存为 `fixes/pNNNN.md`，重新转换后依然生效；否则放进 `fixes/rejected/` 并记入 `fixes/review.log.jsonl`。`pdf2md apply` 在没有转换进程时立即重建该书（否则下次组装时自动应用）。
- 修正文件同样只在本地，不入库、不贴出去。查书时遇到 `inferred` 标记，关键数值仍以 PDF 为准。

**查书顺序**：

1. 先查该书的 `index.md`，或用关键词搜 `<书名>.md`，按 `<a id="pN">` 得到 PDF 页码。
2. 以下情况一律打开 PDF 对应页核对：`index.md` 列为低质量的页、**怪物/NPC 数据块**（转换不重建数据块，数值以 PDF 为准）、表格看起来错位、文字明显缺漏，以及所有**地图**（用 PDF 或 `img/` 下的页面图）。
3. 书还没转到那一页：直接查 PDF。**PDF 永远是权威来源**；Markdown 只是便于检索的副本。

## 0.3.0：校对、原位图片、房间与地图索引

- **校对规则**（移植自 ConvertPDF，只改正文，不碰页锚点、`<!-- -->` 标记、图片和链接、行内代码、表格和标题）：
  - NUM：骰子和数字里的字母混淆，如 `Sd6`→`5d6`、`ld8`→`1d8`、`SO ft.`→`50 ft.`。
  - ONE：把当作 I 用的数字 1 改回来（`1 want`→`I want`）。
  - SPLIT：被字距拆开的词（`charac ter`→`character`），两半都是真词时不合并。
  - SPACE：标点后缺空格、标点前多空格、连续空格。
  - GLUE：粘在一起的词（`ofYsgard`→`of Ysgard`）。
- 每次重建都会在书目录写 `proofread-report.md`，列出每条改动，方便复核（只留在本地）。
- **图片放在原位**：PDF 里嵌入的图片（地图、插图）按原来的位置插进正文，存为 `img/pNNNN-KK.webp`。旁边有 `MAP x.y: ...` 图注时，图注就是图片说明。
- **index.md 的「Maps」一节**：按图注列出每张地图，链接到所在页和图片，并标出 DM 图或玩家图。整页图仍列在「Full-page maps and illustrations」。
- **房间编号**：`V1: GRAND ENTRANCE` 这类房间行会提升为标题，OCR 把 1 认成 l/I 的情况（`Vl`、`Vl4`）会改正，所以房间都能在索引里找到。
- 升级到 0.3.0 后，已转好的书会在后台逐块重做。
