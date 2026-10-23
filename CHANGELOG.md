# 更新日志

本文件记录 TalkDND 每个发布版本的变化，按版本从新到旧排列。版本号规则见 [docs/versioning.md](docs/versioning.md)。GitHub Release 的说明文字由本文件对应的版本小节生成（`cargo run --manifest-path tools/talkdnd-dev/Cargo.toml -- release-notes <版本>`）。

每个版本分为：**新功能**、**变更**、**修复**、**更新说明 / 迁移**、**已知限制**。括号里的 `#N` 是合并的 PR。

## v0.8.0 — 2026-10-23

### 新功能
- **按场景读冒险书**：新增 `bin/talkdnd adventure section <章节|地点|关键词> [--list] [--reason "<原因>"]`。程序在本地转换好的冒险 Markdown（`Adventures/<slug>/.md/<书名>/<书名>.md`）里按 Markdown 标题切分，只打印标题匹配的那一节（含它的子标题），不再把整本书倒出来。`--list` 只列标题，不显示正文。章节是否“超前”按存档的冒险进度轨和当前章节判断：超出当前进度默认拒绝；确有必要时加 `--reason`，程序会打印该节并在会话日志记「超前阅读」。标为 DM-only 的小节同样要 `--reason`。没有本地 Markdown 时给出转换/设置冒险的提示。`adventure section` 的标题匹配也做了查询时归一化（大小写、引号、标点、连字、断行连字符），不修改源 Markdown。
- **冒险按页搜索**：新增 `bin/talkdnd adventure search "<词或短语>" [--limit 20]`（别名 `find`），在页码版 `<书名>.pages.md` 中逐行查找，每个命中带 `p.<PDF 页码>`。查询时归一化大小写、弯/直引号、撇号、标点、空白、连字（`ﬁ`→`fi`）和跨行连字符（`exam-` + `ple`→`example`），每行开头允许丢 1–4 个字符（扫描页边遮挡），因此 `known` 能命中 `..mown`。只读 Markdown，不重写书；`--limit 0` 不限条数。
- **pdf2md 开书体检与章节起点**：转换一本书时统计文字层字符数、0 字页、乱码率、扫描/数字版判断与书签数，给出 `text layer usable` / `OCR needed` 结论，写入 `manifest.json`、`index.json`，并在 `index.md` 的「Intake audit」「Chapter starts」两节展示；重复运行结果不变。CropBox 超出 MediaBox 的页会先复制单页并夹取副本的 CropBox 再读文字层，原 PDF 不动，不会仅因此误报 `OCR needed`。`pdf2md status` 与 `adventure search`/`section` 会显示简短结论。没有书签的书可用 `--chapter-open "<每章开头短语>"` 或 `PDF2MD_CHAPTER_OPEN` 识别章节起点，结果存进 intake 元数据；`pdf2md audit --chapter-open` 同样生效。命令示例：`uv run --project tools/pdf2md python -m pdf2md run --chapter-open "The Adventure Begins"`、`uv run --project tools/pdf2md python -m pdf2md audit --book "书名" --chapter-open "The Adventure Begins"`。
- **玩家已知清单 `bin/talkdnd known`**：汇总 `reveals.yaml` 的已揭示事实、`handouts.yaml` 已展示资料图、`identified.yaml` 已识别怪物/物品、已出场或隐藏面已揭示的原创内容、队伍角色卡基础信息、当前章节与地点。`bin/talkdnd scene "<地点>"` 把当前场景写进 `progress.yaml`、记日志并自动打印完整 `known`；`bin/talkdnd start` 的摘要里也带压缩版。
- **秘密词与防剧透检查**：
  - `bin/talkdnd secret add <词> [--chapter N] [--note "…"]`、`secret list`、`secret remove <词>`，手动秘密词只存在 `Saves/<存档>/secret-terms.yaml`（已 gitignore）。
  - 自动来源：`dm-secrets.md` 与 `dm-plan.md` 里的 `- term: X` / `[[X]]`、当前进度之后的章节标题、DM-only 小节的标题/`[[词]]`/连续大写英文专名、原创内容 `--hidden`。已出现在 `reveals.yaml` 里的词自动排除。
  - `bin/talkdnd guard "<草稿>"`（或 stdin / `-`）做不区分大小写的子串匹配，每个命中打印「剧透词：X（建议改写为玩家已知的说法；来源：…）」；有命中退出码 1，干净为 0。清单缓存在 `secret-terms.cache.json`，源文件变化后自动失效，检查很快。
- **怪物/物品名称遮蔽**：
  - 新增 `bin/talkdnd identify <东西> [--how check|reveal|lore] [--reason "…"]`，把识别记进 `Saves/<存档>/identified.yaml`；`bin/talkdnd identify --appearance <东西> "<描述>"` 可设置识别前对玩家使用的描述。
  - combat-sim 的 `status`、`log`、`simulate` 支持全局 `--player`：未识别怪物显示 appearance 或默认的「体型 + 类型」描述（如“一只大型野兽”），已识别的恢复真名，PC/同伴 NPC 永远显示真名。DM 视图不加 `--player`。
  - 存档位置从 `--campaign` 指向的 `<存档>/combat/campaign.yaml` 推导，也可用 `--save Saves/<存档>` 或环境变量 `TALKDND_SAVE` 指定。
