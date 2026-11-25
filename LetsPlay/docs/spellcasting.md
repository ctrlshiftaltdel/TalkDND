# 施法

本页给 DM 与玩家使用：法术数据在哪、战斗内外怎么施法、哪些效果由程序自动结算、哪些必须 DM 手动，以及 2014/2024 两版差异。命令都在游戏文件夹里运行，`T=bin/talkdnd`、`CS=bin/combat-sim`（Windows：`bin\talkdnd.exe`、`bin\combat-sim.exe`）。引擎与数据格式的开发文档在游戏文件夹之外（仓库内的 combat-sim 开发文档），玩家不需要读；本页不重复规则原文，裁定依据是桌上的 SRD。

## 法术数据在哪

- `Rules/combat/2014/spells.yaml` 与 `Rules/combat/2024/spells.yaml` 是手写法术层。每条都带结构化 `spell` 元数据（环级、学派、施法时间、成分、材料、仪式、职业表、范围/持续文本、升环表、戏法成长表）。两版都含 Mending。
- 其余几百个 SRD 法术在 `Rules/combat/<edition>/srd/` 的 pack 里，**只有原文文本，没有结构化元数据**（tracked pack 未重新生成）。要新增结构化法术，改手写层（同 id 会覆盖 pack）。
- 角色的法术列表、准备与战斗块引用由 `talkdnd` 维护（见 `docs/character-data.md`）；`combat-sim` 只负责结算。

## 战斗内施法

```text
$CS cast <谁> <法术> [目标...] [--slot N] [--material "文本"] [--ignore-components]
$CS use  <谁> <法术> [目标...]            # 同参数；reaction/触发类用法
```

- 有环法术默认按基础环结算并扣 `spell-slot-基础环`；`--slot N` 升环扣 `spell-slot-N`（N ≥ 基础环且 ≤ 9）。
- 魔法飞弹给一个目标时，三枚都射向该目标。分配给多个目标时，按飞弹逐一列出三个名字，可以重复同一名字。比如把两枚分给甲、一枚分给乙，就依次填甲、甲、乙。升环新增的飞弹仍由 DM 手动处理。
- 两版护盾术都可在角色被攻击命中或被魔法飞弹选为目标时使用。飞弹触发时，在结算任何一枚之前逐个询问可反应的玩家。同一角色确认一次即可挡下该次施法射向自己的全部飞弹，其他目标照常结算。已有遭遇沿用创建时保存的规则内容，新建遭遇加载更新后的法术数据。
- 有逐级骰表时，升环替换主伤害骰或治疗骰。没有骰表时，DM 用 `--confirm-manual` 确认自己处理额外飞弹、多目标或睡眠池等升环效果，程序按所选环级扣槽并结算基础效果。
- 戏法不能 `--slot`；伤害戏法（Fire Bolt、Shocking Grasp、Sacred Flame）按角色等级 5/11/17 自动换骰。
- 契约施法者（Warlock）的 ref 已被 `spells sync` 覆盖为契约环，`--slot` 会被拒绝；Mystic Arcanum 消耗 `arcanum-N`，同样拒绝 `--slot`。
- 分钟或小时级的法术走下文的长施法流程。

## 材料与成分（声明制）

- 有价格门槛或会消耗的材料：`cast` 必须带非空 `--material "…"`，程序只原样记录（`MATERIAL DECLARED`），不匹配文本、不查物品栏。消耗品另记 `MATERIAL CONSUMED`。
- 例：2024 Bless 的圣徽（5 GP）不声明就施不出来；错误信息会写明缺什么。
- V/S 只做展示，引擎不判断手是否空、是否被沉默；`--ignore-components` 是 DM 显式越权，会记 `COMPONENTS IGNORED`。
- 仪式施法同样适用材料声明：仪式不耗槽不免材料。

## 常见错误与含义

| 错误（节选） | 含义 / 处理 |
|--------------|-------------|
| `--slot N is below the base level` | 升环不能低于基础环 |
| `is a cantrip and cannot be cast with a spell slot` | 戏法不占槽 |
| `is a Mystic Arcanum spell … --slot is not accepted` | 奥术不占法术位 |
| `has no upcast entry for slot level N; refusing to guess` | 数据没有该环骰表；按基础环施放或 DM 手动 |
| `UPCAST NOT MODELED`（日志行） | 已确认手动处理，高环消耗所选法术位，程序结算基础效果 |
| `needs manual settlement` | DM 用 `--confirm-manual` 确认手动处理后重新提交，本次动作和资源未消耗 |
| `needs a material component that …` | 昂贵/消耗材料未声明；给 `--material` 或 DM `--ignore-components` |
| `casting time of N minute(s)/hour(s)` | 使用下文的长施法流程 |
| `UPCAST NOT MODELED` 之外的 `…no main damage or heal effect to scale` | 骰表找不到可替换的主效果（数据问题，报给开发） |

## 战斗外与仪式

- 仪式施法：`$T char <名字> ritual <法术>`。只校验资格并写日志（正常施法时间 + 10 分钟），**不扣槽、不结算效果、不推世界时间**；效果由 DM 叙述或用程序命令另记。
  - 2014：按职业——Bard=已知、Cleric/Druid=已准备、Wizard=法术书（书内未准备也可）。
  - 2024：已准备 + 带 Ritual 标签即可（职业无关）；Wizard 的 Ritual Adept 另允许书内未准备。
  - 仪式不能升环，只能按基础环施放。
