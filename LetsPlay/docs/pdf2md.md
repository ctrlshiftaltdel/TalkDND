# 书籍文本：pdf2md

> 本文件是 `AGENTS.md` 的参考章节，按需查阅。路径都相对于游戏文件夹（解压出来的那个文件夹，名字随意）。


玩家只需把 PDF 放进 `Rules/2014/`、`Rules/2024/`、`Adventures/`。`talkdnd start` 先把每本书整理进自己的文件夹（`<slug>/`，见 `docs/layout.md`「书库」），`tools/pdf2md` 再在后台把 PDF 逐段转成便于搜索的 Markdown，放在书文件夹的 `index/` 里，文件名与 PDF 相同：

```
Rules/2024/players-handbook-2024/Player's Handbook 2024.pdf
Rules/2024/players-handbook-2024/index/Player's Handbook 2024.pages.md  # 唯一的正文：<a id="pN"></a> = PDF 第 N 页
Rules/2024/players-handbook-2024/index/index.md                         # 标题 → 页码、数据块检查、地图、插图、低质量页
Rules/2024/players-handbook-2024/index/img/p0123.webp                   # 整页地图/插图（最长边约 1600 px）
Rules/2024/players-handbook-2024/index/img/p0123-a1.webp                # 从扫描页裁出的插图（最长边约 1000 px）
```

不在书文件夹里的 PDF（还没整理过）仍按旧方式输出到 PDF 旁的隐藏目录 `.md/<书名>/`；下次 `talkdnd start` 会把它们一起移进 `<slug>/index/`。

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

- `uv run --project tools/pdf2md python -m pdf2md audit [--book 名称]`：逐页对照 PDF 文字层打分（召回率、未知词/千词、可疑标题、表格结构、空页/重复/串栏、低质量标记、`inferred`/`check PDF` 标记数），结果按最差在前写入该书 `index/audit.json`；`pdf2md fix-queue` 列出待审页。
- `pdf2md review --book 名称 [--page 12,30-32 | --top 10] --engine grok|cmd:<命令> --jobs 2`：把该页 Markdown、PDF 文字层和页面图交给审核代理改写。`grok` 用 Grok Build；`cmd:<命令>` 可接任何代理（提示文件路径作为最后一个参数，修正后的 Markdown 输出到 stdout）。拿不准的地方先查页面图、SRD 与其他本地书、本书交叉引用；仍不确定就按上下文合理补全并标 `<!-- inferred: 原因 -->`，只有完全无法辨认才标 `<!-- check PDF -->`。
- 只有召回率不降、未知词不升时才采用，保存为 `fixes/pNNNN.md`，重新转换后依然生效；否则放进 `fixes/rejected/` 并记入 `fixes/review.log.jsonl`。`pdf2md apply` 在没有转换进程时立即重建该书（否则下次组装时自动应用）。
- 修正文件同样只在本地，不入库、不贴出去。查书时遇到 `inferred` 标记，关键数值仍以 PDF 为准。

**游玩中修正（正文与 PDF 不符时）**：以 PDF 为准，当场改好再继续。

1. `uv run --project tools/pdf2md python -m pdf2md render "<书名>" <页>` 生成该页图片（或直接打开 PDF），核对文字、数字和表格。
2. 把该页正确的 Markdown 写到临时文件；不要写页锚点，程序会补。
3. `uv run --project tools/pdf2md python -m pdf2md fix --book "<书名>" --page <页> --from <临时文件> --reason "<一句话原因>"`。没有转换进程时它立即重组；提示 `worker running` 时，修正会在下次组装生效。
4. 用 `bin/talkdnd adventure section ...` 再读一次确认。修正留在 `index/fixes/`，不入库、不贴出去。

**查书顺序**：