- **原创内容隐藏面**：`bin/talkdnd original add … --hidden "<隐藏面>"`，隐藏面只写 `original.yaml`（DM-only），并进入 `guard` 的秘密词清单。`bin/talkdnd reveal add --original O-001 [--source "…"]` 记录隐藏面已揭示（自动加 `original:o-001` 标签；不写事实文本时自动生成一条揭示记录）。`bin/talkdnd original plan` 创建 `Saves/<存档>/dm-plan.md` 长期计划模板（只给 DM，已 gitignore）。`bin/talkdnd check` 会报告已登记隐藏面（或其 `[[词]]`）原样出现在会话日志里、却没有揭示记录的情况。
- **骰子显示开关**：`bin/talkdnd settings` 查看，`bin/talkdnd settings dice on|off` 切换；默认开，局中可改，每次改动写会话日志，`bin/talkdnd start` 显示当前设置。关闭后 DM 只叙述结果，不报骰值/DC/修正/HP 数字；所有骰子仍由程序掷并记录。
- **掷骰日志**：`Saves/<存档>/rolls.log` 追加记录 `talkdnd check`、`chargen roll`、起始金币、`npc random`、`level-up --hp roll`、`combat-sim roll`，以及 combat-sim 战斗状态命令（`attack`、`cast`、`use`、`save`、`check`、`damage`、`init`、`deathsave`、`rest` 等）中带骰式的行。只含骰式的行会写入，条件/叙述行不会，战斗 state 日志不变。`bin/talkdnd log rolls [--last N]` 随时查看。`combat-sim roll` 加 `--save` 或 `TALKDND_SAVE` 即写入同一份日志。
- **选项写法与防剧透文档**：新增 `LetsPlay/docs/option-writing.md`，给出“只从 `known` 出发、不报未公开 DC/做法、未发现内容不进选项、模糊情报保持模糊”的规则，以及三个改写自劫案式冒险的正误示例（只揭示机关被触发却报 DC、把未发现的暗门写进选项、拿粗略草图认定哪扇门有警报）。`LetsPlay/AGENTS.md` 压到 150 行并加入对应摘要；`docs/anti-spoiler.md`、`docs/dice.md`、`docs/combat.md`、`docs/deviation.md`、`docs/layout.md` 同步更新。

### 变更
- 存档 schema 升到 **5**：`save-meta.yaml` 新增 `settings: { dice_display: true }`。旧存档（schema 4 及以前）在 `talkdnd start` / `talkdnd migrate` 时先整份备份到 `Saves/_backups/`，再迁移、写会话日志；骰子显示默认为开，可随时切换。重复运行不会重复迁移。
- `original.yaml` 台账版本升到 **2**：新增可选 `hidden:` 字段。版本 1 的台账仍可直接读；迁移或下一次 `original` 写入时升到 2，旧条目 `hidden` 为空，不丢数据。
- `progress.yaml` 新增可选 `scene:` 字段（当前场景/地点），旧存档无需迁移。
- 所有程序版本统一升到 0.8.0（talkdnd、combat-sim、talkdnd-dev）。
- combat-sim 的 creature 数据新增可选 `appearance:` 字段（`deny_unknown_fields` 下仍兼容旧数据），并与 `base:` 继承合并。

### 修复
- 同一行被不同字体拆成多个 span 时，先按顺序拼接再分词、识别标题和统计词数，修复 `exam` + `ple` 被当成两个词、标题识别丢失的问题（fallback、figures、headings、rooms 共用 `textutil.line_text`）。
- pdf2md 把超出 MediaBox 的 CropBox 夹到 MediaBox 后再提取、OCR 和渲染整页图；单页读取或渲染失败不再中断整个分块。开书体检遇到同类页时复制单页、夹取副本的 CropBox 再读文字层，原 PDF 不变。`cropbox-clamped`、`page-fallback`、`page-error`、`render-failed` 写进 manifest 页条目，并计入 `pdf2md status` 的 `page fallback` 与 `pdf2md audit` 的 `fallback_pages`。
- pdf2md worker 锁改为操作系统文件锁（POSIX `flock`、Windows `msvcrt.locking`），锁文件同时记录 pid 与时间戳，不再有“先删后建”的空窗；活进程的锁不会被抢，死 pid 或超过 600 秒未更新的旧锁自动接管。释放前先写 `released: true`，Windows 上删除失败留下的文件也会被下次启动当作陈旧锁接管。
- `known` 与 `start` 摘要里的队伍行改为读取结构化角色卡的职业（不再显示 `?`），摘要中子项缩进显示。
- `guard` 的英文秘密词要求整词匹配（`Inn` 不再命中 `inner`），并去掉转换残留的 `_`/`*` 标记；单个英文单词的章节标题（如 `Wharf`）太泛，不再自动当作秘密词（DM-only 小节里的专名照常收集）。秘密词缓存版本升级，旧缓存自动失效。
- `adventure search` 先列出精确（归一化后）命中，再列出行首容错命中，后者标注「（模糊）」。
- combat-sim 玩家视图里多个同类未识别怪物带编号区分（如「一只中型野兽 #1」「#2」）。
- 建卡、战斗、迁移、校验、资料图和偏离度流程保持兼容（原有测试全部通过，并新增覆盖：guard 命中/干净/已揭示排除/stdin/退出码、名称遮蔽与识别、`known` 内容、超前阅读拒绝与 `--reason` 日志、骰子设置切换、schema 4→5 迁移、原创 `--hidden` 进入 guard 与日志泄漏检查、模糊搜索页码、intake/章节起点、CropBox 与坏页兜底、锁并发与接管）。

