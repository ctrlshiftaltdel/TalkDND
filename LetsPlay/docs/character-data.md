# 角色卡数据设计（`talkdnd char`）

> 目标：角色数据由程序直接更新，DM 只补程序做不到的部分；开局和每轮更快、更省 token。路径都相对于游戏文件夹（解压出来的那个文件夹，名字随意）。

## 数据在哪里

| 数据 | 唯一来源 | 谁写 |
|------|----------|------|
| 选项（战斗风格、背景、专长、语言、工具、性格四项、阵营等）、物品栏（数量/装备/同调/弹药/魔法加值）、激励、力竭、笔记、`dm_notes`、combat 数据块 | `Saves/<存档>/sheets/<id>.yaml` | `talkdnd chargen sheet`、`talkdnd char` |
| 当前 HP、临时 HP、资源（法术位、生命骰、狂暴等）当前值、状态、死亡豁免 | `Saves/<存档>/combat/party-state.yaml` | `talkdnd char`、combat-sim（战斗和 `party` 命令） |
| 金币（多币种，按铜币记）、经验 | `Saves/<存档>/progress.yaml` | `talkdnd char … gold`、`talkdnd loot`、`talkdnd xp` |
| 可读角色卡 `Characters/<名字>.md` | **生成物**，不是数据源 | 每次 `char` 命令后重新生成 |

combat-sim 仍从角色卡的 `combat yaml` 块读数据，从 `party-state.yaml` 读当前值，所以两边始终一致：战斗里扣的 HP，`char` 能看到；`char` 治疗或长休，下一场战斗也会用到。

## 法术列表（schema 10）

`sheets/<id>.yaml` 的 `spells` 块（存档 `schema: 10` 起）是法术列表的**唯一数据源**；角色卡 Markdown 的 `## 法术` 段只是展示：

| 字段 | 含义 |
|------|------|
| `version` | 块格式版本，当前 1 |
| `mode` | 施法模式：`known`（已知）、`prepared`（准备）、`spellbook`（法术书）；来自职业数据，2014 吟游诗人/游侠/术士/邪术师是 `known`，2024 都是 `prepared`，法师两版都是 `spellbook` |
| `ability` | 施法属性（审计副本；与职业数据不一致时命令会拒绝） |
| `cantrips` / `prepared` / `known` / `book` | 各列表，存法术 id |
| `always_prepared` | 子职常备法术（如 2014 生命领域、2024 邪魔庇主）：不占准备数量、不能被取消准备 |
| `arcanum` | Mystic Arcanum（邪术师 11/13/15/17 级解锁 6/7/8/9 环，每环至多一个，不占法术位） |
| `legacy` | 无法映射到法术 id 的旧自由文本条目，原样保留 |

- **来源与校验**：法术 id 必须属于基础职业法术表（`classes-<版本>.yaml` 的 `spells`）或该职业数据里声明的授予来源（子职常备/扩表、特性授予）——授予语义按各子职自己的 SRD 文本区分，不合并处理。Land 的地形法术只在你用 `spells terrain` 选定地形后进入来源，未选地形时拒绝并提示先选。数量校验沿用建卡的职业表：过多报错，过少警告。
- **always_prepared 播种**：子职的 always_prepared 授予（2014 Life/Devotion/Land、2024 各领域/庇主）在建卡 `set spells` 与 `spells sync` 时按 `from_level`（和地形）自动播种进 `always_prepared` 列表——不占准备数量、不可 `unprepare`；已在任何列表里的法术不重复播种。T2 迁移出的空块由下一次 `spells sync` 补齐。
- **迁移（schema 9→10）**：`talkdnd start` / `talkdnd migrate` 会先整份备份到 `Saves/_backups/<存档>-<时间>-schema9`，再给每张角色卡补 `spells` 块：优先从 `chargen.json` 精确对号，其次从角色卡正文的 `## 法术` 段按「法术 id 或名字的 slug」精确匹配；对不上的进 `legacy`，不猜测。重复迁移不二次改写；DM 手改过的角色卡（校验清单标记 dirty）保持原样不重写，法术列表之后用命令补。恢复备份即可回退。
- 旧的 `spell-slot-*` 法术位资源不改名、不重算当前值，仍由 `combat/party-state.yaml` 管。

