# 目录结构与交接清单

> 本文件是 `AGENTS.md` 的参考章节，按需查阅。路径都相对于游戏文件夹（解压出来的那个文件夹，名字随意）。

## 项目结构

| 路径 | 内容 | 是否入库 |
|------|------|----------|
| `Rules/2014/<slug>/` | 玩家自备的 5e（2014）规则书，一本一个文件夹：PDF + `index/`（pdf2md 生成）。至少 SRD 5.1；可加 PHB/DMG/MM 2014 | ❌ 仅 README 入库 |
| `Rules/2024/<slug>/` | 玩家自备的 5.5e（2024）规则书，同上。至少 SRD 5.2.1；可加 PHB/DMG 2024、MM 2025 | ❌ 仅 README 入库 |
| `Rules/packs/` | 已构建的完整书内容包 `full-<edition>-<book>.pack.json`（由本机书籍解锁）；只放本机文件，永不入库 | ❌ |
| `Rules/rulings.md` | 两版通用的 DM 裁定（本机副本，**牌桌上以它为准**；没改过的副本随出厂默认自动升级，改过的不覆盖） | ❌（本机文件，不入库） |
| `Rules/rulings-2014.md` / `Rules/rulings-2024.md` | 版本差异裁定（本机副本；突袭、精通、睡眠术、激励、躲藏、力竭、擒抱、状态细节…） | ❌（本机文件，不入库） |
| `Rules/defaults/` | 随版本发布的裁定出厂默认（`rulings*.md`），每次更新覆盖；DM 不读、不改它，只读 `Rules/rulings*.md`。见下文「裁定文件」 | ✅ |
| `Rules/combat/` | combat-sim 规则数据，分层：`common/` + `2014/` + `2024/`（只收 SRD 内容） | ✅（SRD，CC-BY-4.0） |
| `Adventures/<slug>/` | 玩家自备的冒险书，一本一个文件夹：PDF + `index/`（pdf2md 生成） | ❌ 仅 README 入库 |
| `Adventures/<slug>/combat/` | 该冒险的战斗数据：`scenes/`、`encounters.yaml`、`creatures.yaml` | ❌（由书改编，本地自建） |
| `Saves/YYYY-MM-DD-NN/` | 一份存档：`Characters/`（全部队伍成员：玩家角色和 DM 操控的同伴，`sheets/<id>.yaml` 的 `controller: player|dm` 区分；`_party.md` 是同伴总览）、`NPCs/`（只放剧情 NPC，可为空）、`Sessions/`（会话日志）、`combat/`（`campaign.yaml` 清单 + `*.state.json` 战斗状态）、`canon.md`（即兴定论）、`progress.yaml`（经验/金币/物品/当前场景）、`reveals.yaml`、`handouts.yaml` 与 `handouts/`（已给玩家看的资料图）（已揭示知识，`talkdnd reveal`）、`memory.yaml`（固定记忆，`talkdnd memory` 维护；玩家视图只显示玩家确认条目）、`world-time.yaml`（游戏内时间与预定事件，`talkdnd time` 维护；默认关闭，启用后才有）、`integrity.yaml`（存档校验，程序维护，不要手改）、`save-meta.yaml`（存档结构版本、锁定的偏离度、桌面设置（骰子显示、DM 风格）与自动化开关（`settings.automation`：存档默认 + 场景/遭遇覆盖），程序维护）、`original.yaml`（DM 原创内容台账，含隐藏面）、`dm-plan.md`（DM 长期计划，可无）、`secret-terms.yaml`（手动秘密词）、`identified.yaml`（已识别怪物/物品）、`rolls.log`（全部掷骰记录）、`house-rules.md`（可选）、`dm-secrets.md`（🔒）、`digest.md`、`chargen.json`、`sheets/`、`npc-drafts/`、`checkpoints/<id>/`（结构化检查点：`manifest.yaml` 记每个副本的 sha256，数据副本覆盖 `progress/reveals/save-meta/original/identified/canon/house-rules` 与 `Characters/NPCs/sheets/combat`；DM 本地恢复数据，不是玩家视图；每次快照后只保留最新 10 份） | ❌（个人进度） |
| `Saves/_backups/` | 存档结构迁移前、以及 `checkpoint restore` 恢复前的整份备份（自动生成，可删） | ❌ |
| `docs/` | DM 参考章节（本文件、掷骰、叙事、战斗、pdf2md、建卡细节、剧本偏离度、世界时间），按需读 | ✅ |
| `tools/pdf2md/` | PDF → Markdown 后台转换工具（Python，经 uv 运行）。输出在书文件夹的 `index/`，见 `docs/pdf2md.md` | ✅ 源码；❌ 输出与 `.venv` |
| `bin/combat-sim`、`bin/talkdnd` | 战斗程序与游玩助手。Windows 加 `.exe`。不入库。放法见 `README.md` 和 `bin/README.md` | ❌ 本地文件 |