### 更新说明 / 迁移
- **覆盖式更新**：下载新发布包，解压到临时位置，把包内 `TalkDND/` 文件夹里的全部内容复制到现在的游戏文件夹，覆盖同名文件。存档目录原样保留。不要把旧包的 `TalkDND/` 整个替换掉（会丢存档、规则书和冒险书）。
- 更新后第一次 `talkdnd start` 会生成 schema 4→5 的迁移备份，并在会话日志写「存档结构迁移 schema 4→5 …；骰子显示默认开」。确认无误后可删除 `Saves/_backups/` 里的旧备份。
- 新文件 `identified.yaml`、`secret-terms.yaml`、`rolls.log`、`dm-plan.md` 都只在存档里按需创建，均已 gitignore；不要提交。
- 已经写好的旧命令不用改；`original add` 不带 `--hidden` 时行为与 0.7.0 相同。

### 已知限制
- `adventure section` 依赖本地转换后的 Markdown，并且只按 Markdown 标题切分；PDF 没转换、标题层级混乱或一本书有多个 `.md` 子目录时，可能找不到想要的小节。先看 `--list`，必要时查 `index.md` 或 PDF。
- `adventure search` 是逐行子串/前缀模糊匹配，不是语义检索；只查页码版，扫描页 OCR 把词认错时仍可能漏。命中多时用 `--limit`，`--limit 0` 会输出全部命中。
- 章节起点短语按“每页第一行开头”匹配，短语太常见会误把正文页当章节；先 `--list` 或看 `index.md` 的「Chapter starts」核对。CropBox 夹取只作用于转换/渲染和体检时的单页副本，原始 PDF 不会被修改。
- “超前章节”靠进度轨名称、章节编号和当前队伍等级推断；没有进度轨的自制冒险不会阻止任何章节，DM 仍要自觉按场景读。
- `guard` 是子串提醒，不是语义判断；改写过的泄密说法可能漏掉，查到同名词也不一定是剧透。已揭示的词靠 `reveals.yaml` 排除，DM 忘了记录时仍会误报。
- 秘密词中的中文只从 `- term:`、`[[…]]`、章节标题和原文中的标记收集；不会自动从任意中文段落里猜专名。需要检查的中文词请显式标记。
- 名称遮蔽只覆盖 combat-sim 的 `status`/`log`/`simulate` 文本和 `--player` JSON；攻击、伤害等逐条命令的输出仍是 DM 视图。不要把 DM 视图直接投给玩家。
- 骰子显示关闭只是叙述约定，程序不会阻止 DM 说出数字；所有掷骰仍会记录在 `rolls.log`（战斗状态命令只记录带骰式的行），DM 不能以“关显示”为由跳过掷骰。
- `identified.yaml`、`secret-terms.yaml`、`rolls.log` 不在 `integrity.yaml` 的指纹清单里；它们是 DM 工具数据，手改不会触发存档校验警告。

## v0.7.0 — 2026-10-22

### 新功能
- **每个存档的「剧本偏离度」**：开新档时用一个档位和一个比例锁定 DM 可以离冒险书多远，之后不能修改（没有修改命令）。
  - `scope 0`：不偏离，只能补必要空白（给未命名 NPC 起名、对白措辞、场景描述）；补空白不算原创。
  - `scope 1`：可以加入原创遭遇、NPC、物品、地点；稀有度上限 `uncommon`。
  - `scope 2`：在 1 的基础上可以加入完整支线；稀有度上限 `rare`。
  - `ratio` 是每章原创内容占「原创 + 正典进度」的比例上限，整数 5–25%；scope 0 不设比例。
  - 开档示例：`bin/talkdnd new --ruleset 2014 --scope 2 --ratio 15`。交互终端不带参数时程序用中文询问；非交互环境默认 scope 0 并打印提示。scope 3 及以上会被明确拒绝。
- **原创内容台账 `bin/talkdnd original`**（只给 DM）：
  - `original add --type <encounter|npc|item|location|sidequest> --name "<名字>" [--rarity <common|uncommon|rare|very-rare|legendary>] [--note "<备注>"] [--hook player]`
  - `original list`：列出本档全部原创条目（编号 `O-001` 起）。
  - `original status`：显示档位、比例、允许类型、稀有度上限（档位与队伍等级取较低者）、当前章节、本章正典进度、本章原创数与占比、剩余可登记项、全战役总数与占比。
  - 规则：先登记再引入；物品类型必须给 `--rarity`；超类型、超稀有度或超预算都会拒绝，预算不足时提示**回到主线**。采用玩家钩子加 `--hook player`，同样算原创、占预算。