### 法术命令（v1.6 T3）

| 命令 | 作用 | 失败时 |
|------|------|--------|
| `char <名字> spells`（加 `--json` 出稳定 JSON） | 只读视图：mode、施法属性、地形（有地形授予的子职）、各列表（按环级+id 排序）、数量 `n/max`、奥术、legacy、法术位（含契约槽与 arcanum） | 未知角色 / 非施法职业 / 缺 spells 块 → 明确错误 |
| `char <名字> prepare <法术>` | 加入准备列表（prepared/spellbook 职业；spellbook 要求已在书里）；校验来源、环级 ≤ 当前可施法环级、数量 | 幂等：已准备报 exit 0 不改文件 |
| `char <名字> unprepare <法术>` | 从准备列表移除；同一次写入里移除该法术的 ref（未准备即不可施放） | `always_prepared`（子职授予）拒绝；未准备是幂等 no-op |
| `char <名字> learn <法术>` | known 职业加入已知列表；邪术师 6–9 环法术自动进 `arcanum`（每环至多一个，未解锁报错） | prepared/spellbook 职业提示用 `prepare`；戏法在建卡时选 |
| `char <名字> scribe <法术> [--hours N] [--cost-gp N]` | 法师抄书：加入 `book`，按 `scribe` 数据扣时间与金币（每环 2 小时 / 50 gp，可用参数覆盖）；不自动准备 | 非 wizard / 已在书 / 金币不足 → 错误且不写任何文件 |
| `char <名字> ritual <法术>` | 仪式施法校验 + 日志（正常施法时间 + 10 分钟，不消耗法术位，不推进世界时间，效果由 DM 叙述） | 无 Ritual 标签 / 不在资格列表 / 内容数据不可用 / 2014 魔祈授予（D-12）→ 失败关闭 |
| `char <名字> spells adopt [--map 原名=id]` | 把 legacy 精确采纳进列表（id 或 slug 精确匹配，无模糊）；缺 spells 块时从正文 `## 法术` 建块；不改正文 | 目标 id 不在职业来源内 / legacy 里没有该名字 → 错误 |
| `char <名字> spells sync [--dry-run]` | 按"当前模式可施法集合"重建 combat 块的法术 refs（cantrips + known/prepared + always_prepared + arcanum）；同时播种子职 always_prepared（dry-run 只报告不写入） | 见下面的受管集合规则 |
| `char <名字> spells terrain <地形>` | 选定 Land 子职的地形（2014：arctic/coast/desert/forest/grassland/mountain/swamp；2024：arid/polar/temperate/tropical），同一原子写入里播种已解锁的该地形授予 | 职业/子职没有地形授予 / 地形不合法 → 错误并列出可选值 |

**refs 与列表的关系**：combat-sim 的 `cast` 只认角色卡 combat 块 `actions` 里的法术 ref。 `spells sync` 新增缺失的 ref、刷新成本覆盖，并去掉同一目标的重复 ref；目标不在列表并集里的 ref（玩家手写的、历史遗留的）**保留不动**并在输出里逐条标注"未受管"。契约施法者（邪术师）的环级 ≥1 法术 ref 一律覆盖成本为 `spell-slot-L`（L=契约槽环级），`arcanum` 法术覆盖为 `arcanum-N`；戏法 ref 不带成本覆盖。内容注册表可用时还会校验新增 ref 的目标是否存在（缺数据给 warning，不阻断）。**不 prepare 就 sync，该法术没有 ref，`cast` 报 "has no ability matching"**；unprepare 则在同一原子写入里移除 ref。所有写操作走 `sheets/<id>.yaml` 原子写：任何校验失败都不改文件。

**数量与来源**：来源校验按职业表 + 子职授予 + Magical Secrets（`classes-<版本>.yaml`）失败关闭；D-8——工具不强制"刚长休才能准备"，只在成功时提示。升级后数量上限变化由 `level-up` 提示新增选择，已有法术与 ref 不会被静默删除。