- 战斗外的短时普通法术（如 Mage Armor）由 DM 扣除所用法术位并把效果写入会话日志。分钟或小时级的普通施法见下文。

## 由程序结算 / 由 DM 手动

| 类别 | 结构化（自动结算）样本 | 手动（note + manual） |
|------|------------------------|------------------------|
| 攻击骰 + 伤害 | Fire Bolt、Shocking Grasp | — |
| 豁免 + 半伤 | Burning Hands、Fireball、Lightning Bolt、Thunderwave（2024） | — |
| 自动命中伤害 | Magic Missile | — |
| 治疗 | Cure Wounds、Healing Word（2014） | — |
| 条件/增益 | Bless、Shield、Sanctuary、Sleep（2024）、Shocking Grasp 免借机 | Guidance、Mage Armor、Command |
| 强制移动 | Thunderwave 推 10 尺（豁免失败时） | — |
| 其余 | — | Sleep 2014（HP 池）、Detect Magic、Mage Hand、Minor Illusion、Mending、Moonbeam、Spirit Guardians（持续区域） |

普通 `cast`、`use` 或 `ready --ability` 遇到需手动处理的法术时，会要求 `--confirm-manual`。手动确认与成分、动作和资源检查全部通过才结算，日志记录 `MANUAL CONFIRMED`，DM 再处理引擎未实现的效果。准备法术时在准备阶段确认，触发时沿用这次确认。

## 同回合施法限制

- **2014**：同一回合施放附赠动作法术后，其余法术只能是施法时间为 1 动作的戏法。引擎按实际施法顺序双向检查。
- **2024**：每个角色在同一回合最多消耗一个法术位施法。无须消耗法术位的施法仍可使用。轮到下一个生物行动时，这项限制重新计算。战斗中用 `heal --slot` 结算的用槽也计入。
- 普通施法、准备动作中的施法和支持的自动反应共用回合记录。重新打开遭遇时记录仍在。旧遭遇从使用新版后的施法开始记录，升级前已发生的施法仍由 DM 核对。

## 跨探索与战斗的长施法

两版都要求施法者在战斗里每回合用一个动作继续施法，并保持专注。漏掉施法动作、失去专注或主动中止时，这次施法结束，法术资源保留。重试从零开始。

1. 用 `$T time enable` 启用世界时间，再用 `$T time casting start <角色ID> <法术ID>` 登记普通长施法。需要昂贵或消耗材料时声明 `--material`。记录命令返回的施法 ID。
2. 探索时用 `$T time advance --minutes <分钟>` 或 `$T time advance --seconds <秒>` 推进。用 `$T time casting status` 查进度。探索中由 DM 判定专注中断，并用 `$T time casting interrupt <施法ID>` 记录。
3. 先建立遭遇并掷先攻，再用 `$T time casting enter <遭遇存档名>` 把正在进行的施法交给战斗。遭遇文件名在当前存档的 `combat/` 内，可以省掉 `.state.json`。施法者的回合用 `$CS long-cast continue <施法ID>`，消耗当回合的动作并增加六秒进度。遭遇接管期间，世界时间随战斗回合投影到 `time show`。
4. 进度足够时用 `$CS long-cast finish <施法ID> --confirm-manual` 在施法回合完成，或在探索用 `$T time casting finish <施法ID> --confirm-manual`。程序记录完成并扣一次所需资源。DM 随后处理法术效果，例如选择 Mending 修理的物件。
5. 回到探索时用 `$T time casting leave` 结清这一段遭遇时间。未结束的施法沿用原 ID 和进度。在同一遭遇继续施法时重新交接；若队伍资源已经变化，先用当前队伍状态建立遭遇。

此流程覆盖结构化数据中的分钟与小时级普通施法。仪式继续使用前述资格与 DM 流程。成分和探索中的专注仍由 DM 裁定。

## 2014 / 2024 数据差异（同一引擎，两份数据）

| 主题 | 2014 | 2024 |
|------|------|------|
| Cure Wounds | 1d8+调整值，塑能；每高 1 环 +1d8 | 2d8+调整值，防护；每高 1 环 +2d8 |
| Sleep | 90 尺 / 20 尺球，5d8 HP 池，无豁免，手动 | 60 尺 / 5 尺球，感知豁免→失能→再豁免→昏迷（部分自动） |
| Detect Magic 职业表 | 不含 Warlock | 含 Warlock |
| Bless 材料 | 一撮圣水（免费） | 值 5+ GP 的圣徽（需声明） |
| 升环文本 | “At Higher Levels” | “Using a Higher-Level Spell Slot” |
| 仪式资格 | 职业特性（见上） | 版本级规则：已准备 + Ritual 标签 |

## D-12：魔祈授予的仪式（2014 Warlock）

v1.6 不建模魔祈选择：2014 Warlock 的职业级仪式资格是「无」，经 Book of Ancient Secrets / Pact of the Chain 等魔祈获得的仪式施法**不会被程序放行**——`$T char <名字> ritual` 会失败关闭并提示。DM 按玩家自备 SRD 裁定：手动记日志、扣应扣的资源，效果自行结算。

## 已知限制

- tracked SRD pack 内的多数法术仍只有自由文本，`content check` 会给出相应提示。结构化施法从两版手写层加载。
- 升环只覆盖主伤害/治疗骰；多目标、多发魔矢、睡眠池等差异不自动。
- 材料只声明：不查库存、不做手部/器材模型。
- 沉默、手被占用等成分细节由 DM 裁定。
- 旧版二进制读不了带 `spell` 元数据的内容包（失败关闭）；程序与数据同版本发布。