自动化开关存在 `save-meta.yaml` 的 `settings.automation`：每个存档一组默认，场景与遭遇按 id 覆盖，生效顺序是 **遭遇 > 场景 > 存档默认**。DM 用 `talkdnd settings automation` 查看/修改，每个值的来源和每次改动都会写明；详见 `docs/automation.md`。

角色卡的结构化数据 `sheets/<id>.yaml` 在 `save-meta.yaml` 的 `schema: 10` 起多一个 `spells` 块：职业施法模式（`mode: known|prepared|spellbook`）、施法属性与 `cantrips` / `prepared` / `known` / `book` / `always_prepared` / `arcanum` / `legacy` 列表（全部存法术 id）。它是法术列表的唯一数据源，角色卡 Markdown 的 `## 法术` 段只是展示。旧存档迁移（schema 9→10）先整份备份到 `Saves/_backups/`，再从 `chargen.json` 或正文精确对号入座，对不上的进 `legacy`；手改过的角色卡不会被重写。字段与校验规则见 `docs/character-data.md`「法术列表」。

`schema: 11` 起每名队伍角色带严格 v1 成长身份：有结构化卡的载体在 `sheets/<id>.yaml` 末尾多一个顶层 `talkdnd_identity` 字段，仅有 Markdown 的老卡在卡末尾多一行 `<!-- talkdnd-identity: … -->` 注释；两者都与 `advancement/<id>.yaml` 成长记录（`version: 1`、`history: unknown`、同一 128 位非秘密标识）成对。10→11 迁移不重写 DM 手改：先 `talkdnd migrate --dry-run` 只读核对每张未绑定卡的前像摘要，再带 `--confirm-binding PATH=DIGEST`（可重复）重跑完成入籍；整个步骤走成长协议 D 流程（写入前复查准入——首批为已验证的本地 Linux ext4，整份备份含 ACL/xattr 元数据，意图记录与显式恢复）。未确认、未准入或失败时存档停在 schema 10。首次迁移创建协议控制文件是唯一例外，且必须在完整确认、准入与复制前置条件（存档与备份同设备、遭遇间隙）通过之后；确认摘要绑定 DM 核对时的前像，核对后修改过的载体（含结构化卡）会被拒绝并要求重新核对。新开存档也从 schema 10 起：建角色后按同一确认流程入籍到 11，避免 11 档出现无绑定角色。`advancement/<id>.yaml` 已存在（包括空文件）时拒绝自动入籍，走人工核对或显式恢复；旧历史一律 `unknown`，同 ID 重绑不在本批。契约与交付边界见 `docs/ADVANCEMENT-IDENTITY-CONTRACT.md` §6。

采用整秒计时的存档将长施法进度写入 `world-time.yaml` v2。`combat/party-state.yaml` v2 将扣费凭据与资源余额保存在同一文件。使用精确计时的检查点也保存世界时间。见 [战役连续性](campaign-continuity.md)。

登记后的 `combat/*.state.json` 保存遭遇 ID 和 DM 的结束原因，使用 v5–v8。旧遭遇的 v1–v4 仍可读取。待确认反应或未结清的计时交接会阻止结束。见 [战斗与遭遇](combat.md)。

## 成长协议过渡期（存档根目录的控制文件）

存档根目录出现 `.talkdnd.advancement.lock` 或 `.talkdnd.advancement.state` 时，说明这份存档已启用**成长协议**（角色成长提交使用的协作锁）。