## 角色卡怎么生成

`Characters/<名字>.md` = 上半部分（身份、属性、技能、特性等可读内容，来自建卡或迁移前的原卡）+ 程序生成部分（`<!-- talkdnd:char … -->` 到 `<!-- /talkdnd:char -->`：角色选项、当前状态、物品栏、笔记、DM 备注、combat 数据块）+ 尾部（升级记录）。

- `talkdnd level-up` 直接改数据块并追加升级记录；下次 `char` 命令把数据块和尾部读回 yaml，所以不会丢。
- 手改角色卡会被下一次 `char` 覆盖（除了数据块和尾部），存档校验也会报警告。**DM 不手改角色卡。**
- **物品来源**：`item add` 只接受规则数据里的武器、护甲、盾牌和常见装备；书里特有或原创物品要用 `--canon`（查过书）或 `--original O-xxx`（已登记的原创物品），说明见 `docs/deviation.md`。原创标记只留在 `sheets/*.yaml`，不会写进给玩家看的角色卡。

## 自动计算

### 只读成长预览

用 `talkdnd advancement identity-preview --level-up <角色 ID 或完整名字>` 查看该玩家角色的下一级。加 `--json` 得到相同信息的结构化输出。省略 `--level-up` 时，命令继续显示原来的身份诊断。

预览支持 schema 9 和 10，按当前战役和已解锁的同版规则来源显示升级机会。5e 把可选专长与职业属性值提升分开提示。5.5e 会显示专长类别和等级安排。候选仍需核对前置条件与过往选择。属性值假设按每项单独计算。结构化记录与 Markdown 数值冲突时，预览标出冲突属性并省略该项预测。玩家和 DM 仍需确认组合、HP 与其余衍生效果。

预览每次读取当前资料。结果显示 `consistency: unverified` 和 `write_capability: unverified`，绑定不可用于提交。未登记或不明的遭遇只显示状态提示，身份未核对时也可查看有可信来源的升级资料。缺少可信职业或等级时会说明原因并停止。

### 只读选择校验

玩家显式给出本次机会和唯一候选后，同一条命令用三态内核出具可解释的资格结果：

```sh
# 专长候选（目录里的精确 ID）
talkdnd advancement identity-preview --level-up <角色> \
  --select-opportunity ability_score_improvement_or_optional_feat \
  --select-feat <专长 ID>

# 属性提升候选（恰好 +2：一项 +2 或两项各 +1）
talkdnd advancement identity-preview --level-up <角色> \
  --select-opportunity ability_score_improvement_feat_or_other_qualified_feat \
  --select-abilities str+1,dex+1

# 未知条件由 DM 单独确认（依据非空；确认不能替代玩家选择）
… --dm-confirm "DM 核对：该角色此前未取过专长"

# 未识别专长声明来源（授权目录的包 ID）
… --select-feat <专长 ID> --select-source srd-5.1-feats
```

规则与边界：

- 每个资格条件逐条给出 `met` / `not_met` / `unknown`，并附来源与版别（2014/2024）。机会 ID 按精确字符串比对；专长 ID 只接受目录的精确小写 ID，别名、显示名与越界数值一律拒绝。
- 来源按规则集加载边界隔离：只打开本版已授权目录（SRD 目录；FULL 规则集另加同版已解锁内容包）。目录里已识别的专长若与显式 `--select-source` 声明冲突，一律拒绝；未识别专长的授权只能由落在授权范围内的声明给出。
- 已知非法优先拒绝：声明了授权范围外的来源（错版目录、任何 FULL 来源之于 SRD 规则集、未解锁的同版 FULL 包）、前提等级、属性 20 上限、总量非 +2、机会不授予该候选形式。已知非法与未知并存时同样拒绝。
- 未识别专长缺少来源声明、或声明的来源无法在本机核对时，授权保持未知并**直接拒绝**——DM 确认不能补足授权，也不能让手动标记绕过未独立通过的条件。
- 重复按规则数据中的 `repeatable` 证据分三种：明示可重复（已记录的此前选取不非法）、明示可重复但附条件（条件无法从角色卡核对，保持未知）、数据没有重复性表述（保持未知，不据此判已知非法）。后两种需 DM 非空确认才能继续；是否把"缺省"当作一次限定，待规则数据或所有者定案。
- 其余未知条件（历史选择、子职特性、目录外内容等）只有在 DM 用 `--dm-confirm` 单独给出非空依据时才继续，结果标 `not_program_verified: true`，不会转为已验证。
- 目录里找不到的专长只有在机会、声明授权与重复三项条件各自通过后才标 `unverified_effects_manual`（未验证、效果手动）；三项条件未全过时标记不给。
- 校验全程只读：成功、输入错误与拒绝都不写任何文件；属性提升与专长效果都不由本入口应用，派生数值、提交与回退由后续任务接线。旧档（仅 Markdown 角色卡）的历史选择保持未知，由 DM 确认后继续。
- 遭遇状态、身份绑定与一致性边界沿用预览的输出；提交前的重新校验由成长提交任务负责。