1. 先查该书的 `index/index.md`（链接都指向 `.pages.md#pN`）。正文只有一份 `index/<书名>.pages.md`（页码版）：连续阅读时跳过 `<a id="pN"></a>` 和 `<!-- PDF p.N -->` 两种行即可，要页码时按 `<a id="pN">` 得到 PDF 页码。冒险书用 `bin/talkdnd adventure section` / `search` 查，输出已去掉锚点行。
2. 以下情况一律打开 PDF 对应页核对：`index.md` 列为低质量的页、正文里带 `> ⚠ Stat block check` 的**数据块**（`index.md` 的 Stat blocks 一节标 ✓ 的数据块数值自洽，可直接用 Markdown）、表格看起来错位、文字明显缺漏，以及所有**地图**（用 PDF 或 `img/` 下的图片）。
3. 书还没转到那一页：直接查 PDF。**PDF 永远是权威来源**；Markdown 只是便于检索的副本。

## 0.6.0（pdf2md）/ TalkDND 0.10.0：一书一文件夹、只生成页码版

- **输出位置**：PDF 在书文件夹里（文件夹名 = 书名的 slug）时，输出写进同文件夹的 `index/`；否则仍写旧位置 `.md/<书名>/`。slug 规则与 `talkdnd` 相同（见 `docs/layout.md`）。
- **只生成一份 Markdown**：`<书名>.pages.md`。不再生成去掉锚点的阅读版 `<书名>.md`；两份逐行相同、只差锚点行，留一份即可。`index.md` 的「Read:」和 `index.json` 的 `book` 都指向页码版。
- **升级不重转**：从 0.5.0 升级时不重做任何分块、不重新组装；只删除已有的多余阅读版（页码版存在时），改写 `index.md`/`index.json` 的指向，并把 `manifest.json` 的版本记为 0.6.0。`pdf2md verify` 若发现页码版旁还有阅读版，会提示它是多余的。`talkdnd start` 整理书库时也会删除这些阅读版，并报告节省的字节。

## 0.8.0：按页搜索、开书体检、章节起点与坏页兜底

- **按页搜索**：`bin/talkdnd adventure search "<词或短语>" [--limit 20]`（别名 `find`）在冒险的页码版 `<书名>.pages.md` 里逐行查找，每个命中打印 `p.<PDF 页码>: 该行`；精确（归一化后）命中排在前面，行首容错命中标「（模糊）」；`--limit 0` 不限条数。匹配只在查询时做归一化：忽略大小写、弯/直引号、撇号、标点、空白、连字符断行（`exam-` 换行 `ple` 当作 `example`）和连字（`ﬁ`→`fi`）；每行开头允许丢 1–4 个字符或糊掉（扫描书页边常见），所以 `known` 也能命中 `..mown`。**只读，不改动源 Markdown**；找不到页码版时提示先转换。`adventure section <词>` 查找标题也使用同一套归一化，`--list` 仍然只列标题。
- **开书体检（intake audit）**：第一次打开/转换一本书时，`pdf2md` 统计文字层字符数、0 字页、乱码率、扫描/数字版判断和书签数，给出 `text layer usable` 或 `OCR needed` 结论，写入 `manifest.json` 的 `intake` 和 `index.json` 的 `intake`/`chapters`；同一 PDF 重复运行结果不变。如果某页 CropBox 超出 MediaBox、原始文字层读成空页，体检只复制这一页，把副本的 CropBox 夹到 MediaBox 后再读文字层，不改动原 PDF，所以不会仅因这个毛病误报 `OCR needed`。`pdf2md status` 每本书显示一行 `intake: ...` 和 `page fallback N`；`adventure search`/`section` 开头也会显示简短结论。完整结果在 `index.md` 的「Intake audit」「Chapter starts」两节，以及 `pdf2md audit` 写出的 `audit.json`（含 `fallback_pages`）。
- **没有书签的书**：运行或审核时加 `--chapter-open "<每章开头都会出现的短语>"`，或设环境变量 `PDF2MD_CHAPTER_OPEN`，例如 `uv run --project tools/pdf2md python -m pdf2md run --chapter-open "The Adventure Begins"`。程序按“每页第一行以该短语开头”（文本层前 80 个字符内）找章节起点，忽略大小写、标点和空白，存进 `intake.chapter_open` 与 `intake.chapters`，并出现在 `index.md` 的「Chapter starts」。传入短语时以短语为准；没传时用书签；两者都没有就留空。已有结果在不传该参数时保留，传入不同短语会重新识别。`pdf2md audit --chapter-open "…"` 也会写入审计结果。
- **CropBox 与坏页兜底**：有些 PDF 的 CropBox 超出 MediaBox，文字层会像空页一样读不出来，整页图/OCR 也可能失败。转换前会把 CropBox 夹到 MediaBox（有交集用交集，没有交集用整页），并对坏页逐个兜底，不让一页错误中断整个分块。manifest 的页条目会记 `cropbox-clamped`、`page-fallback`（退回整页）、`page-error`（页本身读不了）、`render-failed`（图渲染失败）；`pdf2md status` 的 `page fallback N` 和 `pdf2md audit` 的 `fallback_pages` 会统计它们。看到这些标记时，查书以 PDF 对应页为准；正文仍会保留页锚点，后续页继续转换。
- **单转换进程的锁**：`worker.lock` 现在用操作系统文件锁（POSIX `flock`、Windows `msvcrt.locking`）整个进程持有，文件里同时写 `pid`、`started`、`updated` 方便诊断。没有“先删后建”的空窗：两个 `pdf2md run` 同时启动只有一个拿到锁；死进程留下的锁（pid 不存在）或超过 600 秒没更新的锁会被自动接管，活着的进程的锁不会被抢。释放锁前会先写 `released: true`（保留 pid/started）；Windows 上文件暂时删不掉时，留下的锁文件也会被下次启动视为陈旧直接接管。`pdf2md run` 拿不到锁时打印 `another pdf2md worker is running; exiting` 并返回 3；`pdf2md apply` 拿不到锁时提示下次组装时应用修正并返回 1。正常退出会删除锁文件。
- **报告文件**：`manifest.json`（分块/页状态与 intake）、`index.json`/`index.md`（标题、地图、插图、数据块检查、intake、章节起点）、`audit.json`（逐页评分与 `fallback_pages`）、`proofread-report.md`（校对改动）都只留在本地该书的 `index/` 下，不入库、不贴到 issue/PR。

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