- **预算按章计算**：原创 + 1 <= floor(ratio% × (原创 + 1 + 本章正典数))。正典数取 `progress.yaml` 的 `xp_log` 条数 + `reveals.yaml` 的已揭示事实数；本章从最近一次 `level-up --milestone` 起算（含该条），没有里程碑时按全战役累计。全战役总数只展示，不作为拒绝依据。规则和示例见 `docs/deviation.md`。
- **`char item add` 来源检查**：规则数据里的武器、护甲、盾牌和常见装备直接通过；其他名称需要 `--canon`（DM 查过书）或 `--original O-xxx`（本档已登记的物品原创），否则拒绝并给出提示。原创标记只存在 `sheets/<id>.yaml`，不会出现在给玩家看的角色卡、资料图或摘要里。
- **`talkdnd check` 扩展**：报告 `save-meta.yaml`（含 `deviation:`）和 `original.yaml` 在工具以外被改、台账条目类型/稀有度超出 scope 或队伍等级上限、角色卡物品标了原创但台账没有对应条目；旧存档迁移继续保留原有的手改标记。
- **`talkdnd start` 摘要新增「剧本偏离度（只看自己）」**：档位、比例、允许类型、稀有度上限、当前章节、本章预算和全战役总数，方便开场前核对。

### 变更
- 存档 schema 升到 **4**：`save-meta.yaml` 新增 `deviation: { scope, ratio }`。`talkdnd start` / `talkdnd migrate` 会先把整份存档备份到 `Saves/_backups/`，再迁移并写会话日志；重复运行不会重复迁移。
- 完整性清单（`integrity.yaml`）现在也跟踪 `save-meta.yaml` 和 `original.yaml`；旧清单里没有这两个文件时，第一次工具写入会把它们纳入基线，不会误报。
- 所有程序版本统一升到 0.7.0（talkdnd、combat-sim、talkdnd-dev）。
- 文档：新增 `LetsPlay/docs/deviation.md`；`AGENTS.md` 增加「剧本偏离度（必须遵守）」；建卡文档、存档结构、校验文档同步更新。

### 修复
- 无行为修复；本版主要是新功能与文档。已有建卡、起始装备、背景装备、`char` 物品/装备、迁移和校验流程保持兼容（原有测试全部通过，并新增覆盖）。

### 更新说明 / 迁移
- **旧存档自动变成 scope 0**：升级后第一次 `talkdnd start`（或 `talkdnd migrate`）会给旧存档写 `deviation: { scope: 0, ratio: null }`，表示「不偏离」。旧存档不能登记原创；想要原创只能开新档。
- **覆盖式更新**：下载新发布包，解压到临时位置，把包内 `TalkDND/` 文件夹里的全部内容复制到现在的游戏文件夹，覆盖同名文件。存档目录原样保留，不需要手工改 `save-meta.yaml`。不要把旧包的 `TalkDND/` 整个替换掉（会丢存档、规则书和冒险书）。
- 更新后第一次运行会生成迁移备份；确认无误后可以删除 `Saves/_backups/` 里的旧备份。

### 已知限制
- 章节根据队伍最低等级和进度轨推断；自制冒险没有 `track` 时全部算「未分章」，预算按整场战役累计。
- 正典计数是记录条数（经验/里程碑 + 已揭示事实），不是剧情重要性；需要 DM 按正常流程记录，不能为了腾预算灌水。
- 名称来源检查只覆盖规则数据里的武器、护甲、盾牌和常见装备；魔法物品、书里特有装备需要 `--canon` 或登记为原创。
- 偏离度没有修改命令，这是有意为之；开档时务必先向玩家解释清楚再执行 `new`。
- 存档校验用于防手滑，不是防作弊的安全边界；`original.yaml`、`dm-secrets.md` 和摘要都只给 DM 看。

## v0.6.1 — 2026-10-22

### 修复
- **游戏文件夹可以放在任意位置、叫任意名字**：以前程序要求玩家保留 `TalkDND/LetsPlay` 这样的目录结构，或者必须在 `LetsPlay/` 里运行，否则找不到游戏文件夹。现在 `talkdnd` 按以下顺序找游戏文件夹，与当前目录和文件夹名字都无关：
  1. `--root <文件夹>`（相对路径按当前目录解析）；
  2. 环境变量 `TALKDND_ROOT`（不是游戏文件夹时直接报错）；
  3. 程序所在 `bin/` 的上一级（解压出来的包就是这种情况，从哪个目录运行都行）；
  4. 当前目录及其上级（开发仓库里的 `LetsPlay/` 子目录仍然支持）。
  游戏文件夹以根部新增的标记文件 `.talkdnd-root` 识别（旧的 `AGENTS.md` + `Saves/` 判断仍然有效）。路径里有空格或中文也没关系。
