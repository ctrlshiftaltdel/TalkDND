# 更新日志

本文件记录 TalkDND 每个发布版本的变化，按版本从新到旧排列。版本号规则见 [docs/versioning.md](docs/versioning.md)。GitHub Release 的说明文字由本文件对应的版本小节生成（`cargo run --manifest-path tools/talkdnd-dev/Cargo.toml -- release-notes <版本>`）。

每个版本分为：**新功能**、**变更**、**修复**、**更新说明 / 迁移**、**已知限制**。括号里的 `#N` 是合并的 PR。

## v0.6.0 — 2026-10-22

### 新功能
- **覆盖式更新**：把新版发布包直接解压覆盖到旧的 `LetsPlay/` 文件夹即可更新。发布包里只有程序文件和说明，不含任何会覆盖用户数据的文件（存档、规则书、本地导入、`fixes/`、资料图登记表都不在包里，打包时自动检查）。
- **存档结构版本与自动迁移**：每个存档有 `Saves/<存档>/save-meta.yaml`（结构版本、上次使用的程序版本）。`talkdnd start` 发现存档结构比程序旧时，先把整个存档备份到 `Saves/_backups/<存档>-<时间>-schema<N>/`，再迁移到当前结构（旧角色卡转为结构化数据等），写会话日志，并更新完整性清单，迁移本身不会被当成手改。
- **本地导入数据版本**：`Rules/chargen/local/imported-<规则>.yaml` 记录导入器版本；版本过旧或书有更新时，`start` 自动重新导入。
- **旧版遗留文件清理**：发布包带 `LetsPlay/.talkdnd-files.txt`（随包文件清单和校验值）。`start` 对比上一个版本的清单，把新版已删除、且用户没改过的旧程序文件移到 `.talkdnd/obsolete/`，不直接删除。
- `CHANGELOG.md`（本文件）和 `docs/versioning.md`；GitHub Release 说明由更新日志生成。

### 变更
- 项目自查（代码审阅由 opencode 完成，见 PR）：修正若干规则与错误处理问题、合并重复代码、缩短 `talkdnd start` 用时，详见 PR 说明。
- README 新增「更新到新版本」步骤。

### 更新说明 / 迁移
- 先存档并退出正在运行的 talkdnd / combat-sim（Windows 上运行中的 `.exe` 不能被覆盖），再把新包解压到 `LetsPlay/` 的上一级目录并选择覆盖。
- 第一次 `talkdnd start` 会自动备份并迁移存档、按需重新导入本地书籍数据；输出里会说明做了什么。备份在 `Saves/_backups/`，确认没问题后可以自行删除。

### 已知限制
- 旧版遗留文件只能在 v0.6.0 之后的更新中识别（v0.6.0 之前的发布包没有文件清单）。
- 自动迁移只覆盖已知的旧结构；手改过的存档迁移后仍会被完整性检查标出。

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