- 程序**不会**自己创建这两个文件；没有它们的存档与以前完全一样，也不会自动初始化。唯一例外是经所有者批准的首次 schema 10→11 迁移：只有在完整 DM 确认（`--confirm-binding` 摘要与当前载体一致）、#344 准入与复制前置条件（存档与备份同设备、遭遇间隙）都通过后，`talkdnd migrate --confirm-binding` 才允许创建这两个控制文件；未确认、确认失效、被拒绝或未准入时不写任何控制或目标文件，存档停在 schema 10。没有通用、无约束的 `init` 命令。
- 有这两个文件时，已接入的 `talkdnd` 与 `combat-sim` 写命令（例如 `talkdnd xp add`、`talkdnd char`、`talkdnd start`、`talkdnd level-up`，以及 `combat-sim init|attack|…`、`combat-sim party rest|set-hp|sync`、`combat-sim roll`）会在写目标前取得 SAVE 锁。SAVE state 缺失、不可写、`active` 或 `in_doubt` 时以退出码 1 拒绝且不写任何目标文件（接入期对缺失 state 的保守处理）。`combat-sim content import-*` 与 `content pack build|release` 只在 `--out` 指向已启用协议的存档内时取 SAVE 锁；`--out` 在存档外时按存档外写入允许；`pack build|release --dry-run` 只读。不要删除或改写这两个文件。
- 尚未接入的写命令（例如 `talkdnd log`、`talkdnd checkpoint "<内容>"`、`talkdnd check`、`talkdnd handout build`）仍会**以退出码 1 拒绝执行**，并说明“已启用成长协议、该入口尚未接入、未写入任何目标文件”。不要当成临时错误反复重试。`talkdnd migrate` 只在带完整 `--confirm-binding` 的 schema 10→11 已确认路径上接入 D 协议（由 D 自己取 SAVE），其余普通迁移仍按未接入拒绝；确认路径不先跑普通迁移的 checkpoint 锁或 ruleset 改写。
- 只读命令（`known`、`xp show`、`log rolls`、`checkpoint list`、`char <who> spells` 普通视图、`combat-sim status`/`scene`/`log`/`simulate`、`combat-sim party show` 等）照常可用，不取 SAVE 锁；`guard` 与 `secret list` 的缓存仍会写入，它属于控制元数据，不算用户数据，也不取 SAVE 锁。
- `start` 的用户数据与迁移写入取 SAVE 锁；它的 `digest.md`/缓存仍是控制元数据，不单独取 SAVE 锁。锁不可用时整条命令拒绝，也不写 digest/缓存。
- `talkdnd new` 仍可创建新存档；新存档默认不启用协议，也不会去写已启用协议的存档。
- 具体命令的 `--help` 会重复这条说明。

## 规则集（`ruleset`）

每个存档在 `combat/campaign.yaml` 里只选一个规则集；开档后不要换 edition。

| 玩家选项 | 存档值 | edition / 目录 | 加载内容 |
|----------|--------|----------------|----------|
| 5.5e FULL | `5.5e-full` | 2024 | SRD 层 → 已解锁 `full-2024-*` pack → 手写/冒险/战役覆盖 |
| 5e FULL | `5e-full` | 2014 | SRD 层 → 已解锁 `full-2014-*` pack → 手写/冒险/战役覆盖 |
| 5.5e SRD | `5.5e-srd` | 2024 | `Rules/combat/common/` + `Rules/combat/2024/`；不加载完整书 pack |
| 5e SRD | `5e-srd` | 2014 | `Rules/combat/common/` + `Rules/combat/2014/`；不加载完整书 pack |

- **FULL 失败关闭**：至少需要一本同 edition 的已解锁完整书。缺书、未解锁或内容包损坏时，`talkdnd start` 与 `combat-sim content check` 会报错并指出缺哪本书、PDF 放哪里、pack 放哪里；不会静默退回 SRD。
- **分层顺序**：FULL 的 SRD 层是 `Rules/combat/common/` + `Rules/combat/<edition>/srd/` + `<edition>/**/srd-*` 覆盖（如 `2024/monsters/srd-spatial.yaml`）；之后加载已解锁 full pack；`<edition>/` 下其余手写文件（`monsters/`、`spells.yaml` 等）与冒险/战役覆盖最后加载，因此手写内容仍可覆盖 full pack。
- **一个存档只用一版**：FULL/SRD 只影响是否叠加完整书，不能把 5e 与 5.5e 混在同一存档；另一版的 `Rules/combat/<edition>/` 目录不会被读取。
- **旧值兼容**：旧存档里的 `ruleset: "2014"` / `"2024"` 仍可读取，分别按 `5e-srd` / `5.5e-srd` 处理；下一次写 manifest（`start`、`migrate` 或 `chargen` 写字段）会把值改成新值并留下一行迁移注释，其他字段和你的注释保持不变。

## 书库：一本书一个文件夹

`Adventures/` 和 `Rules/2014/`、`Rules/2024/` 用同一个标准：每本书（一个 PDF）一个文件夹，文件夹名是书名的 slug。

```
Adventures/
  keys-from-the-golden-vault/
    Keys from the Golden Vault.pdf      # 原始 PDF（权威来源）
    index/                              # pdf2md 输出（自动生成，不入库）
      Keys from the Golden Vault.pages.md   # 唯一的正文：<a id="pN"></a> = PDF 第 N 页
      index.md  index.json  manifest.json  headings.json  img/  pages/ …
      handouts.yaml                     # 资料图登记表（handout 命令维护）
    combat/                             # 战斗数据（AI DM 按书自建）
Rules/
  2024/
    srd-cc-v5-2-1/
      SRD_CC_v5.2.1.pdf
      index/
  chargen/  combat/  defaults/  rulings*.md   # 程序数据和裁定，不是书，留在原处
```