- **combat-sim 同样不依赖当前目录**：`--campaign`（或 `COMBAT_SIM_CAMPAIGN`）给的是相对路径、而当前目录下没有这个文件时，会到游戏文件夹（`TALKDND_ROOT`，或程序所在 `bin/` 的上一级）里找，所以 `bin/combat-sim --campaign Saves/<存档>/combat/campaign.yaml …` 在任何目录下都能用。存档清单里的库路径本来就是相对清单文件的，不受影响。
- pdf2md 总是在游戏文件夹里以 `uv run --project tools/pdf2md` 启动，不依赖当前目录（原本如此，本次核对确认）。

### 变更
- **发布包的顶层文件夹改名为 `TalkDND/`**（以前是 `LetsPlay/`，那是开发仓库里的目录名）。里面的内容是自包含的完整游戏文件夹，可以随意移动和改名。
- 发布流程新增冒烟测试：把刚打好的包解压后改名为带空格和中文的文件夹，从另一个目录运行 `talkdnd new`、`chargen status`、`start` 和 `combat-sim roll`，任何一步失败都不发布。
- 新增集成测试 `tools/talkdnd/tests/portable.rs`：把最小游戏文件夹复制到任意命名的临时目录，从别的目录运行建档、建卡、接档和战斗内容检查，并确认当前目录里不会写入任何东西。
- 文档：`AGENTS.md`、`docs/` 各篇、`Rules/defaults/` 裁定、各 README 的路径说明都改成“相对于游戏文件夹”，不再写死 `LetsPlay/`。README 的解压和更新步骤按新的包结构重写。

### 更新说明 / 迁移
- 已经在玩的：照常更新。下载新包，解压到临时位置，把 `TalkDND/` 文件夹**里面的全部内容**复制到你现在的游戏文件夹（即使它还叫 `LetsPlay`，或者被你改过名），覆盖同名文件。这样会带上新的 `.talkdnd-root` 标记文件。存档不需要迁移。
- 新玩家：解压到任意位置，文件夹随意改名，在里面启动 Agent 即可。
- 请不要删除游戏文件夹根部的 `.talkdnd-root`。

### 已知限制
- 如果把 `bin/` 里的程序单独复制到别处运行，程序就找不到游戏文件夹了，这时需要 `--root` 或 `TALKDND_ROOT`。
- 通过符号链接调用程序时，按链接指向的真实位置找 `bin/..`。
- combat-sim 不加 `--campaign` 时仍然不读取存档（与以前相同）；DM 手册里的命令都带 `--campaign`。
- Windows 包没有跑解压冒烟测试（只在 Linux、macOS 上跑）；Windows 上的路径查找由同一套代码和单元测试覆盖。

## v0.6.0 — 2026-10-22

### 新功能
- **覆盖式更新**：把新版发布包直接解压覆盖到旧的 `LetsPlay/` 文件夹即可更新。发布包里只有程序文件和说明，不含任何会覆盖用户数据的文件（存档、规则书和冒险书、本地导入 `local/`、`fixes/`、`handouts.yaml`、用户的 `Rules/rulings*.md`、`.talkdnd/` 都不在包里，打包时自动检查，命中即发布失败）。
- **存档结构版本与自动迁移**：每个存档有 `Saves/<存档>/save-meta.yaml`（结构版本、上次使用的程序版本、迁移记录）。当前结构版本为 3。`talkdnd start` 发现存档结构比程序旧时，先把整个存档备份到 `Saves/_backups/<存档>-<时间>-schema<N>/`，再依次迁移（1→2：Markdown 角色卡转为 `sheets/<id>.yaml` 结构化数据；2→3：写入 `save-meta.yaml`，给 `progress.yaml`、`reveals.yaml`、`party-state.yaml` 加版本字段），在会话日志写一行迁移记录，并更新完整性清单。迁移本身不会被当成手改，但迁移前已经被标为手改的文件仍然保持标记。重复运行不会重复迁移。存档结构比程序新时会报错，提示先更新程序。
- 新命令 `talkdnd migrate [--dry-run]`：手动预览或执行迁移。
- **本地导入数据版本**：`Rules/chargen/local/imported-<规则>.yaml` 记录导入器版本（当前为 2）；版本过旧或书有更新时，`start` 自动重新导入。新导入的子职业和背景数量比旧的少时（例如某本书的 Markdown 缺失或识别变差），旧文件保留为 `.bak` 并给出警告。
- **规则裁定不再被更新覆盖**：仓库自带的裁定移到 `Rules/defaults/rulings*.md`，你的 `Rules/rulings*.md` 不在发布包里。缺失时 `start` 从默认值复制；默认值有更新而你的文件改过时，只提示你去合并，不会覆盖。
- **旧版遗留文件清理**：发布包带 `LetsPlay/.talkdnd-files.txt`（随包文件清单和 sha256）。`start` 对比上一版的清单，把新版已删除、且你没改过的旧程序文件移到 `.talkdnd/obsolete/<时间>/`，不直接删除；改过的只提示。
- **发布说明由更新日志生成**：`talkdnd-dev release-notes <版本>` 输出 `CHANGELOG.md` 对应小节和下载说明，发布流程直接用它作为 Release 正文。
- `TALKDND_TIMING=1` 时 `talkdnd start` 在 stderr 输出各阶段耗时。
- 新增 `CHANGELOG.md`（本文件）和 `docs/versioning.md`（版本号规范：`0.0.x` 小修小补，`0.x` 功能增删或大改，`x.0` 大版本；各位逐位递增，不按十进制进位）。

