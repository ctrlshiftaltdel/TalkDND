# 施法（v1.6 T5）

本页给 DM 与玩家使用：法术数据在哪、战斗内外怎么施法、哪些效果由程序自动结算、哪些必须 DM 手动，以及 2014/2024 两版差异。命令都在游戏文件夹里运行，`T=bin/talkdnd`、`CS=bin/combat-sim`（Windows：`bin\talkdnd.exe`、`bin\combat-sim.exe`）。引擎口径见 `tools/combat-sim/docs/COMBAT-SPELLS.md`；本页不重复规则原文，裁定依据是桌上的 SRD。

## 法术数据在哪

- `Rules/combat/2014/spells.yaml` 与 `Rules/combat/2024/spells.yaml` 是手写法术层，两版各 17/18 个常用法术，每个都带结构化 `spell` 元数据（环级、学派、施法时间、成分、材料、仪式、职业表、范围/持续文本、升环表、戏法成长表）。数值按 SRD 文本抄录。
- 其余几百个 SRD 法术在 `Rules/combat/<edition>/srd/` 的 pack 里，**只有原文文本，没有结构化元数据**（tracked pack 未重新生成）。要新增结构化法术，改手写层（同 id 会覆盖 pack）。
- 角色的法术列表、准备与战斗块引用由 `talkdnd` 维护（见 `docs/character-data.md`）；`combat-sim` 只负责结算。

## 战斗内施法

```text
$CS cast <谁> <法术> [目标...] [--slot N] [--material "文本"] [--ignore-components]
$CS use  <谁> <法术> [目标...]            # 同参数；reaction/触发类用法
```

- 有环法术默认按基础环结算并扣 `spell-slot-基础环`；`--slot N` 升环扣 `spell-slot-N`（N ≥ 基础环且 ≤ 9）。
- 升环只替换**主伤害/治疗骰**（按数据里的逐级骰表）；没有骰表的升环效果（多一发魔矢、多一个目标、睡眠池 +2d8）不会自动发生，DM 按文本手动补。
- 戏法不能 `--slot`；伤害戏法（Fire Bolt、Shocking Grasp、Sacred Flame）按角色等级 5/11/17 自动换骰。
- 契约施法者（Warlock）的 ref 已被 `spells sync` 覆盖为契约环，`--slot` 会被拒绝；Mystic Arcanum 消耗 `arcanum-N`，同样拒绝 `--slot`。
- 施法时间带分/小时的法术（如 Mending 的 1 分钟）**在战斗内直接拒绝**：跨回合长施法由 DM 手动跑（见下）。

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
| `UPCAST NOT MODELED`（日志行，非失败） | 该法术未建模升环，高环只产生基础效果，DM 手动补 |
| `needs a material component that …` | 昂贵/消耗材料未声明；给 `--material` 或 DM `--ignore-components` |
| `casting time of N minute(s)/hour(s) … resolve it outside combat` | 战斗内无法完成；走战斗外流程 |
| `UPCAST NOT MODELED` 之外的 `…no main damage or heal effect to scale` | 骰表找不到可替换的主效果（数据问题，报给开发） |

## 战斗外与仪式

- 仪式施法：`$T char <名字> ritual <法术>`。只校验资格并写日志（正常施法时间 + 10 分钟），**不扣槽、不结算效果、不推世界时间**；效果由 DM 叙述或用程序命令另记。
  - 2014：按职业——Bard=已知、Cleric/Druid=已准备、Wizard=法术书（书内未准备也可）。
  - 2024：已准备 + 带 Ritual 标签即可（职业无关）；Wizard 的 Ritual Adept 另允许书内未准备。
  - 仪式不能升环，只能按基础环施放。
- 非仪式的战斗外施法（Mending、Mage Armor、长施法等）也由 DM 手动：扣槽用 `$T char <名字> use slotN`，效果写日志。

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

手动类别的法术仍然可以在战斗里施放（扣槽、成分检查照常），只是效果要 DM 自己落地。

## 同回合限制与长施法（引擎不强制）

- **2014**：本回合用附赠动作施过法后，不能再施法，唯一例外是施法时间为 1 动作的戏法。
- **2024**：每回合只能用**一个法术位**施放一个法术；戏法不受此限。
- **长施法**（施法时间分钟/小时级）：施法者之后每回合用一个动作继续，并全程专注；中断或放弃则施法失败且**不消耗法术位**，重来须从头。程序不会替你记账——战斗内这些法术直接被拒绝，DM 手动跑并记日志。

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

- tracked SRD pack 未重新生成：pack 内法术无结构化元数据（`content check` 对它们给 legacy warning，不影响使用）；手写层 17/18 个法术才可 `cast` 自动结算。
- 升环只覆盖主伤害/治疗骰；多目标、多发魔矢、睡眠池等差异不自动。
- 材料只声明：不查库存、不做手部/器材模型。
- 引擎不因失能/昏迷自动断专注；沉默、手被占用等成分细节由 DM 裁定。
- 旧版二进制读不了带 `spell` 元数据的内容包（失败关闭）；程序与数据同版本发布。