**slug 规则**（程序和 pdf2md 用同一套，结果确定）：取 PDF 文件名（不含 `.pdf`），ASCII 字母和数字转小写保留；撇号 `'`、`’` 直接去掉；其余每一段连续字符变成一个 `-`；去掉首尾的 `-`。例：`Storm King's Thunder` → `storm-kings-thunder`，`SRD_CC_v5.2.1` → `srd-cc-v5-2-1`，`Café — Book!` → `caf-book`。书名里一个 ASCII 字母或数字都没有时，用 `book-` 加 8 位十六进制（FNV-1a 32 位哈希）。

**自动整理**：`bin/talkdnd start` 每次都会先整理书库；也可以手动 `bin/talkdnd library tidy`（加 `--dry-run` 只看不动）。

- 直接放在 `Adventures/`、`Rules/<版本>/` 下的 PDF，或放在名字不对的文件夹里的 PDF，连同旧版转换结果（`.md/<书名>/`）一起移进 `<slug>/`、`<slug>/index/`。每一处移动都会报告，并记在 `.talkdnd/library-moves.yaml`；存档里引用旧路径的地方随之改写。
- 已经在对的文件夹里的书不动；整理可以重复运行，第二次不会再有改动。
- 不知道版本的规则书（直接放在 `Rules/` 下或 `Rules/` 的其他子目录）只提示，不移动。
- 从不删除书文件。目标位置已有同名文件时两份都保留，并提示你手动处理。唯一会删的是旧版多余的「无页码阅读版」`index/<书名>.md`：只有同目录的页码版 `<书名>.pages.md` 存在且带页锚点时才删，并报告节省的字节。
- 手工放的 `.md` 文本不是标准格式，程序不读它们，整理时会提示；需要的话请放 PDF。
- pdf2md 正在后台转换时不整理，等它结束后再运行。
- 读取仍然兼容旧结构（`.md/<书名>/`），所以没整理过的书照样能查。

**整本与分卷**：一本书拆成多个 PDF 时，文件夹名是 `<slug>-a`、`<slug>-b`、`<slug>-2` 等，读取时按同一本书合并。拿到整合版 PDF 后，把它放进 `<slug>/`，运行 `bin/talkdnd library merge <slug>`：旧分卷整体移到 `.talkdnd/book-backups/merge-<slug>/`（不删除），分卷的战斗数据在整本没有 `combat/` 时自动移入，存档里的旧路径重指向整本。`library tidy` 发现整本与分卷并存时会提示这条命令，不会自己搬。

## 裁定文件（`Rules/rulings*.md` 与 `Rules/defaults/`）

两份同名文件不是重复，分工固定：

- `Rules/defaults/rulings*.md`：出厂默认，随版本发布，每次更新都会被覆盖。**DM 不读、不改。**
- `Rules/rulings*.md`：本机副本，**牌桌上只以它为准**，新裁定写在这里。
- `bin/talkdnd start` 同步：副本缺失时从默认复制；副本没被改过（与上次复制的默认或任何历史出厂版本一致）时自动换成新默认；副本改过则保留，若默认也变了，会提示你对照 `Rules/defaults/` 手动合并。

角色卡是 Markdown：上半部分写给人看，末尾有一个 ```` ```combat yaml ```` 数据块给引擎读。两部分都由 `talkdnd char` 从 `sheets/<id>.yaml` 生成，**不要手改**（见 `docs/character-data.md`）。

## 交接清单（换 DM 之前 / 每次结束会话时）

- [ ] 会话日志末尾写好 `## 存档点（现实日期 时间）`：游戏内时间、地点、每个角色 HP/资源/金币/重要物品、未决事项、下一步可能的选项。
- [ ] 需要能回滚时打了结构化检查点：`talkdnd checkpoint snapshot [--reason 原因]`（`checkpoint list` / `show <id>` 看状态，`snapshot diff <id> current` 看变化，`checkpoint restore <id> --yes` 恢复——恢复前会自动整份备份）。
- [ ] 本场的 HP、资源、物品、金币变化都用 `talkdnd char` 记过（`talkdnd char <名字>` 看一行状态）；`content check` 0 error；`talkdnd check`（不带参数）无警告。
- [ ] 本次得知的新事实已 `talkdnd reveal add`。
- [ ] 新裁定已写进本机副本 `Rules/rulings.md` 或 `Rules/rulings-<版本>.md`（`Rules/defaults/` 是出厂默认，不要改；升级后本机副本不会被覆盖）。
- [ ] 新秘密/伏笔写进 `dm-secrets.md`（长期计划写 `dm-plan.md`），日志里没有泄露秘密。
- [ ] 本次即兴说出的公开设定已写进 `canon.md` / `NPCs/`（剧情 NPC）。
- [ ] 进行中的战斗：state 文件名写进日志，并注明当前轮次与回合者。
- [ ] 不要把版权文本、存档、`dm-secrets.md` 提交到 git（`.gitignore` 已排除，提交前 `git status` 检查）。