### 变更
- **`talkdnd start` 提速**：战斗内容检查的结果缓存在 `Saves/<存档>/.cache/content-check.json`，以 combat-sim 程序和它读取的每个文件（大小+修改时间）为键，没变化就不再启动 combat-sim；去掉了每次启动都要跑的 `uv --version` 探测（直接启动 pdf2md，缺 uv 时报告“未启动”）；会话日志追加不再读取整个文件；每条命令只做一次完整性快照。
- 升级（`level-up`）后自动重算法术位、契约法术位、职业资源上限和施法攻击/DC，当前值按新上限截断；结构化角色卡同样适用。
- README 新增「更新到新版本」步骤。

### 修复
- **存档写入改为原子写**：进度、角色卡、建卡记录、已揭示信息、资料图登记、本地导入、队伍状态、完整性清单等都先写临时文件再替换，中途断电或崩溃不会留下半个文件。pdf2md 的状态文件写入也会先落盘再替换。
- 重新运行 `chargen sheet` 不再清空游戏中积累的数据（物品、备注、DM 备注、专长、语言、工具、战斗风格、钱币），只刷新建卡推导出的字段。
- 角色和 NPC 名字转换成文件名时统一做安全处理，id 只允许小写字母、数字和连字符，任何路径都不能跑出存档目录（例如 `../../evil`）。
- 等级是 -1、0、25 之类的异常值或生命骰面数为 0 时，不再崩溃，给出明确错误。
- 队伍短休先检查生命骰够不够，再扣除；不会扣到一半出错。
- `handout build` 时如果某张图找不到了，保留原登记和标注（标为 `missing`）并警告，不再丢掉。
- `pdf2md status` 遇到格式不对的清单时显示错误行，不再整体中止。
- `chargen set gold` 的提示改成真实存在的命令。
- 优势和劣势同时存在时一律抵消为普通掷骰（包括先攻），不再取决于先后顺序。
- 抗性和易伤按伤害类型合计后只计算一次（两次 1 点挥砍对抗性 = 1，而不是 0）。
- 手动 `heal` 会计入目标的“受到治疗”加成。
- `char damage` 把 HP 降到 0 时自动加上“昏迷”等濒死状态；治疗、死亡豁免掷出 20、长休、`combat-sim party set-hp` 回到 HP>0 时自动移除。
- 角色卡和完整性清单的版本号会被检查，比程序支持的新时报错并指出文件。

### 更新说明 / 迁移
- 先存档并退出正在运行的 talkdnd / combat-sim（Windows 上运行中的 `.exe` 不能被覆盖），再把新包解压到 `LetsPlay/` 的上一级目录并选择全部覆盖。不需要挑文件，也不需要先删旧版。
- 第一次 `talkdnd start` 会自动备份并迁移存档、按需重新导入本地书籍数据、复制缺失的默认裁定；输出里会说明做了什么。备份在 `Saves/_backups/`，确认没问题后可以自行删除。
- 如果你改过旧版的 `Rules/rulings*.md`：这些文件不会再被覆盖。新版默认裁定在 `Rules/defaults/`，`start` 提示时可自行对照合并。

### 已知限制
- 旧版遗留文件只能在 v0.6.0 之后的更新中识别（v0.6.0 之前的发布包没有文件清单）。
- 自动迁移只覆盖已知的旧结构；手改过的存档迁移后仍会被完整性检查标出，需要 DM 确认。
- 审阅中的以下问题本版未处理：暴击范围（19–20）、力竭的实际效果、场外检定不计状态、`charsheet` 重算 AC 时不考虑职业无甲防御和护甲熟练、`rest all` 跨队伍不是原子操作、`char set class` 不清理旧熟练和攻击、冒险章节数据写死在程序里、休息和死亡豁免的重复代码合并、pdf2md 锁接管的竞争问题。

## v0.5.0 — 2026-10-22

### 新功能
- **资料图（handout）**（#91）：把本地 pdf2md 转换出的冒险书图片（地图、信件、物品图）给玩家看。
  - 每本转换后的书一个登记表 `Adventures/…/.md/<书名>/handouts.yaml`（本地、gitignore），由 pdf2md 的 `index.json`（地图图注、插图、整页图）和 `img/` 生成。
  - **默认全部「仅城主」**，只有图注写明玩家版（Player Version）的地图自动标为玩家可见。
  - `talkdnd handout show <id|关键词> [--crop x,y,w,h] [--png]`：只导出玩家可见的图到 `Saves/<存档>/handouts/`，打印文件路径和 Markdown 图片行，同时写入已揭示账本和会话日志；城主图一律拒绝。
  - `handout list [--dm]`、`handout mark <id> --player|--dm [--crop …]`、`handout build`。登记了裁剪区域后只能导出这一块。
  - AGENTS.md 新增资料图规则（何时展示、绝不展示城主图、不同宿主的展示方式），新文档 `docs/handouts.md`。