- **AC**：装备中的护甲（基础值 + 敏捷，按上限）+ 盾牌 +2 + 物品 `ac_bonus` + 防御风格 +1（穿护甲时）；不穿护甲时用 `ac_base`（无甲公式或迁移时的原 AC）。
- **攻击**：每件装备中的武器生成一条攻击（熟练按职业武器熟练、灵巧取高、远程用敏捷、魔法加值），可双手武器另生成 `<id>-2h`。非武器攻击（徒手、法术、天生攻击）原样保留。
- **战斗风格**：箭术 +2 命中（远程）；对决 +2 伤害（单手近战）；巨武器战斗在双手近战攻击上写 `reroll_damage_below: 2`（2014）或 `damage_die_min: 3`（2024），combat-sim 掷伤害时自动执行。
- 短休恢复短休资源，并可花生命骰恢复 HP，掷骰记入日志。长休回满 HP 和资源，清除临时 HP 和死亡豁免。2014 恢复生命骰总数的一半（向下取整，至少 1），并在 DM 用 `--food-and-water` 确认进食和饮水后降低 1 级力竭。2024 恢复全部已花费的生命骰，并降低 1 级力竭。
- **伤害**：先扣临时 HP；0 HP 时再受伤记一次死亡豁免失败；一次伤害溢出 ≥ 最大 HP 即死亡。

## 日志与存档校验

每条 `char` 命令（batch 也是一条）在最新会话日志写一行 `角色 <名字>：…`，金币走 `progress.yaml` 记账（`战利品/花费…` 行）。命令写完文件后更新存档校验清单（`integrity.yaml` 也跟踪 `sheets/*.yaml`），所以 `talkdnd check` 的对账能通过。batch 是原子的：任何一项出错，什么都不写。

## 迁移旧存档

旧角色卡第一次用 `char` 时自动迁移：数据块成为 yaml 的 combat 部分，其余正文原样保留，已有武器攻击变成「已装备」物品。迁移不改数值。

护甲与 AC：原卡正文里写到的护甲和盾牌（不算熟练项、法术行）会变成「已装备」物品，前提是它们能算出原卡的 AC（正好相等，或多 1 且原卡写了防御战斗风格，这时风格也一并记上）；之后 AC 由物品重算，`ac_base` 只是无甲 10 + 敏捷。算不出来（没写护甲、写了未知护甲、数值对不上）就保留原 AC 作为 `ac_base`，并记 `ac_base_has_style: true`：原 AC 被当作已含战斗风格，之后 `set fighting-style defense` 不会再 +1。要让 AC 改由物品计算，就 `item add` 并 `equip` 实际护甲。之后用 `char … set` 补缺的栏目（战斗风格、背景装备与金币等）。

## 程序做不到的

写进 `dm_notes`（`char <名字> dm-note "…"`）：剧情特殊效果、临时裁定、书里有但数据没有的特性。不要把能用命令表达的东西写进备注。

## 命令速查

见 `AGENTS.md`「角色卡数据」，或 `talkdnd char <名字> --help`。