## 0.4–0.5：阅读版/页码版、数据块检查、图片

- **两种输出，一次生成**（0.4–0.5 的做法，0.6.0 起只保留页码版）：`<书名>.pages.md`（页码版，带 `<a id="pN">` 和 `<!-- PDF p.N -->`）供查页码、审核和修正；当时还另生成去掉这两种行的 `<书名>.md`（阅读版）。审核、`audit`、`fixes/` 都按页码版工作。
- **数据块检查**：组装时解析每个数据块（AC、HP 与骰子、六项属性、CR/XP/PB），核对 HP 平均值与体质加值、属性调整值、CR→XP、熟练加值，并与本地 combat-sim 数据交叉核对。有问题时在数据块前插入 `> ⚠ Stat block check: …; check PDF p.N.`，结果也写进 `index.json` 的 `statblock_checks`。
- **图片（移植自 ConvertPDF）**：扫描页（整页一张位图）上，图多字少的页（墨迹 ≥20%、长行文字 ≤3%）存为整页图；文字页里的插图按「去掉文字格后的最大无字矩形」裁出，向外扩到碰到文字为止，含文字、过长过窄或画面被截断的裁剪直接丢弃。图片一律 WebP，插在图片上方那段文字之后（找不到就放在下方那段之前，再不行放页末），并列在 `index.md`「Illustrations cut from scanned pages」。
- **图片总览**：`uv run --project tools/pdf2md python -m pdf2md contact-sheet [--book 名称]` 在 `review/contact-NN.webp` 生成每张 16 格的缩略图总览，方便审核裁剪是否合适。
- **已有修正的安全**：PDF 变了或升级要求重做时，转换前会把 `fixes/` 复制到 `backups/<时间>-<原因>/fixes`。升级到 0.5.0 会在后台重做全部分块；`fixes/` 中的修正保留并重新应用。