### 修复
- **迁移后的 AC 叠加防御风格**（#91）：旧角色卡迁移时，原 AC（已含防御 +1）被当作无甲基础值，`set fighting-style defense` 又加了 1（19 → 20）。现在迁移会把原卡写到的护甲和盾牌变成已装备物品（能算出原 AC 时），AC 由物品计算；算不出来就保留原 AC 并标记 `ac_base_has_style`，不再重复加防御。

### 更新说明 / 迁移
- 已经手动加了护甲物品绕过此问题的存档不受影响，不需要改。
- 资料图需要冒险书已经用 pdf2md 转换过。

### 已知限制
- 程序无法判断图片内容；能否给玩家看完全靠城主判断，所以默认仅城主。
- 图注、区域来自 pdf2md 的启发式结果；整页图常带正文，多半要裁剪。裁剪只能是矩形。
- 原卡写了无法识别的护甲时，AC 固定为原值，直到用 `char … item add/equip` 加上真实护甲。

## v0.4.0 — 2026-10-22

### 新功能
- **角色卡字段对齐官方角色卡**（#89）：战斗风格（2014 职业特性 / 2024 专长）、背景特性/专长、工具、语言、背景装备与金币、职业起始装备（2014 装备组或掷金币，2024 方案 A/B/C）、专长、性格特点/理想/羁绊/缺点、阵营。`chargen set fighting-style | class-equipment | background-equipment | languages | tools | feats | traits | ideals | bonds | flaws | alignment`；装备和金币按数据自动发放并记账。
- **战斗风格效果进入 combat-sim**：巨武器战斗（2014 重骰 1–2 一次；2024 把 1–2 当 3；双手武器和可双手的多用武器）、防御（着甲 +1 AC）、对决（单手近战 +2 伤害）、箭术（远程攻击 +2 命中）。新增攻击字段 `reroll_damage_below`、`damage_die_min`。
- **结构化角色数据与 `talkdnd char` 命令**：`Saves/<存档>/sheets/<id>.yaml` 是唯一数据源，实时数值在 `combat/party-state.yaml`（与 combat-sim 共用），Markdown 角色卡由程序渲染。命令：伤害/治疗/临时 HP、资源与法术位使用/恢复、短休/长休与生命骰、状态、力竭、激励、死亡豁免、多币种金币、物品增删/装备/同调（AC 与攻击自动重算）、弹药、笔记、DM 备注、`set`、原子化 `batch`；每条一行输出，自动写日志并保持完整性清单干净。
- **完整规则书优先**：本地导入同时提取背景；`talkdnd start` 在本地数据缺失或过旧时自动导入，并把完整规则书列为首要规则来源（SRD 只兜底）；`data status` 显示规则来源。
- 新文档 `docs/character-data.md`；AGENTS.md 开新局流程 1–14 步、「角色卡只能用 `char` 命令改」。

### 更新说明 / 迁移
- 旧角色卡第一次用 `char` 命令时自动迁移为结构化数据，正文原样保留，数值不变。用 `char <角色> set …` 补缺的栏目（战斗风格、背景等）。

### 已知限制
- 背景导入是启发式的，2024 版部分背景会漏掉（OCR 质量差）。
- 保护、双武器等风格只记为备注，由 DM 处理；对决 +2 不检查副手。
- 占位装备项需要手动命名；怪物数据仍来自 SRD 导入。

## v0.3.0 — 2026-10-22

### 新功能
- **桌面诚信与防作弊**（#87）：AGENTS.md「桌面诚信」一节（场外指令、作弊、意图与结果、开局前约定的房规存进存档）；`docs/table-integrity.md`。
- **存档完整性清单**：`integrity.yaml` 记录 `progress.yaml`、角色卡、`combat/*.yaml`、`reveals.yaml`、`house-rules.md` 在每次程序写入后的指纹；`start`/`check` 提示被手改的文件；`talkdnd integrity status|accept --reason`。
- **`talkdnd check`（无参数）对账**：HP、金币、物品、XP 的变化与会话日志、`xp_log` 对账。
- **防剧透**：`docs/anti-spoiler.md`（信息纪律、按需阅读、回复前自检、说漏了怎么办）和已揭示账本 `talkdnd reveal add|list|check`；摘要显示最近揭示。

### 更新说明 / 迁移
- 旧存档第一次运行时自动建立完整性基线。

### 已知限制
- 完整性清单只能发现改动，不能阻止；对账依赖日志行格式。

## v0.2.0 — 2026-10-21

### 新功能
- **pdf2md 0.4.0 阅读版与分页版**（#75）：`<书>.md` 阅读版（无页锚点）和 `<书>.pages.md` 查页版；索引链接到分页版；阅读版漂移时校验失败；真正重转前先备份 `fixes/`。
- **pdf2md 0.4.1 数据块检查**（#78）：怪物数据块可疑处加 ⚠ 标记。
- **pdf2md 0.5.0 图片阶段**（#80）：从扫描页切出插图、整页图，`contact-sheet` 生成缩略图总览。

### 变更
- **移除可打印角色卡**（#81）：删除 `talkdnd sheet export`（PDF/HTML）和 `sheet blank`（#76 曾加入 v2 版式），依赖 krilla/lopdf/ttf-parser 一并去掉；DM 不再提供打印卡，`chargen sheet`（Markdown）保留。
- 文档：pdf2md 阅读版/分页版、数据块检查、图片阶段（#83）。

### 已知限制
- 图片阶段的插图切割是启发式的；扫描质量差的书效果有限。

## v0.1.7 — 2026-10-21

### 新功能
- **混合制成长**（#73）：事件给 XP、里程碑升级。`xp add [N] --category --reason [--difficulty] [--again]`，`xp_log` 记录、重复拒绝；2014 DMG / 2024 DMG 门槛与预算；XP 最多领先角色卡一级，超出部分记为暂扣；`level-up <角色|all> --milestone --reason`；`xp show` 显示金库劫案进度。

### 修复
- 只加载与战役规则一致的 `2014/`、`2024/` 子文件夹（#71），修复 2024 存档读到 2014 劫案数据导致的 95 个错误。

### 更新说明 / 迁移
- 旧 `progress.yaml` 原样可读（新字段默认为空）。

## v0.1.6 — 2026-10-21

### 新功能
- `talkdnd sheet export`：可填写的 A4 PDF/HTML 角色卡（#68）。（v0.2.0 已移除）

## v0.1.5 — 2026-10-21

### 修复
- combat-sim 加载器扫描（#67）：冒险目录有 `combat/` 时只加载它；跳过隐藏目录（`.md/`、`.git`）；目录里的非内容文件改为警告；带 `common/` 的嵌套库只通过 `library:` 加载。
- `start` 摘要每本书只列 `<书>.md` 和 `index.md`。

## v0.1.4 — 2026-10-21

### 新功能
- **XP、升级与战利品**（#63）：`xp add/show`、`level-up`（HP、熟练、生命骰、豁免、技能、命中自动更新，新特性与子职提示）、`loot add|spend|give|split|show`（多币种，个人钱袋与队伍公共池）。
- **SRD 5.1 批量导入与快速遭遇**（#61）：`combat-sim content import-srd`，生成 331 个怪物、302 个法术。
- **2014 PHB 子职导入**（#55、#59）：本地书中识别 28 个非 SRD 子职。
- **pdf2md 0.3.0**（#53）：校对规则、图片原位提取、地图与房间索引；冒险书缺口修复（#57）：OCR 补房间标题、朗读框转引用。
- **持久队伍状态**（#46）：`combat/party-state.yaml`、`combat-sim party show|rest|set-hp|sync`。
- **`talkdnd check` 检定**（#48）、建卡校验（等级、子职、护甲、法术数量、种族特性，#49）。

### 修复
- 规则修正（#44）：非本回合施法/使用只允许反应；短休花生命骰；长休结束专注、恢复一半生命骰；新增 `npc` 类型（同伴会做死亡豁免）。
- 生物死亡时结束自己的专注并移除他人专注造成的状态（#51）。

## v0.1.1 — 2026-10-21

### 新功能
- **talkdnd 游玩命令行**（#37）：`start` 接档摘要、`new`、可续接的 `chargen`（掷骰 6×4d6 去最低、记日志）、`npc random/sheet`、`checkpoint`、`log`、`options`。
- **精简 AGENTS.md 与玩家主导建卡**（#39）：每回合只做一个决定，玩家自己选。
- **建卡生成攻击、职业特性与资源；本地书籍数据导入**（#41）：`data status|import`。
- **pdf2md**（#18 起）：后台 PDF→Markdown，OCR、增量分块、审阅（#29）、标题修正（#22–#27）、审阅软门槛（#35）等。
- **2014 战斗数据**（#33）：SRD 守卫、活化盔甲、拟形怪，五个法术，`support` AI。
- **统一掷骰**（#15）：`combat-sim roll`，DM 掷骰规则；即兴叙事指引（#16）。

### 修复
- 测试偶发失败（#14）、Windows 路径（#30）、审阅召回率对行尾连字符（#31）。

## v0.1.0 — 2026-10-20

### 新功能
- 首个发布（#2、#4、#7、#8、#10）：`LetsPlay/` 游玩文件夹（DM 手册、战斗数据、规则裁定）和预编译 combat-sim；Linux x86_64 / Windows x86_64 / macOS arm64 三个平台包与 sha256；CI（fmt、clippy、三平台测试、cargo-deny、泄露检查、yamllint、链接检查）；项目章程。
- 发布包只含 git 跟踪的 `LetsPlay/` 文件；打包时拒绝 PDF、`dm-secrets.md`、存档和书籍内容。
