# 施法核心设计（v1.6）

> **状态：设计稿，未实现。** 本文只定义目标、边界、数据模型、命令面与实现拆分，不代表任何能力已经交付。
> 任务：TASK-0120 / Issue #236。
> 本文的改动范围只有 `docs/SPELLCASTING.md` 与 `docs/PLAN.md` 的设计目标；不修改代码、测试、规则数据、存档 schema、Cargo/CI 或发布文件。
> 带「待所有者决定」的条目是产品决定，本设计只给选项与推荐，不替项目所有者拍板。

## 0. 摘要

- **现状**：法术已经是 `Ability`，但只有“名称 + 标签 + 动作经济 + 法术位成本 + 少量结构化效果 + 大段 `manual` 文本”。没有结构化等级/学派/成分/仪式/职业表，也没有已知/准备/法术书模型；`chargen` 只按自由文本计数，角色卡只把法术写在 Markdown 里。
- **元数据模型推荐**：不新增独立 `SpellDef`；在 `Ability` 上加一个可选的嵌套块 `spell: SpellMeta`。执行路径、内容包、角色卡 `AbilityRef`、`use`/`cast` 全部保持一套。
- **列表模型**：在 `sheets/<id>.yaml` 增加 `spells` 块，区分 `known` / `prepared` / `book` / `cantrips` / `always_prepared` / `legacy`；职业模式与数量来自 `tools/talkdnd/data/classes-<edition>.yaml` 与现有 `classdata::generate`，不按职业名写死在引擎里。
- **存档 schema**：推荐 `save-meta` 9 → 10 并带整档备份迁移；如果所有者选择维持 9，则必须把角色卡 `CharFile.version` 升到 2，否则旧版 `talkdnd` 在重写角色卡时会静默丢掉新字段。见 D-1。
- **唯一执行器**：机械效果只由 `combat-sim` 的 `cast`/`use` 解析；`talkdnd` 只管理列表、成分声明、仪式与法术位记录，绝不复制一套施法结算。
- **任务拆分**：5 个任务。T1 元数据与导入；T2 列表模型与迁移；T3 列表命令与角色卡 refs；T4 施法结算（成分/仪式/升环）；T5 常见法术数据、DM 文档与章程。T1 后可并行 T2 与 T4 的代码工作；T3 依赖 T2；T5 最后合并。

## 1. 现状：数据流与能力边界

### 1.1 端到端数据流

```text
上游 5e-database JSON
  └─ tools/combat-sim/src/srd_pack.rs::import_spells
       └─ tools/combat-sim/src/srd.rs::spell
            └─ Ability { id, name, tags, activation, cost, slot_level, range,
                         target, area, concentration, effects, manual, ... }
                 ├─ LetsPlay/Rules/combat/<edition>/srd/srd-*.pack.json
                 └─ LetsPlay/Rules/combat/<edition>/spells.yaml（手写，覆盖同 id 的 pack 条目）
                      └─ content/loader.rs（pack 先加载，普通文件按 id 覆盖）
                           └─ ContentRegistry.abilities
                                └─ Creature.actions / Creature.features 的 AbilityEntry
                                     └─ combat-sim cast / use
                                          └─ engine::use_ability
                                               └─ engine::use_resolved_ability
                                                    ├─ spend_action（activation）
                                                    ├─ spend_cost（AbilityCost）
                                                    ├─ begin_concentration
                                                    └─ apply_effects（Effect 列表）
```

`talkdnd` 一侧是另一条链，目前与上面的 `Ability` 只在 `spellcasting` 数值上相接：

```text
Saves/<slot>/chargen.json 的 Creation.spells（自由文本）
  └─ chargen::validate
       ├─ data::is_cantrip（静态 CANTRIPS 列表）
       └─ classdata::generate 的 cantrips / spells 数量
            └─ chargen::sheet_with / gen_md
                 ├─ Markdown 的 "## 法术" 列表（纯文本）
                 ├─ combat yaml 的 spellcasting: {ability, attack, dc}
                 └─ combat yaml 的 resources: spell-slot-N
                      └─ charsheet::from_chargen
                           └─ Saves/<slot>/sheets/<id>.yaml（CharFile，无法术列表字段）
                                └─ charsheet::Op::Use（只扣资源，不结算法术）
```

关键引用：

- 转换：`tools/combat-sim/src/srd.rs::spell`、`tools/combat-sim/src/srd_pack.rs::import_spells`。
- 数据：`LetsPlay/Rules/combat/2014/spells.yaml`、`LetsPlay/Rules/combat/2024/spells.yaml`、`LetsPlay/Rules/combat/<edition>/srd/srd-*.pack.json`。
- 加载顺序：`tools/combat-sim/src/content/loader.rs::load_source`；pack 先加载，普通文件后加载并按 id 覆盖。测试 `tools/combat-sim/tests/full_pack.rs::legacy_yaml_and_srd_packs_still_load_and_override_a_full_pack` 覆盖了“手写 YAML 覆盖 pack”的行为。
- 执行：`tools/combat-sim/src/engine.rs::use_ability`、`use_resolved_ability`、`spend_cost`、`begin_concentration`、`break_concentration`、`concentration_damage_check`。
- 施法数值：`tools/combat-sim/src/content/schema.rs::Spellcasting` 与 `engine.rs::score_context` 的 `{spell_attack}` / `{spell_dc}` / `{spell_mod}`。
- 角色创建：`tools/talkdnd/src/chargen.rs`（`Creation.spells`、`validate`、`sheet_with`、`gen_md`、`gen_yaml`）与 `tools/talkdnd/src/classdata.rs`（`ClassDef`、`generate`）。
- 角色卡：`tools/talkdnd/src/charsheet.rs`（`CharFile`、`from_chargen`、`Op::Use`、`Op::Rest`）与 `tools/talkdnd/src/progress.rs::rebuild_class_derived`。
- 存档版本：`tools/talkdnd/src/migrate.rs::CURRENT_SCHEMA`（当前 9）与 `charsheet.rs::store` / `parse_char_file`（`CharFile.version` 当前 1）。

### 1.2 已支持的能力

| 能力 | 实际位置 | 现状 |
|------|----------|------|
| 法术作为 `Ability` 存在 | `srd.rs::spell`、`srd_pack.rs::import_spells`、`content/schema.rs::Ability` | 支持；pack 的 `abilities` 列表与手写 `spells.yaml` 同 id 覆盖 |
| 动作/附赠/反应 | `Ability.activation`、`Activation` | 支持；`srd.rs::spell` 只看 `casting_time` 是否包含 `bonus` / `reaction`，其余一律 `action` |
| 法术位成本 | `AbilityCost { resource, amount }`、`spell-slot-N` | 支持；`spend_cost` 扣资源，`can_afford` 预检 |
| 法术位资源 | `ResourceDef`、`Recharge`、`chargen::gen_yaml`、`charsheet::Op::Use` | 支持；长休恢复 `Recharge::Long`，短休恢复 `Recharge::Short`，契约法术位写成 `spell-slot-L` + `notes: "pact magic"` |
| 施法攻击/DC | `Spellcasting { ability, attack, dc }`、`score_context` | 支持；`{spell_attack}` / `{spell_dc}` / `{spell_mod}` 可由数据使用 |
| 专注 | `Ability.concentration`、`begin_concentration`、`break_concentration`、`concentration_damage_check` | 支持；受伤时 DC = `max(10, damage / 2)` 的体质豁免，死亡/失能/替换会中断 |
| 结构化效果 | `Effect::{Attack, Save, Damage, Heal, ApplyCondition, RemoveCondition, ForcedMove, Use, Activate, Check, BasicAction, Choice, Note, ToggleTrigger}` | 支持这 14 种；但当前 SRD 法术 pack 的结构化覆盖很低（见 1.4） |
| 范围与区域 | `Ability.range`、`AreaSpec`、`engine::area_targets` | 支持；但导入的 SRD pack 目前没有 `area`，只有手写 2024 overlay 补了若干形状 |
| 持续时间 | `Ability.duration`（`rounds` / `until` / `minutes` / `permanent`） | 类型存在，但 `srd.rs::spell` 不写它；持续时间目前主要靠 `manual` 文本 |
| 仪式/学派 | `Ability.tags` 的 `ritual` / 学派 tag | 只是标签；没有仪式流程，也没有 `school` 字段 |
| 角色卡法术位 | `chargen::gen_yaml`、`charsheet.rs` 的资源显示、`progress.rs::rebuild_class_derived` | 支持生成、显示、升级重算、长休/短休恢复 |
| 角色卡施法数值 | `chargen.rs` 的 `spellcasting` 行、`progress.rs::rebuild_class_derived` | 支持按职业施法属性生成/重算 |
| 自由文本法术列表 | `Creation.spells`、`chargen::validate`、`gen_md` 的 `## 法术` | 只做数量校验与展示；不映射到内容 id，不进入 combat 块 |
| 战斗内施法 | `combat-sim cast` / `use` | 支持，但只解析角色 `actions` / `features` 里已有的 `AbilityRef`（`state.rs::find_owned_ability`） |

### 1.3 缺失的能力

| 缺失 | 影响 | 证据 |
|------|------|------|
| 结构化法术元数据 | 无法可靠区分等级、学派、成分、仪式、范围/持续时间文本、升环 | `Ability` 没有 `spell`/`school`/`components`/`ritual` 字段；只有 `tags` 与 `manual` |
| 职业法术表 | 无法校验“这个法术是否属于该职业”；当前只校验数量 | class pack 的 `data.spells` 是 URL 字符串（如 `/api/2014/classes/bard/spells`），不是 id 列表；`srd_pack::import_classes` 原样保留该字符串；导入后的 `Ability` 也没有 `classes` |
| 已知/准备/法术书列表 | 无法表达“准备了哪些”“法术书里有哪些”“戏法有哪些” | `CharFile` 没有 `spells` 字段；`Creation.spells` 是单一自由文本 `Vec<String>` |
| 准备约束 | 施法时不会检查该法术是否已准备 | `find_owned_ability` 只看角色卡 `actions`/`features` 的 refs；`chargen` 不生成法术 refs |
| 成分与材料 | 不能校验 V/S/M、昂贵材料、消耗材料 | `srd.rs::spell` 把 `components` 数组拼进 `manual` 文本；没有材料成本/消耗字段 |
| 仪式流程 | 仪式只是 tag，不减少法术位、不增加 10 分钟 | `srd.rs::spell` 只加 `ritual` tag；引擎没有 ritual 分支 |
| 升环与戏法成长 | `--slot N` 只影响 `{slot_level}` 上下文，不改变扣费，也不替换骰子 | `use_resolved_ability` 的 `spend_cost` 仍用 `ability.cost`；`srd.rs::first_dice` 只取最低等级骰；`todo` 明说未建模 |
| 戏法识别的版本差异 | 2024 新戏法可能被当成有环法术计数 | `data.rs::CANTRIPS` 是静态列表；2024 pack 的 `elementalism` / `sorcerous-burst` 不在其中，2014 的 `thaumaturgy` 也不在其中 |
| 法术书/抄书 | 没有法术书列表、抄书成本/时间、书内法术与准备列表的关系 | `classdata::ClassDef` 只有 `known` / `prepared` 数量，没有 `book` 或 `scribe` |
| 自动法术 `AbilityRef` | 新建角色默认不能 `cast` 选中的法术 | `classdata::generate` 的 `g.refs` 只来自职业特性；`chargen::gen_yaml` 只写这些 refs；选中的 `Creation.spells` 只写 Markdown |
| 战斗外施法 | 没有独立的战斗外施法入口 | `combat-sim cast` 需要 `--state`；`talkdnd char` 只有 `use slotN`（只扣资源）与 `heal`/`condition` 等手动命令 |

> **当前行为的准确表述**：`combat-sim cast` 只解析角色 `actions` / `features` 里已有的 `AbilityRef`（`state.rs::find_owned_ability`）。`chargen` 生成的 combat 块只写职业特性 refs（`classdata::generate` 的 `g.refs`），选中的法术只写 Markdown 的 `## 法术`（`chargen::gen_md`）。因此新建角色默认没有法术 ref；`LetsPlay/docs/combat.md` 里的 `cast` 示例只在角色卡已有对应 ref 的存档中成立。旧存档是否手工补过 ref、或是否有其他写入路径，需用真实存档复核（见 U-2）。

### 1.4 数据现状（只读统计）

本次设计对已跟踪的 SRD 法术 pack 做了只读统计（口径：读取 `content.abilities`，按 `effects` 的 key 分类；不含手写 `spells.yaml` 的覆盖）：

| 指标 | 2014 SRD pack | 2024 SRD pack |
|------|---------------|---------------|
| `abilities` 总数 | 319 | 339 |
| 只有 `note` 效果（manual） | 309 | 324 |
| `attack` 效果 | 0 | 15 |
| `heal` 效果 | 10 | 0 |
| `ritual` tag | 29 | 29 |
| `cantrip` tag | 24 | 27 |
| `concentration: true` | 126 | 132 |
| 有 `slot_level` | 295 | 312 |
| 有 `area` | 0 | 0 |
| 有 `duration` | 0 | 0 |

手写覆盖层增加了结构化条目：

- `LetsPlay/Rules/combat/2014/spells.yaml`：17 个法术（fire-bolt、shocking-grasp、sacred-flame、magic-missile、shield、cure-wounds、bless、sanctuary、sleep、detect-magic、mage-hand、minor-illusion、guidance、mending、healing-word、command、mage-armor）。
- `LetsPlay/Rules/combat/2024/spells.yaml`：18 个法术（前 12 个同上，另有 burning-hands、fireball、lightning-bolt、moonbeam、spirit-guardians、thunderwave；不含 guidance、mending、healing-word、command、mage-armor）。
- 2024 文件还包含 SRD 5.2.1 的 AoE 形状 overlay（注释在文件内），但多数仍是 `note` + `manual`，没有 `save`/`damage` 结构。
- 加载顺序保证手写 `spells.yaml` 覆盖同 id 的 pack 条目（`content/loader.rs::load_source` 先 pack 后普通文件；测试见上）。

**结论**：当前“法术可执行”的覆盖面远小于“法术存在”的覆盖面；绝大多数 SRD 法术只是 `manual` 备注。v1.6 的重点不是把所有法术变成自动结算，而是先把**元数据、列表与唯一执行路径**补齐，再按类别逐步提高结构化覆盖率。

### 1.5 现状结论

1. `Ability` 已经是唯一执行单位，且执行器只有 `combat-sim` 一套；这是最值得保留的资产。
2. 法术的“身份”不完整：等级/学派只在 tag 与 `slot_level` 里，成分/仪式/升环在 `manual` 文本里，职业表根本不在已跟踪数据里。
3. `talkdnd` 的法术是“建卡时数数量 + 角色卡上写一行字”，没有可查询、可准备、可抄书的结构。
4. 因此 v1.6 必须是“元数据 + 列表 + 执行器增强”三层一起设计，但实现要分层交付，避免一次性重写所有法术数据。

## 2. 法术元数据模型

### 2.1 两个方案

| 维度 | 方案 A：扩展 `Ability` 的可选字段 | 方案 B：新增独立 `SpellDef` |
|------|-----------------------------------|------------------------------|
| 执行路径 | 直接复用 `use_ability` / `use_resolved_ability` | 需要把 `SpellDef` 转成 `Ability`，或在引擎里加第二套解析 |
| 内容包 | 仍是 `content.abilities`，旧 pack 原样加载 | 新增 item kind / pack 字段；旧 loader 不认识，需版本升级 |
| 角色卡 | 仍是 `AbilityRef`，`find_owned_ability` 不变 | 需要 `SpellRef` 或在加载时转换；准备列表与 refs 关系更复杂 |
| 数据来源 | 一个 id、一个定义、一个覆盖顺序 | 两套 id 空间，容易出现 `Ability` 与 `SpellDef` 双重来源 |
| 校验 | 只能在 `spell` 块存在时校验法术字段 | 可以强制所有法术字段必填，但非法术 `Ability` 仍要另一套 |
| 改动量 | 小到中；加字段 + check + 导入 | 大；新类型、loader、registry、CLI、pack、测试 |
| 风险 | `Ability` 变大；法术专用字段出现在通用类型上 | 双模型漂移；旧内容/旧角色卡迁移；两套执行器风险 |

### 2.2 推荐：`Ability` + 可选的 `spell: SpellMeta`

推荐 **方案 A 的收敛版**：不把 20 个字段平铺到 `Ability`，而是加一个可选的嵌套块：

```rust
pub struct Ability {
    // 现有字段保持不变
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spell: Option<SpellMeta>,
}

pub struct SpellMeta {
    pub level: u8,                        // 0 = cantrip
    pub school: SpellSchool,
    pub casting_time: CastingTime,
    pub components: SpellComponents,
    pub ritual: bool,
    #[serde(default)]
    pub range_text: Option<String>,       // "Touch", "Self (60-foot cone)", "Sight"
    #[serde(default)]
    pub duration_text: Option<String>,    // "Instantaneous", "Up to 1 minute"
    #[serde(default)]
    pub upcast: Option<UpcastSpec>,
    #[serde(default)]
    pub cantrip_scaling: Option<CantripScaling>,
    #[serde(default)]
    pub classes: Vec<String>,             // 职业 id；空 = 未收录/未知
}
```

理由：

1. **一套执行路径**：`use_resolved_ability` 不需要知道 `SpellDef`；`spell` 只是附加数据。`spell` 里的 `upcast` 等字段由 `use_resolved_ability` 读取，仍然只有一条结算路径。
2. **一套内容来源**：`ContentRegistry.abilities`、`AbilityEntry::Ref`、`ResolvedAbility`、`find_owned_ability`、`content show` 全部不变；不会出现“`Ability` 与 `SpellDef` 同 id 但内容不同”的双重来源。
3. **旧内容零迁移**：`spell` 是 `Option` 且 `skip_serializing_if`；旧 `spells.yaml`、旧 SRD pack、旧角色卡、旧 full pack 都不带该字段，按 `None` 加载。旧 `cast`/`use` 行为不变。
4. **失败关闭可局部化**：只有带 `spell` 的条目才启用新校验（等级/成本一致、成分完整、升环表完整）；没有 `spell` 的旧数据继续按旧行为工作，`content check` 给 warning 而不是 error。
5. **不推荐独立 `SpellDef`**：如果 `SpellDef` 包住 `Ability`，它实际上等于 `Ability + SpellMeta`，只是多了一层转换和一个新的 item kind；如果 `SpellDef` 复制字段，就会产生两套模型与两套执行器风险，直接违反“唯一施法执行器”的目标。

**边界**：`spell` 只用于“法术”。职业特性、陷阱、物品动作继续用没有 `spell` 的 `Ability`；`content check` 不要求它们有 `spell`。如果未来需要“法术书物品”或“法术卷轴”，它们是物品/能力引用，不是新的执行器。

### 2.3 字段定义

| 字段 | 类型 | 必填 | 含义 | 2014 / 2024 差异 |
|------|------|------|------|------------------|
| `spell.level` | `u8` | 是（有 `spell` 时） | 0 = 戏法，1–9 = 有环 | 同；数据逐版 |
| `spell.school` | `SpellSchool` 枚举 | 是 | abjuration / conjuration / divination / enchantment / evocation / illusion / necromancy / transmutation | 同枚举；个别法术的学派逐版以数据为准 |
| `spell.casting_time` | `CastingTime { amount, unit }` | 是 | `action` / `bonus_action` / `reaction` / `minute` / `hour` / `special` | 2024 文本称 Magic action；引擎仍映射到 `Activation::Action` |
| `spell.components.verbal` | `bool` | 是 | 是否需要言语 | 同 |
| `spell.components.somatic` | `bool` | 是 | 是否需要姿势 | 同 |
| `spell.components.material` | `Option<MaterialComponent>` | 否 | 材料文本、价格门槛、是否消耗 | 逐法术数据；2024 的器材/材料表述以数据为准 |
| `spell.ritual` | `bool` | 是 | 是否可仪式施法 | 同机制；职业是否具备仪式施法由职业数据决定 |
| `spell.range_text` | `Option<String>` | 否 | 展示用范围文本（Touch / Self / Sight / Unlimited / Self (cone)） | 逐版文本 |
| `spell.duration_text` | `Option<String>` | 否 | 展示用持续时间文本（Instantaneous / Up to 1 minute / Until dispelled） | 逐版文本 |
| `spell.upcast` | `Option<UpcastSpec>` | 否 | 升环骰表 + 展示文本 | 逐版骰表；2014 “At Higher Levels”，2024 “Using a Higher-Level Spell Slot” |
| `spell.cantrip_scaling` | `Option<CantripScaling>` | 否 | 戏法按角色等级的骰表 | 逐版骰表；触发等级通常 5/11/17 |
| `spell.classes` | `Vec<String>` | 否 | 职业 id 列表；空表示未收录；推荐仅作派生/展示缓存，唯一来源见 2.4 与 D-2 | 2014/2024 职业表不同，逐版数据 |

复用的现有字段（不要重复造）：

| 现有字段 | 用途 | 与 `spell` 的关系 |
|----------|------|-------------------|
| `activation` | 引擎动作经济 | 必须与 `spell.casting_time.unit` 一致（action/bonus/reaction）；分钟/小时法术用 `activation: action` 仅作目录值，`content check` 给 warning |
| `cost` / `slot_level` | 默认法术位成本与基础环级 | `spell.level` 必须等于 `slot_level`（有环时）；`cost.resource` 默认 `spell-slot-{level}`，允许 `AbilityRef.cost` 覆盖（契约法术位/特殊资源） |
| `range` | 引擎距离（尺） | `range_text` 是展示文本；`range` 仍是引擎判定的数值，Touch=5、Self=0 |
| `target` / `area` | 目标与区域 | 不变；`spell` 不复制 |
| `concentration` | 专注 | 不变；`spell` 不复制 |
| `duration` | 结构化持续时间（用于条件/效果） | `duration_text` 是展示文本；两者可同时存在；纯即时法术可以只有 `duration_text` |
| `effects` | 结构化效果 | 不变；`upcast`/`cantrip_scaling` 只替换“主伤害/治疗骰”，不新增 Effect 类型 |
| `manual` | DM 可读文本 | 保留 SRD 原文/裁定；新字段是结构化补充，不是替代 |
| `tags` | 搜索与旧兼容 | 保留 `spell` / `cantrip` / `ritual` / 学派 tag；`content check` 在 `spell` 存在时校验 tag 与字段一致（warning） |
| `source` / `todo` / `notes` | 来源与未建模说明 | 不变 |

`MaterialComponent` 建议形状：

```rust
pub struct MaterialComponent {
    pub text: String,              // "a bit of fleece" / "a diamond worth 300 gp"
    #[serde(default)]
    pub cost_gp: Option<u32>,      // 有价格门槛的材料
    #[serde(default)]
    pub consumed: bool,            // 施法后消耗
}
```

`UpcastSpec` / `CantripScaling` 建议形状：

```rust
pub struct UpcastSpec {
    pub text: String,                       // 展示用 "At higher levels: ..."
    #[serde(default)]
    pub dice_by_level: BTreeMap<u8, String>, // 槽位等级 -> 主伤害/治疗骰表达式
}

pub struct CantripScaling {
    pub text: String,
    #[serde(default)]
    pub dice_by_character_level: BTreeMap<u8, String>,
}
```

这样升环不是“引擎猜每级加多少”，而是直接使用数据里的逐级骰表；2014/2024 的差异、非线性成长（每两级加一次）都能表达。引擎只负责按 `slot_level` 或角色等级选表，并把选中的骰子用于**主伤害/治疗效果**。

### 2.4 数据放置

| 数据 | 位置 | 维护方式 | 理由 |
|------|------|----------|------|
| 法术 mechanics | `LetsPlay/Rules/combat/<edition>/spells.yaml` | 手写/审校 | 法术逐版不同；已有覆盖层与加载顺序 |
| SRD 导入结果 | `LetsPlay/Rules/combat/<edition>/srd/srd-*.pack.json` | `combat-sim content import-srd` 生成，不手改 | 与现有 pack 流程一致；`spell` 字段由 `srd.rs::spell` / `srd_pack` 写入 |
| 职业进度（caster/known/prepared/cantrips/slots） | `tools/talkdnd/data/classes-2014.yaml`、`classes-2024.yaml` | 手写/审校 | `talkdnd` 已嵌入并用于建卡/升级；不需要引擎 |
| 职业法术表 | 推荐新增 `classes.<id>.spells: [spell-id, ...]`（同两个 YAML） | 手写/审校，后续可由 SRD 导入生成 | 当前 class pack 的 `data.spells` 是 URL，不是列表；`talkdnd` 需要离线可用；见 D-2 |
| 领域/子职始终准备 | `classes.<id>.subclasses.<sub>.always_prepared: [...]` | 手写/审校 | 不计入准备数量；与现有 `Sub` 结构一致 |
| 仪式施法资格 | `classes.<id>.ritual_casting: true/false` | 手写/审校 | 2014/2024 职业差异；不要按职业名硬编码 |
| 抄书/仪式数值 | 推荐 `rules.spellcasting`（`common/rules.yaml` 形状 + 各版覆盖） | 数据 | 时间/金币逐版不同；引擎只读值，不硬编码 |
| 法术书数量 | `classes.<id>.book_per_level` 或 `book: {start, per_level}` | 手写/审校 | 只有 wizard 使用；见 D-2 |

**不放在 `common/`**：法术、职业表、仪式资格都逐版不同；`common/` 只放共享的规则形状（例如 `rules.spellcasting` 的字段名与默认语义）。这与现有 `rules.combat_actions` / `rules.exhaustion` 的分层一致。

### 2.5 与现有 `Ability` 执行路径和内容包的兼容

- **旧内容**：没有 `spell` 的 `Ability` 完全按旧路径执行；`spell` 是 `Option`，不改变 `activation`、`cost`、`effects`、`manual`。
- **旧角色卡**：`AbilityRef` 不变；`find_owned_ability` / `resolve_ability` 不变。角色卡不需要因为 `spell` 字段改格式。
- **旧 pack**：新二进制读旧 pack，`spell: None`；`content check` 对“有 `spell` tag 但没有 `spell` 元数据”只给 warning，不阻止旧内容运行。
- **旧二进制读新 pack**：`Ability` 有 `deny_unknown_fields`，旧二进制会拒绝含 `spell` 的新 pack。这是**失败关闭**，不是静默降级；pack 与二进制同版本发布，FULL pack 由玩家本机构建，版本不匹配时应报错并提示更新二进制。设计上不承诺旧二进制读新 pack。
- **content hash**：带 `spell` 的 pack 重新生成后 hash 改变；旧 pack 的 hash 不变。不要为了让 hash 不变而省略字段。
- **`content check` 新增检查**（建议，均不改变旧内容）：
  - `spell.level` 与 `slot_level` 一致；有环法术的 `cost.resource` 与 `spell-slot-{level}` 一致（允许 `AbilityRef.cost` 覆盖）。
  - `spell.components.material.cost_gp` / `consumed` 一致（有价格或消耗时必须有 `text`）。
  - `spell.upcast.dice_by_level` 的 key 必须 ≥ `level`；`cantrip_scaling` 的 key 必须 ≥ 1 且覆盖 5/11/17（若使用）。
  - `spell.school` 与学派 tag、`spell.ritual` 与 `ritual` tag 不一致时 warning。
  - 只有在该数据源承诺携带 `spell.classes` 时，`classes` 为空才 warning；若职业表以 `classes-*.yaml` 为唯一来源（推荐 A），不强制 `spell.classes`。
- **`content list` / `content show`**：展示 `spell` 字段；`content show` 不执行法术，只读。

### 2.6 2014 / 2024 差异

| 主题 | 2014 | 2024 | 数据位置 |
|------|------|------|----------|
| 职业施法模式 | Bard/Ranger/Sorcerer/Warlock 为已知；Cleric/Druid/Paladin/Wizard 为准备/法术书 | 所有 SRD 施法职业都改为准备/法术书（Warlock 仍契约位） | `classes-*.yaml` |
| 准备数量 | Cleric/Druid `施法属性+等级`；Paladin `施法属性+半等级`；Wizard `智力+等级` | 使用职业表的 `prepared_spells` | `classes-*.yaml` + class pack level rows |
| 戏法替换 | 多数在升级时按职业文本替换 | 多数在长休时替换 | `classes-*.yaml` 的 `cantrip_replace` |
| 代表法术数值 | Cure Wounds `1d8+mod`；Sleep 是 HP 池、无豁免 | Cure Wounds `2d8+mod`；Sleep 是 5 尺球、WIS 豁免/失能 | `2014/spells.yaml` 与 `2024/spells.yaml` |
| 成分/材料 | V/S/M；器材/材料包可替代无价格材料 | 同结构；具体法术文本与材料以 2024 数据为准 | `spells.yaml` 的 `spell.components` |
| 仪式 | 10 分钟、不消耗法术位；职业资格按职业特性 | 同机制；资格按 2024 职业特性 | `classes-*.yaml` + `rules.spellcasting` |
| 升环 | “At Higher Levels” 文本 | “Using a Higher-Level Spell Slot” 文本 | `spell.upcast` |
| 戏法成长 | 角色等级 5/11/17 | 同 | `spell.cantrip_scaling` |

**不要**在引擎里写 `if edition == ...`；所有差异走数据。这与 `rules.combat_actions` / `rules.exhaustion` / `rules.ready` 的既有做法一致。

### 2.7 待探测

- 上游 `5e-SRD-Spells.json` 是否包含 `classes` 数组；即使有，`srd.rs::spell` 目前不读取。见 U-1。
- FULL 包是否包含职业法术表；取决于 `full_pack` 构建器读取的书籍数据。见 U-5。
- `spell.casting_time` 的分钟/小时法术在 `combat-sim cast` 中应拒绝还是仅 warning；推荐拒绝（不是战斗动作），见 D-5。

## 3. 已知 / 准备 / 法术书 / 戏法模型

### 3.1 四种模式的准确含义

“四种模式”不是四个互斥职业类型；**戏法是正交列表**，每个施法职业都可能有或没有：

| 模式 | 含义 | 谁使用 | 变更时机 |
|------|------|--------|----------|
| `known` | 角色知道一组有环法术；这些就是可施法列表 | 2014 Bard/Ranger/Sorcerer/Warlock；2024 的 SRD 职业不再使用 | 升级时按职业表学习/替换；长休不改变 |
| `prepared` | 角色从职业法术表（或法术书）中每天准备一个子集 | 2014 Cleric/Druid/Paladin/Wizard；2024 所有 SRD 施法职业 | 长休时替换；数量按职业表/公式 |
| `spellbook` | 角色有一本法术书，书记录可准备的法术；准备列表是书的子集 | 2014/2024 Wizard | 升级 +2 到书；抄书增加；长休准备子集 |
| `cantrips` | 戏法列表；不占准备数量，不消耗法术位 | 除 Paladin/Ranger 外的 SRD 施法职业 | 2014 多数升级替换；2024 多数长休替换；按职业数据 |
| `pact`（槽位） | 契约法术位：数量少、等级固定、短休恢复 | Warlock | 与 `known`/`prepared` 正交；`classes-*.yaml` 的 `caster: pact` |

一个角色同时有：`mode`（known/prepared/spellbook）+ `cantrips` 列表 + 可选 `always_prepared`；Warlock 额外有 `pact` 槽位。**不要**把 `pact` 当成第四种列表模式。

### 3.2 职业映射

| 职业 | 施法属性 | 2014 模式 | 2024 模式 | 2014 数量来源 | 2024 数量来源 |
|------|----------|-----------|-----------|---------------|---------------|
| Bard | CHA | known | prepared | `classes-2014.yaml` 的 `known` 表 + `cantrips` 表 | `classes-2024.yaml` 的 `prepared` 表 + `cantrips` |
| Cleric | WIS | prepared | prepared | `prepared: wis+level` + `cantrips` | `prepared` 表 + `cantrips` |
| Druid | WIS | prepared | prepared | `prepared: wis+level` + `cantrips` | `prepared` 表 + `cantrips` |
| Paladin | CHA | prepared | prepared | `prepared: cha+half`；无戏法 | `prepared` 表；无戏法 |
| Ranger | WIS | known | prepared | `known` 表；无戏法 | `prepared` 表；无戏法 |
| Sorcerer | CHA | known | prepared | `known` 表 + `cantrips` | `prepared` 表 + `cantrips` |
| Warlock | CHA | known（pact 槽） | prepared（pact 槽） | `known` 表 + `cantrips`；`pact(lvl)` | `prepared` 表 + `cantrips`；`pact(lvl)` |
| Wizard | INT | spellbook + prepared | spellbook + prepared | 书 `6 + 2×(level−1)`；准备 `int+level`；`cantrips` | 书 `6 + 2×(level−1)`；`prepared` 表；`cantrips` |

说明：

- 这张表是**当前已跟踪数据**的映射：`tools/talkdnd/data/classes-2014.yaml` 与 `classes-2024.yaml` 已有 `caster` / `cantrips` / `known` / `prepared`；`classdata::ClassDef` 与 `classdata::generate` 已经在用它们。v1.6 不需要重新发明数量表，只需要把“模式”和“法术列表”补上。
- 2014/2024 的“已知 vs 准备”差异不是猜测：2014 的 Bard/Ranger/Sorcerer/Warlock 在 `classes-2014.yaml` 里是 `known`，2024 对应职业在 `classes-2024.yaml` 里是 `prepared`；class pack 的 level rows 也分别有 `spells_known` 与 `prepared_spells`。
- 子职的“始终准备”（领域法术、2024 子职法术）放在 `classes.<id>.subclasses.<sub>.always_prepared`；它们不占准备数量，也不能被 `unprepare` 移除。
- **职业法术表目前不存在**：class pack 的 `data.spells` 是 URL（如 `/api/2014/classes/bard/spells`），不是 id 列表；`tools/talkdnd/data/classes-*.yaml` 也没有 `spells` 字段。v1.6 必须新增一个职业法术表来源（推荐 `classes.<id>.spells`），否则“准备”只能校验数量，不能校验“这个法术属于该职业”。见 D-2。

### 3.3 数量与校验

- `classdata::generate` 已经返回 `g.cantrips`、`g.spells`（`("已准备/可准备", n)` 或 `("已知法术", n)`）、`g.slots`、`g.pact`；`chargen::validate` 已用它们做“太多是 error、太少是 warning”的检查。
- v1.6 复用这些数量，但把“自由文本分类”改为结构化列表：
  - `cantrips.len()` 对比 `g.cantrips`；
  - `known.len()`（known 模式）或 `prepared.len()`（prepared/spellbook 模式）对比 `g.spells.1`；
  - `always_prepared` 不计入；
  - 法术书 `book.len()` 至少为 `6 + 2×(level−1)`（wizard），抄书只能增加。
- 校验时机：`chargen validate`、`chargen sheet`、`talkdnd level-up`、`talkdnd char <name> spells sync`、`talkdnd char <name> prepare/unprepare/scribe`。
- 失败关闭：未知法术 id、法术等级高于当前可施法环级、不在职业表、不在法术书、超过准备数量、重复添加、`unprepare` 一个 `always_prepared` 法术，都必须给出明确错误并**不写任何文件**。
- 旧 `data::is_cantrip` 只作为 legacy 回退；新逻辑以 `spell.level == 0`（或 `tags` 含 `cantrip`）为准，避免 2024 `elementalism` / `sorcerous-burst` 被当成有环法术。

### 3.4 长休 / 升级 / 抄书 / 仪式

| 事件 | known | prepared | spellbook | cantrips | 备注 |
|------|-------|----------|-----------|----------|------|
| 长休 | 不改变已知列表 | 可以替换准备列表；数量按表/公式 | 从书中准备子集；可以替换 | 2024 多数职业可替换一个戏法（按职业数据） | 工具只改列表并写日志；不自动长休、不自动回血/回法术位 |
| 升级 | 按职业表学习新法术；可替换一个（按职业文本） | 准备上限可能增加；选择新准备法术 | 书 +2 个法术；准备上限可能增加 | 数量可能增加；选择新戏法 | `talkdnd level-up` 重算槽位/施法数值；列表变化由玩家选择 |
| 抄书 | 不适用 | 不适用 | 只有 wizard：把法术加入书；花费 `rules.spellcasting.scribe` 的时间/金币；不自动准备 | 不适用 | 需要 DM 确认来源（卷轴/书）；`talkdnd char <name> scribe` |
| 仪式 | 若职业有仪式施法且法术在已知列表 | 若职业有仪式施法且法术已准备 | 若职业有仪式施法且法术在书中 | 不适用（戏法无仪式） | 10 分钟、不消耗法术位；战斗外；`talkdnd char <name> ritual` |

**准备时机**：RAW 是长休时替换准备列表。工具不强制“必须刚长休”，但会在 `prepare`/`unprepare` 时写一条日志并提示“这是长休时的选择”；如果存档有世界时间，可建议 `talkdnd time` 记录。是否强制长休门禁是 D-8。

**升级**：`progress.rs::rebuild_class_derived` 已经在新等级重算资源上限、法术位、`spellcasting` 攻击/DC，并保留当前值（按新上限截断）。v1.6 需要在同一路径增加“列表数量/新增选择”的检查；`level-up` 不应静默删除玩家已有的法术。

**抄书**：成本/时间不硬编码。推荐在 `rules.spellcasting` 放 `scribe_hours_per_level`、`scribe_cost_gp_per_level`（common 形状，2014/2024 覆盖）；具体数值由实现任务从 SRD 文本核对后写入。`talkdnd char <name> scribe` 只加书、扣金币（`progress` 的钱袋）、写日志；不自动准备。

**仪式**：职业资格放 `classes.<id>.ritual_casting`；`talkdnd char <name> ritual` 校验“法术有 `ritual: true` + 职业有资格 + 法术在 known/prepared/book 中”，不消耗法术位，写一条“10 分钟”的日志；不自动推进世界时间。见 D-7。

### 3.5 存档位置

推荐在 `Saves/<slot>/sheets/<id>.yaml` 的 `CharFile` 增加一个可选块：

```yaml
spells:
  version: 1
  mode: prepared          # known | prepared | spellbook
  ability: wis            # 施法属性，冗余存储用于审计；与 classdata 不一致时 fail closed
  cantrips: [guidance, sacred-flame, resistance]
  prepared: [bless, cure-wounds, shield-of-faith]
  known: []               # known 模式使用；prepared 模式为空
  book: []                # wizard 使用；其他职业为空
  always_prepared: []     # 领域/子职；不计入数量
  legacy: []              # 无法映射的旧自由文本名字
```

- **唯一数据源**：`sheets/<id>.yaml` 的 `spells` 块是列表的唯一来源；Markdown 的 `## 法术` 是生成物/展示，不是数据源。
- **当前值仍在 `party-state.yaml`**：`spell-slot-N` 的 `current`/`max` 继续由 `combat/party-state.yaml` 管；`spells` 块只存“列表与选择”，不存当前法术位。这个分工与 `LetsPlay/docs/character-data.md` 的现有约定一致。
- **`CharFile.version`**：如果 `save-meta` 升 10，`CharFile.version` 可以保持 1（旧二进制在 `save-meta` 就被拒绝）；如果 `save-meta` 保持 9，则必须升 `CharFile.version` 到 2，否则旧版 `charsheet::store` 会重写 YAML 并丢掉 `spells` 块。见 D-1。
- **`chargen.json`**：继续是“建卡中”的状态；确认后 `sheets` 是来源。`Creation.spells` 保留为旧输入兼容，新增 `Creation.spell_lists` 由 `set spells` 自动填充/由 `spells adopt` 填充。

### 3.6 存档 schema：保持 9 还是升 10

| 选项 | 做法 | 优点 | 缺点 | 结论 |
|------|------|------|------|------|
| A：保持 9，additive | `spells` 块可选；不迁移 | 无迁移、无备份；旧存档直接加载 | 旧版 `talkdnd` 会忽略未知字段，并在 `charsheet::store` 重写时**静默丢失** `spells`；没有全档门禁；需要把 `CharFile.version` 升 2 才能失败关闭 | 不推荐 |
| B：升 10，带迁移 | `CURRENT_SCHEMA = 10`；整档备份；给每个 sheet 加空 `spells` 块或从 `chargen.json`/正文精确 adopt；写迁移记录 | 旧二进制在 `save-meta` 直接拒绝；整档备份可回退；与 `docs/versioning.md` 的“结构变化 → 次版本 + 迁移”一致 | 迁移代码与测试；需要更新 `docs/PROJECT-CHARTER.md`、`LetsPlay/docs/layout.md`、`CHANGELOG`；`migrate.rs` 的测试期望从 9 改 10 | **推荐** |

推荐 B 的迁移步骤（实现任务 T2）：

1. `talkdnd migrate --dry-run` 报告 schema 9 → 10、备份位置、将处理的 sheet。
2. 整档备份到 `Saves/_backups/<slot>-<时间>-schema9`（沿用 `migrate.rs::backup_save` 的命名/去重规则）。
3. 对每个 `sheets/*.yaml`：
   - 已有 `spells` 块：只补 `version`/缺省字段，不覆盖玩家列表；
   - 没有 `spells` 块：若 `chargen.json.spells` 存在且 id 匹配，按精确 id/name 映射到 `known`/`prepared`/`book`/`cantrips`；无法映射的写入 `legacy`；
   - 否则从 Markdown body 的 `## 法术` 段解析 `- <名字>` 行，精确映射；无法映射写 `legacy`；
   - 不自动改 `combat` 块的法术 refs（避免改变旧行为）；由 `talkdnd char <name> spells sync` 后续显式生成。
4. `save-meta.schema = 10`，追加迁移记录与会话日志；幂等：已是 10 的存档不再写。
5. 回退：恢复整档备份；`talkdnd migrate --dry-run` 不写文件。

**注意**：`migrate.rs::ensure` 对“玩家已改过”的 dirty 文件不重写（保留手改/未知字段）；v9→10 迁移也要遵守这个规则：dirty sheet 只报告、不覆盖，`spells` 块缺失时按 legacy 处理。这样不会因为迁移丢掉玩家手写内容。

### 3.7 旧自由文本 `spells` 兼容

| 来源 | 迁移策略 | 失败关闭 |
|------|----------|----------|
| `chargen.json` 的 `Creation.spells` | `set spells` 仍接受自由文本；`spells adopt`/`chargen sheet` 按 id/name 精确映射；映射成功写入对应列表 | 未知/歧义名字写入 `legacy` 并 warning；不猜测 |
| 已确认角色卡的 Markdown `## 法术` | `talkdnd char <name> spells adopt` 解析 `- 名字` 行；只做精确 id/slug/name 匹配 | 未匹配/歧义 → `legacy`，列出原名字；不删除正文 |
| 旧 `spell-slot-*` | 不改名、不改当前值；继续由 `party-state.yaml` 管 | 迁移不重算当前值 |
| 旧 `cast`/`use` | 没有 `spells` 块时不改变行为；有 ref 就能 `cast` | `talkdnd prepare` 在未 adopt 时拒绝，提示先 `spells adopt` |
| 旧 `data::is_cantrip` | 保留为 legacy 回退；新数据优先 `spell.level == 0` | 2024 新戏法不再被误计为有环法术 |

**不自动 adopt**：自由文本可能包含书外法术、中文名、错别字或玩家自创名；自动模糊匹配会制造错误数据。只做精确匹配，其余明确列入 `legacy`，由 DM/玩家用 `talkdnd char <name> spells adopt --map "<原名>=<id>"` 逐条确认。见 D-9。

### 3.8 待探测

- 旧存档是否真的存在“手工补过法术 ref”的角色卡；仓库没有 `Saves/` 内容。见 U-2。
- `chargen.json` 在确认后是否仍保留、以及它是否总能与 `sheets` 对齐。见 U-6。
- 2024 各职业的戏法替换时机是否都能从 class pack 文本稳定解析；不能解析时用 `classes-*.yaml` 显式字段。见 U-3。

## 4. CLI / DM 流程

### 4.1 命令面

| 命令 | 新增/现有 | 作用 | 是否结算效果 |
|------|-----------|------|--------------|
| `talkdnd char <名字> spells [--json]` | 新增 | 列出 mode、戏法、已知/准备/法术书、始终准备、法术位、legacy；显示数量 `n/max` | 否 |
| `talkdnd char <名字> spells --class-list [--level N]` | 新增 | 列出该职业当前可选的职业法术（数据缺失时明确说“职业表未收录”） | 否 |
| `talkdnd char <名字> prepare <法术>` | 新增 | 把法术加入 `prepared`；校验职业表/书/等级/数量 | 否 |
| `talkdnd char <名字> unprepare <法术>` | 新增 | 从 `prepared` 移除；`always_prepared` 拒绝 | 否 |
| `talkdnd char <名字> learn <法术>` | 新增 | known 模式加入 `known`；按升级/职业表校验 | 否 |
| `talkdnd char <名字> scribe <法术> [--hours N] [--cost-gp N]` | 新增 | wizard 抄书：加入 `book`，扣金币/记时间；不准备 | 否 |
| `talkdnd char <名字> ritual <法术>` | 新增 | 校验仪式资格/列表；不消耗法术位；写 10 分钟日志 | 否（效果由 DM 叙述） |
| `talkdnd char <名字> spells adopt [--map 原名=id]` | 新增 | 把旧自由文本/正文名字精确迁入列表；未匹配进 `legacy` | 否 |
| `talkdnd char <名字> spells sync` | 新增 | 按列表重建角色卡 combat 块的 `actions`/`features` 法术 refs（保留非法术 refs） | 否 |
| `talkdnd char <名字> use slotN` | 现有 | 只扣法术位/资源；用于手动/叙述型法术 | 否 |
| `combat-sim cast <谁> <法术> [目标...] [--slot N] [--material ...] [--ignore-components]` | 现有，扩展 | 唯一机械结算入口；动作/附赠/反应、法术位、专注、效果 | **是** |
| `combat-sim use <谁> <法术/攻击> ...` | 现有 | 攻击或能力的统一入口；法术仍走 `use_ability` | **是** |
| `combat-sim content show <法术>` | 现有 | 查看 `spell` 元数据、components、ritual、upcast、manual | 否 |
| `combat-sim content list --kind ability --tag spell` | 现有 | 列出所有法术条目 | 否 |
| `combat-sim status --json` | 现有 | 资源、专注、状态；用于查看法术位/专注 | 否 |

**不做**：`talkdnd char <名字> cast`。如果将来需要战斗外施法，应让 `combat-sim` 在临时 state 上执行同一个 `use_ability`，而不是在 `talkdnd` 里复制效果结算。见 D-5。

### 4.2 如何复用现有 `use` / `cast`

1. **准备列表决定 refs**：`talkdnd char <名字> spells sync` 根据 `spells` 块生成角色卡 combat 块的 `actions`/`features` 法术 refs：
   - known：`known` + `cantrips` + `always_prepared`；
   - prepared：`prepared` + `cantrips` + `always_prepared`；
   - spellbook：`prepared` + `cantrips` + `always_prepared`（书本身不是可施法列表，除非已准备）。
   - 非法术 refs（职业特性、攻击等）原样保留；只增删/刷新能识别为法术的 refs。
2. **引擎仍然只认 refs**：`find_owned_ability` 继续只查 `actions`/`features`；因此“未准备”在引擎侧表现为“没有该 ref”，`cast` 报 `has no ability matching`。不需要在引擎里新增“准备列表”字段，也不需要第二套执行器。
3. **`cast` 扩展**：默认扣除 `Ability.cost`（或 `AbilityRef.cost`）指定的资源；`--slot N` 显式改为扣除 `spell-slot-N`；`--material` 声明昂贵/消耗材料；`--ignore-components` 是 DM 显式越权并写日志。
4. **`use` 不变**：`use` 仍先找攻击再找能力；法术走 `use_ability`。`heal --slot` 仍是资源助手，不是法术执行器。
5. **`talkdnd` 不结算**：`prepare`/`unprepare`/`scribe`/`ritual` 只改列表、资源或日志；不调用 `apply_effects`，不写 HP/条件。

### 4.3 稳定输出与错误语义

| 场景 | 输出 | 退出码 | 写入 |
|------|------|--------|------|
| `spells` 文本 | 头行 + 每个列表一行，按法术等级、id 排序；数量 `n/max` | 0 | 无 |
| `spells --json` | 固定键：`mode`、`ability`、`cantrips`、`known`、`prepared`、`book`、`always_prepared`、`legacy`、`slots`、`counts` | 0 | 无 |
| `prepare` 成功 | `准备 <id>（prepared n/max）` | 0 | 原子写 sheet；一行会话日志 |
| `prepare` 已准备 | `已准备 <id>（prepared n/max）` | 0 | 无（幂等） |
| `unprepare` 未准备 | `未准备 <id>（prepared n/max）` | 0 | 无（幂等） |
| `prepare` 未知/不在职业表/超数量/不在书 | `error: ...`，说明原因与补救命令 | 1 | 无 |
| `scribe` 成功 | `抄录 <id> → 法术书（n）；花费 X gp、Y 小时；未准备` | 0 | 原子写 sheet + 金币 + 日志 |
| `scribe` 非 wizard/已在书/金币不足 | `error: ...` | 1 | 无 |
| `ritual` 成功 | `仪式 <id>（10 分钟，不消耗法术位）` | 0 | 日志；不改槽位 |
| `ritual` 非仪式/无资格/未准备 | `error: ...` | 1 | 无 |
| `combat-sim cast` 成功 | 现有日志（`X casts Y`、扣费、豁免/伤害/治疗）+ 新元数据行（如有） | 0 | 战斗 state 正常保存 |
| `combat-sim cast` 未拥有/未命中/无槽/材料缺失/不能升到该环 | `Error: ...`，含法术、槽位、材料与补救 | 1 | 不保存半成品 |

约定：

- 0 = 成功（包括幂等 no-op）；1 = 运行时/数据错误；2 = clap 用法错误（现有 CLI 行为）。不要为新命令引入第三套退出码。
- 文本输出保持一行一条状态；`--json` 不混入中文提示，键名/顺序稳定。
- 所有 `talkdnd char` 写操作走现有 `atomic_write`/`commit` 路径；任何校验失败都不写文件（与 `charsheet` 的 batch 原子性一致）。
- `cast` 的现有成功日志不要改写；新信息只**追加**，避免破坏 DM 习惯与现有测试。

### 4.4 战斗外施法 / 仪式

- 战斗外机械结算（治疗、伤害、条件）当前没有 `--state` 就无法走 `combat-sim cast`。v1.6 推荐：
  - 叙述/效用法术：`talkdnd char <名字> use slotN` 扣法术位 + DM 叙述；需要治疗/条件时用现有 `talkdnd char heal`/`condition` 手动记录；
  - 仪式：`talkdnd char <名字> ritual <法术>` 只校验与记日志，不自动推进时间；
  - 如果所有者要求“战斗外也自动结算”，另开任务让 `combat-sim` 用临时 state 跑 `use_ability`，不在 `talkdnd` 里写第二套效果。见 D-5。
- 这条边界是**有意的**：v1.6 的目标是“施法核心”，不是“所有场景都能自动结算”。

### 4.5 无第二执行器检查清单

- [ ] `talkdnd` 不调用 `apply_effects` / `use_resolved_ability` / `use_ability`。
- [ ] `talkdnd` 不写 HP、临时 HP、条件、死亡豁免；这些仍走 `charsheet`/`party-state` 的既有命令。
- [ ] `talkdnd char use slotN` 只扣资源；不根据法术 id 推断效果。
- [ ] `combat-sim cast` 与 `use` 共用 `use_ability` → `use_resolved_ability`；没有第二条效果解析路径。
- [ ] `heal --slot` 仍是资源助手；不要把它扩成“治疗法术执行器”。
- [ ] `content show` 只读；不执行。

## 5. 成分、仪式与常见效果

### 5.1 V / S / M 校验

| 成分 | 规则要点（两版共同） | v1.6 的硬校验 | 说明 |
|------|----------------------|----------------|------|
| V 言语 | 必须能说话；沉默/失能不能施法 | 数据完整性 + 展示；引擎没有 `silenced` 条件，暂不硬判 | `content show`/`spells` 显示 V；DM 裁定沉默 |
| S 姿势 | 至少一只手自由；器材/盾牌可占用 | 数据完整性 + 展示；引擎没有手部/装备栏，暂不硬判 | 不假装引擎知道手是否空 |
| M 材料 | 材料包/器材可替代无价格材料；有价格或消耗的材料必须具体准备 | **有价格门槛或 `consumed: true` 时，`cast` 必须收到 `--material "<文本>"`**，否则失败 | 这是“声明”而非库存自动检查；见 D-3 |
| 器材/材料包 | 可替代无价格的材料成分 | `spell.components.material.cost_gp == None && !consumed` 时默认有器材/材料包；不要求 flag | 2024 器材规则以数据/文本为准 |
| 材料消耗 | `consumed: true` 施法后消耗 | `cast` 记录“材料已消耗”；`talkdnd` 可后续扣物品；v1.6 不自动从物品栏删除 | 见 D-3 |

**为什么不做完整手部/库存模型**：引擎没有装备栏（v1.5 已知限制），角色卡物品是自由文本；强行做会引入大量误判。v1.6 只对“有价格/会消耗”的材料做硬性声明检查，其余由 DM 按桌规处理。`--ignore-components` 是 DM 显式越权，必须写日志。

### 5.2 材料字段与数据

- `MaterialComponent.text`：SRD 文本（例如“a diamond worth 300 gp”），用于展示与 `--material` 匹配。
- `MaterialComponent.cost_gp`：有价格门槛时填写；没有就 `None`。
- `MaterialComponent.consumed`：施法后消耗。
- `spell.components.material: None`：不需要材料。
- `content check`：`cost_gp`/`consumed` 为真但 `text` 为空 → error；`material` 存在但 `components.verbal/somatic` 都 false 且没有文本 → warning。
- `talkdnd char <名字> spells` 显示材料文本与“昂贵/消耗”标记；`combat-sim content show` 显示同样字段。
- 库存自动扣减不在 v1.6；如果所有者要求，需另开任务并定义物品 id/匹配规则。

### 5.3 仪式

- `spell.ritual: true` + 职业 `ritual_casting: true` + 法术在 known/prepared/book 中。
- 仪式施法时间 = 正常施法时间 + 10 分钟；**不消耗法术位**；可以专注（如 Detect Magic）。
- 战斗内不适用（10 分钟 >> 战斗轮）；`combat-sim cast` 不提供 `--ritual`。`talkdnd char <名字> ritual` 是唯一仪式入口，只校验/记日志，不结算效果。
- 2014/2024 的仪式资格差异放 `classes-*.yaml`；不要按职业名硬编码。
- 如果仪式法术需要材料，仍按 5.1 的“有价格/消耗”规则提示；仪式不因为“不耗槽”而免材料。

### 5.4 专注与打断

现有实现已经覆盖，v1.6 不重写：

- `Ability.concentration` → `begin_concentration`；新专注替换旧专注，并清掉由旧专注维持的条件。
- 受伤 → `concentration_damage_check`：DC = `max(10, damage / 2)` 的 CON 豁免；失败 `break_concentration`。
- 死亡、失能、0 HP、休眠、长休 → 结束专注；`on_death` 清理来源为该施法者的专注条件。
- Ready：v1.5c 的准备法术在设置时支付并开始专注，触发不再重复支付；专注被打断后准备法术不能释放且不退槽（已知限制）。
- v1.6 新增的 `upcast`/成分/仪式不改变上述语义；`cast --slot` 仍走同一个 `begin_concentration`。

### 5.5 升环与戏法成长

- `spell.upcast.dice_by_level`：槽位等级 → 主伤害/治疗骰表达式；`cast --slot N` 选择 N 后：
  1. 校验 N ≥ `spell.level`（戏法不能升环）；
  2. 扣 `spell-slot-N`（`--slot` 显式覆盖；契约法术位用 `spell-slot-L`）；
  3. 若表中有 N，用该骰子替换**主伤害/治疗效果**的骰子；若表中没有 N 且 N > `level`，失败关闭（数据不完整，不猜）；
  4. 若法术没有 `upcast`，N > level 时允许施法但只产生基础效果，并记录“该法术未建模升环”；
  5. `slot_level` 上下文设为 N，供 `{slot_level}` 与 `Predicate::SlotLevelMin` 使用。
- `spell.cantrip_scaling.dice_by_character_level`：角色等级 → 主伤害骰；`cast` 无槽，按角色等级选最高不超过当前等级的 key；没有表则用基础骰。
- **只替换主效果**：`AttackEffect.damage` 的第一项、`SaveEffect.on_fail` 中的第一个 `Damage`、或 `HealEffect`。多段/多目标/特殊成长仍 `manual`；`content check` 对“有 `upcast` 但找不到主伤害/治疗”给 warning。
- 2014/2024 的数值差异全部在 `dice_by_level` / `dice_by_character_level` 数据里；引擎不写“每级 +1d8”之类的规则。

### 5.6 常见效果覆盖矩阵

| 法术类别 | 现有 `Effect` | 需要新 effect？ | v1.6 处理 | 例子 |
|----------|---------------|-----------------|-----------|------|
| 攻击骰 + 伤害 | `Attack` | 否 | 结构化数据 | Fire Bolt、Guiding Bolt |
| 豁免 + 伤害/半伤 | `Save { half, on_fail }` | 否 | 结构化数据 + `AreaSpec` | Fireball、Burning Hands |
| 自动命中伤害 | `Damage { repeat }` | 否 | 结构化数据 | Magic Missile |
| 治疗 | `Heal` | 否 | 结构化数据 | Cure Wounds、Healing Word |
| 条件/增益/减益 | `ApplyCondition` + `Modifier` | 否 | 结构化数据；复杂buff仍 manual | Bless、Bane、Sanctuary |
| 强制移动 | `ForcedMove { distance, direction }` | 否 | 结构化数据；放在 `Save.on_fail` 下 | Thunderwave、Gust of Wind |
| 召唤 | 无 | 暂不新增 | `Note` + `manual` | Conjure Animals |
| 变形 | 无 | 暂不新增 | `Note` + `manual` | Polymorph |
| 传送 | 无 | 暂不新增 | `Note` + `manual` | Misty Step |
| 持续区域/墙 | 无（`AreaSpec` 只用于瞬时区域） | 暂不新增 | `Note` + `manual` | Wall of Fire |
| 幻术/侦查/社交 | `Note` / `manual` | 否 | manual | Minor Illusion、Detect Magic |
| 反制/解除 | 无（没有 `on_spell_cast` trigger） | 暂不新增 | manual | Counterspell、Dispel Magic |
| 复活 | 无 | 暂不新增 | manual | Revivify |
| 睡眠 HP 池 | 无 | 暂不新增 | 2014 manual；2024 用 `Save` + `ApplyCondition`（已有数据） | Sleep |

**结论**：v1.6 不新增 `Effect` 变体。能用现有 14 种表达的就结构化；不能表达的明确 `manual`，不假装自动结算。这样 `Effect` 枚举、`content check`、AI 与测试的改动面可控。

### 5.7 2014 / 2024 差异（执行层）

| 主题 | 2014 | 2024 | 引擎是否分支 |
|------|------|------|--------------|
| 施法动作名 | action | Magic action | 否，映射到 `Activation::Action` |
| Cure Wounds | `1d8+mod` | `2d8+mod` | 否，数据 |
| Sleep | HP 池 5d8，无豁免 | 5 尺球，WIS 豁免/失能 | 否，数据 |
| 职业模式 | 部分 known | 全部 prepared/spellbook | 否，`classes-*.yaml` |
| 准备数量 | 公式 | 表 | 否，`classes-*.yaml` |
| 仪式资格 | 按 2014 职业 | 按 2024 职业 | 否，数据 |
| 材料/成分 | 逐法术 | 逐法术 | 否，数据 |
| 升环骰表 | 逐法术 | 逐法术 | 否，数据 |

### 5.8 什么时候需要新 effect

只有当“现有 14 种 + manual”无法表达且该类别在 v1.6 验收范围内时，才考虑新 effect。建议的后续候选（不在 v1.6）：

- 持久区域/墙（需要区域生命周期）；
- 召唤/变形（需要 creature template 与替换 statblock）；
- 传送（需要空间目标与合法性）；
- 反制/解除（需要 `on_spell_cast` 反应触发器）；
- HP 池睡眠（2014）。

这些都要单独任务、单独验收，不能塞进“施法核心”。

## 6. 任务拆分

### 6.1 总览

| 任务 | 单一目标 | 依赖 | 可并行 | 主要路径 |
|------|----------|------|--------|----------|
| T1 | 法术元数据 `SpellMeta` + SRD 导入 + content check | 无 | 无（T2/T4 的前置） | `tools/combat-sim/src/content/schema.rs`、`srd.rs`、`srd_pack.rs`、`content/check.rs`、测试、`tools/combat-sim/docs/COMBAT-SPELLS.md` |
| T2 | `talkdnd` 列表模型、存档块、schema 10 迁移 | T1 | 与 T4 可并行实现，合并串行 | `tools/talkdnd/src/{classdata,chargen,charsheet,migrate,progress,data}.rs`、`tools/talkdnd/data/classes-*.yaml`、测试、`LetsPlay/docs/{chargen,character-data}.md` |
| T3 | 列表 CLI、角色卡 refs、建卡/升级接入 | T2 | 与 T4 可并行实现，合并串行 | `tools/talkdnd/src/{lib,charsheet,chargen,progress,classdata}.rs`、测试 |
| T4 | `combat-sim` 施法结算：槽位/升环/成分/仪式边界 | T1 | 与 T2/T3 代码并行 | `tools/combat-sim/src/{engine,cli}.rs`、`tests/combat_spells.rs` |
| T5 | 常见法术数据、DM/玩家文档、章程 | T1、T3、T4 | 数据编写可在 T4 前开始，合并最后 | `LetsPlay/Rules/combat/{2014,2024}/spells.yaml`、`LetsPlay/docs/spellcasting.md`、`LetsPlay/AGENTS.md`、`tools/combat-sim/docs/COMBAT-SPELLS.md`、`tests/real_content.rs`、`docs/PROJECT-CHARTER.md` |

> **并行口径**：T2/T3 都改 `talkdnd`，必须串行；T4 只改 `combat-sim`，可与 T2/T3 的代码工作并行。所有改变用户可见行为的任务都要更新 `docs/PROJECT-CHARTER.md`；同一文件不能并行合并（见 6.3）。

### 6.2 任务详情

#### T1 — 法术元数据与 SRD 导入

**目标**：给 `Ability` 增加可选 `spell: SpellMeta`，让 SRD 导入和 `content check` 能读写法术元数据；不改变任何执行结果。

**依赖**：无。**可并行**：无；T1 是 T2/T4 的前置。

**允许路径**：

- `tools/combat-sim/src/content/schema.rs`
- `tools/combat-sim/src/srd.rs`
- `tools/combat-sim/src/srd_pack.rs`
- `tools/combat-sim/src/content/check.rs`
- `tools/combat-sim/tests/srd.rs`
- `tools/combat-sim/tests/srd_pack.rs`
- `tools/combat-sim/tests/content.rs`
- `tools/combat-sim/docs/COMBAT-SPELLS.md`（新建）

**禁止**：引擎执行路径（`engine.rs`/`cli.rs`）、规则数据、`talkdnd`、存档 schema、Cargo/CI。

**验收标准**：

1. `Ability` 增加 `spell: Option<SpellMeta>`，所有字段 serde default + `skip_serializing_if`；没有 `spell` 的旧 YAML/pack 按原样加载，序列化不新增空字段。
2. `srd.rs::spell` 在输入含 `components`/`ritual`/`higher_level`/`school`/`level`/`casting_time`/`range`/`concentration` 时填充 `spell`；缺失字段保持 `None`/默认，不 panic。
3. `srd_pack::import_spells` 输出仍为 `abilities`；`Ability` 的旧字段与 `manual` 不变。
4. `content check` 对 `spell` 做 2.5 列出的检查；旧内容 0 error；新字段不一致时 error/warning 符合定义。
5. 不改变 `content_hash` 之外的行为；旧 pack 仍加载；`spell: None` 时 `content show` 不报错。
6. 测试覆盖：完整法术、缺字段法术、旧 pack、tag/字段不一致、升环表 key 校验。
7. `cargo test --manifest-path tools/combat-sim/Cargo.toml` 通过；两版 `content check` 0 error。

**证据**：`git diff`、`cargo test` 输出、`combat-sim content check` 两版退出码、测试名。

**风险/回退**：

- `deny_unknown_fields` 使旧二进制拒绝新 pack；这是失败关闭，需在发布说明写清“pack 与二进制同版本”。回退：revert schema/导入提交；旧 pack 不变。
- 上游字段缺失导致大量 `spell: None`；`content check` 只 warning，不阻塞。回退：数据可后续补，不需要回退代码。
- 重新生成 tracked SRD pack 会产生大 diff；如果本机没有上游 5e-database JSON，T1 只交 schema/导入/测试，pack 重新生成留给有数据的环境（在 PR 证据里注明）。

#### T2 — `talkdnd` 列表模型、存档块与迁移

**目标**：在 `sheets/<id>.yaml` 增加 `spells` 块，定义 known/prepared/spellbook/cantrip/always_prepared/legacy；实现 schema 9→10 迁移与旧自由文本兼容；不新增用户命令。

**依赖**：T1（`SpellMeta` 字段与 `content check` 语义）。**可并行**：与 T4 代码并行；与 T3 串行（同 crate/文件）。

**允许路径**：

- `tools/talkdnd/src/classdata.rs`
- `tools/talkdnd/src/chargen.rs`
- `tools/talkdnd/src/charsheet.rs`
- `tools/talkdnd/src/migrate.rs`
- `tools/talkdnd/src/progress.rs`
- `tools/talkdnd/src/data.rs`
- `tools/talkdnd/data/classes-2014.yaml`
- `tools/talkdnd/data/classes-2024.yaml`
- `tools/talkdnd/tests/cli.rs`（或新增 `tests/spells.rs`）
- `LetsPlay/docs/chargen.md`
- `LetsPlay/docs/character-data.md`

**禁止**：`lib.rs` 的 CLI 路由（T3）、`combat-sim` 源码、规则数据、Cargo/CI。

**验收标准**：

1. `CharFile` 增加 `spells: Option<SpellLists>`；旧 YAML 无该字段按 `None` 加载；`store` 保留字段；`version` 策略按 D-1 执行。
2. `Creation` 增加结构化列表（或 `spell_lists`），`set spells` 仍可用；`validate` 优先结构化列表，旧自由文本回退；数量与 `classdata::generate` 一致。
3. 职业数据增加 `spells`（职业法术表）、`ritual_casting`、`book`/`scribe`、`cantrip_replace` 所需字段；8 个 SRD 施法职业覆盖 3.2 表。
4. `migrate.rs` 实现 9→10：dry-run、整档备份、逐 sheet 添加/补全 `spells`、从 `chargen.json`/正文精确 adopt、`legacy` 列表、幂等、dirty 文件不覆盖。
5. 旧存档（schema 9）迁移后 `talkdnd start`/`char` 可读；迁移失败不半写；回退备份可用。
6. 测试覆盖：旧 sheet、旧 chargen.json、无法映射的名字、重复迁移、dirty sheet、2024 新戏法识别。
7. `cargo test --manifest-path tools/talkdnd/Cargo.toml` 通过；`validate_kit`/`check-leaks`/`check-letsplay` 不受影响。

**证据**：迁移测试名与输出、`talkdnd migrate --dry-run` 记录、`git diff`、`cargo test` 输出。

**风险/回退**：

- **最高风险是数据丢失**：迁移必须整档备份；dirty sheet 不重写；迁移后旧二进制若被使用会拒绝 schema 10（预期）。回退：恢复 `Saves/_backups/...-schema9`，revert 代码。
- 职业法术表数据错误会阻止合法准备；测试用 SRD 样本覆盖，未知职业表时 warning/拒绝策略按 D-2。
- `chargen.json` 与 sheet 不一致时，以 sheet 为准、chargen 只作 adopt 输入；不覆盖已有列表。

#### T3 — 列表 CLI、角色卡 refs、建卡/升级接入

**目标**：给 DM 提供 `spells`/`prepare`/`unprepare`/`learn`/`scribe`/`ritual`/`adopt`/`sync` 命令；把列表同步为角色卡 `actions`/`features` 的法术 refs；建卡与升级时校验数量。

**依赖**：T2。**可并行**：与 T4 代码并行；与 T2 串行。

**允许路径**：

- `tools/talkdnd/src/lib.rs`
- `tools/talkdnd/src/charsheet.rs`
- `tools/talkdnd/src/chargen.rs`
- `tools/talkdnd/src/progress.rs`
- `tools/talkdnd/src/classdata.rs`
- `tools/talkdnd/tests/cli.rs`（或 `tests/spells.rs`）
- `LetsPlay/docs/chargen.md`
- `LetsPlay/docs/character-data.md`

**禁止**：`combat-sim` 源码、规则数据、存档 schema（T2 已做）、Cargo/CI。

**验收标准**：

1. `talkdnd char <名字> spells [--json]` 输出 4.3 的稳定字段/排序；未知角色/无法术职业给出明确错误。
2. `prepare`/`unprepare`/`learn`/`scribe`/`ritual` 按 3.4 与 4.3 的规则校验；幂等行为符合 4.3；失败不写文件。
3. `spells sync` 只增删/刷新法术 refs，保留非法术 refs；known/prepared/spellbook/cantrips/always_prepared 与 refs 一致；`combat-sim cast` 能对已准备法术工作，对未准备法术报 `has no ability matching`。
4. 建卡 `chargen sheet` 生成 `spells` 块与 refs；`chargen validate` 用结构化列表；`level-up` 重算数量并提示新增选择，不静默删法术。
5. `spells adopt` 只做精确匹配；未匹配进 `legacy`；不删除正文。
6. 测试覆盖：8 职业的 prepare/unprepare、超数量、不在职业表、不在书、重复执行、未准备 cast 失败、已准备 cast 成功、refs 保留。
7. `cargo test --manifest-path tools/talkdnd/Cargo.toml` 通过；`check-letsplay` 通过。

**证据**：CLI transcript、测试名、`git diff`、两版 `content check`（如果用到规则数据）。

**风险/回退**：

- `spells sync` 可能误删/误改玩家手写 refs；实现必须只处理“能识别为法术”的 refs，并在日志里列出增删。回退：`git`/存档备份；或先 `spells sync --dry-run`（建议实现）。
- `prepare` 与 `cast` 跨进程：DM 忘记 sync 时 `cast` 失败；错误信息要指向 `spells sync`。回退：revert CLI，不影响 T2 数据。
- 命令太多；按 4.1 固定命名，不新增 `talkdnd cast`。

#### T4 — `combat-sim` 施法结算：槽位/升环/成分/仪式边界

**目标**：扩展 `cast`/`use_ability`，让 `--slot` 真正扣对应槽位并按 `spell.upcast` 替换主骰；对昂贵/消耗材料做声明校验；分钟/小时法术与仪式不在战斗内结算；保持唯一执行路径。

**依赖**：T1。**可并行**：与 T2/T3 代码并行（不同 crate）。

**允许路径**：

- `tools/combat-sim/src/engine.rs`
- `tools/combat-sim/src/cli.rs`
- `tools/combat-sim/tests/combat_spells.rs`（新建）
- `tools/combat-sim/tests/engine.rs`
- `tools/combat-sim/tests/cli.rs`

**禁止**：`content/schema.rs`（T1 已完成；如缺字段先回 T1）、规则数据、`talkdnd`、Cargo/CI。

**验收标准**：

1. `cast --slot N`：校验 N ≥ 基础环；扣 `spell-slot-N`（`--slot` 显式覆盖 `Ability.cost`/`AbilityRef.cost`）；`slot_level` 上下文为 N；不改变无 `--slot` 的旧行为。
2. `spell.upcast.dice_by_level`：N 在表中时替换主伤害/治疗骰；N 不在表中且 N > 基础环时失败关闭；没有 `upcast` 时允许但记录未建模。
3. `spell.cantrip_scaling.dice_by_character_level`：按角色等级选表；无表用基础骰。
4. `spell.components.material` 有 `cost_gp` 或 `consumed` 时，缺少 `--material` → 失败；`--ignore-components` → 成功但日志 `COMPONENTS IGNORED`。
5. `spell.casting_time.unit` 为 minute/hour 时 `combat-sim cast` 拒绝（提示用 `talkdnd ritual` 或手动）；action/bonus/reaction 不变。
6. 专注/打断行为不变；新参数不影响 `begin_concentration`、`concentration_damage_check`、Ready 交互。
7. 旧 `cast`/`use`（无新参数）对无 `spell` 元数据的法术完全不变。
8. 测试覆盖：升环扣费/骰子、契约槽、cantrip scaling、材料缺失/覆盖、分钟法术拒绝、旧行为回归。
9. `cargo test --manifest-path tools/combat-sim/Cargo.toml` 通过。

**证据**：测试名、CLI transcript、两版 `content check`、`git diff`。

**风险/回退**：

- 升环替换主效果可能改变既有战斗结果；只对带 `spell.upcast` 的数据生效，旧数据行为不变。回退：revert 代码；数据可单独回退。
- `--slot` 扣费改变旧行为（旧 `--slot` 只改上下文）；这是修复，但要在 changelog/文档中说明。回退：保留旧参数语义需要 feature flag，不建议。
- 材料校验可能挡住旧桌子的“口头材料”；`--ignore-components` 是逃生门。回退：数据去掉 `cost_gp`/`consumed` 或 revert。
- 分钟/小时法术拒绝可能影响现有 `cast`；目前 `srd.rs` 把它们映射为 action，旧行为会误放行。拒绝是失败关闭，需在文档说明。

#### T5 — 常见法术数据、DM/玩家文档、章程

**目标**：把常见类别的法术数据补到可执行/明确 manual；写玩家/DM 的施法文档；更新章程与 v1.6 发布说明所需决策；不新增引擎逻辑。

**依赖**：T1、T3、T4。**可并行**：数据编写可在 T4 前开始（按 T1 schema），但合并最后。

**允许路径**：

- `LetsPlay/Rules/combat/2014/spells.yaml`
- `LetsPlay/Rules/combat/2024/spells.yaml`
- `LetsPlay/docs/spellcasting.md`（新建）
- `LetsPlay/AGENTS.md`
- `tools/combat-sim/docs/COMBAT-SPELLS.md`（T1 创建后补完）
- `tools/combat-sim/tests/real_content.rs`
- `tools/combat-sim/tests/spell_data.rs`（新建）
- `docs/PROJECT-CHARTER.md`

**禁止**：引擎/CLI 源码、`talkdnd` 源码、存档 schema、Cargo/CI、书籍正文。

**验收标准**：

1. 17/18 个手写法术（2014 为 17、2024 为 18）都有 `spell` 元数据：level/school/casting_time/components/ritual/classes/range_text/duration_text；需要升环的填 `upcast.dice_by_level`；戏法填 `cantrip_scaling`。
2. 覆盖矩阵中的类别至少各有一个结构化样本：attack、save half、auto damage、heal、condition、forced move；其余明确 `note` + `manual`。
3. 2014/2024 差异用数据表达（Cure Wounds 骰、Sleep 机制、职业模式）；没有引擎 edition 分支。
4. `content check` 两版 0 error；`real_content`/`spell_data` 测试通过；`check-leaks`、`check-letsplay` 通过。
5. `LetsPlay/docs/spellcasting.md` 写清 DM 流程、命令、错误、限制；`LetsPlay/AGENTS.md` 增加一行指引（仍 ≤150 行）。
6. `docs/PROJECT-CHARTER.md` 的 Decision Log 与受影响章节记录 v1.6 的施法决定（schema、职业范围、材料、升环、仪式、旧数据兼容），并注明对应实现任务；不声称未实现部分已交付。
7. 不把未实现能力写成已交付；不写书籍正文/存档/秘密。

**证据**：`git diff`、两版 `content check`、测试输出、`check-leaks`/`check-letsplay`、章程 diff。

**风险/回退**：

- 数据错误会直接改变战斗结果；每个法术用 SRD 文本核对，测试用固定 seed 断言。回退：revert 数据文件；schema 不变。
- 18 个法术的 `classes` 可能不完整；`content check` warning，准备命令按 D-2 处理。
- 文档与数据不一致；T5 必须同时改文档与数据，PR 里逐条对应。

### 6.3 顺序与并行

推荐顺序：

```text
T1 ──┬─> T2 ──> T3 ──┐
     └─> T4 ──────────┴─> T5
```

- **T1 必须先做**：T2/T4 都依赖 `SpellMeta` 字段与校验语义。
- **T2 与 T4 的代码可并行**：T2 在 `talkdnd`，T4 在 `combat-sim`；文件不重叠。
- **T3 依赖 T2**：同 crate、同文件（`lib.rs`/`charsheet.rs`），不能并行。
- **T5 最后**：依赖 T1/T3/T4 的行为与数据；数据编写可提前，但 `content check` 需要 T1/T4。
- **合并串行**：T2/T3/T4/T5 若都更新 `docs/PROJECT-CHARTER.md`，同一路径锁冲突，不能并行合并。可以并行“实现 + 测试”，但合并顺序必须串行；或者所有者授权把章程更新集中到 T5（见 D-10）。这不改变代码并行的事实，只是合并队列。

### 6.4 合并门与证据

每个任务按 `kit/orchestration.md` G1–G6 审查：

- **G1**：任务内 AC 逐条有证据；设计文档不是实现证据。
- **G2**：只改允许路径；T2/T3 的 `LetsPlay/docs` 改动必须与行为对应。
- **G3**：基于最新 `main`；T2 的 schema 迁移与 T3 的 CLI 不能跨任务“先合并后补”。
- **G4**：`cargo test` 对应 crate；T5 还要两版 `content check`。
- **G5**：测试/命令/退出码绑定当前 head SHA；迁移测试要证明备份与幂等。
- **G6**：schema 迁移、材料/成分行为、旧数据兼容属于风险项；未决的 D-1/D-3/D-6 不能进入 `READY`。

## 7. 待所有者决定

以下条目**不由本设计拍板**。未决时，相关实现任务不得进入 `READY`；所有者决定后写入 `docs/PROJECT-CHARTER.md` 的 Decision Log，并在受影响章节同步。

| ID | 问题 | 选项 | 推荐 | 影响 |
|----|------|------|------|------|
| D-1 | `save-meta` 保持 9 还是升 10？ | A：9 + additive + `CharFile.version` 2；B：10 + 迁移 | **B**（10 + 整档备份迁移） | T2；旧二进制兼容；发布说明 |
| D-2 | 职业法术表范围：只 8 个 SRD 基础职业，还是含子职/领域/完整书职业？职业表放哪？ | A：`classes-*.yaml` 的 `spells` 为唯一来源，`spell.classes` 仅派生；B：每个法术的 `classes` 为唯一来源；C：新 `class-spells.yaml` | **A**，8 个基础职业 + 子职 `always_prepared`；完整书职业表 v1.6 不承诺 | T2/T5；准备校验的严格度 |
| D-3 | 材料跟踪深度：只声明、库存自动检查、还是手部/器材模型？ | A：声明 + DM 裁定；B：读 `sheets` 物品栏自动检查；C：完整手部/器材 | **A**（v1.6 只声明；B/C 另开任务） | T4/T5；误判与桌规 |
| D-4 | 成分不满足时硬失败还是 warning？谁可越权？ | A：昂贵/消耗硬失败，V/S warning；B：全部 warning；C：全部硬失败 | **A**；`--ignore-components` 仅 DM 且写日志 | T4 |
| D-5 | 战斗外施法：手动 `talkdnd char`，还是给 `combat-sim` 临时 state？ | A：手动（v1.6）；B：临时 state 走 `use_ability`；C：`talkdnd` 内结算 | **A**；B 另开任务；**绝不 C** | T3/T5 |
| D-6 | 升环范围：只扣槽/上下文，还是逐级骰表？戏法成长是否同版？ | A：槽位 + `upcast` 骰表 + 戏法表；B：只扣槽；C：全部 manual | **A**（主伤害/治疗；其他 manual） | T4/T5 |
| D-7 | 仪式：哪些职业有资格？是否允许战斗内？ | A：数据驱动 `ritual_casting`，战斗外；B：仅战斗外但按职业名；C：战斗内也允许 | **A** | T2/T5 |
| D-8 | 准备/替换是否强制“刚长休”？ | A：工具提示 + 日志，不强制；B：强制最近长休；C：完全不管 | **A**（v1.6 不强制；靠日志与桌规） | T2/T3 |
| D-9 | 旧自由文本 adopt：自动精确匹配，还是全手动？ | A：精确 id/slug/name + `legacy`；B：模糊匹配；C：全手动 | **A**；不做模糊匹配 | T2 |
| D-10 | 章程更新：每个行为 PR 各自更新，还是集中到 T5？并行合并如何排队？ | A：各自更新、串行合并；B：所有者授权集中 T5 | **A**（符合 LIVING CHARTER）；若要并行合并选 B 并明确授权 | 所有任务；路径锁/合并队列 |

## 8. 待探测 / 未验证假设

| ID | 假设/未知 | 为什么重要 | 怎么验证 | 影响 |
|----|-----------|------------|----------|------|
| U-1 | 上游 `5e-SRD-Spells.json` 是否含 `classes` 数组 | 决定职业表能否从 SRD 导入，而不是手写 | 用 `5e-database` 源 JSON 检查 `spells[].classes`；当前 `srd.rs::spell` 不读取 | T1/T2 数据来源 |
| U-2 | 旧角色卡是否已有法术 refs，或 `chargen` 是否在别处写 refs | 决定“当前 cast 是否真的对新建 PC 可用” | 用真实存档 `sheets/*.yaml` 检查 `actions`/`features`；代码路径见 1.3 | 现状结论、T3 sync 策略 |
| U-3 | 2014/2024 抄书时间/金币、各职业仪式资格、戏法替换时机 | 数据不能猜 | 逐条对照 SRD 职业/法术文本；写入 `classes-*.yaml`/`rules.spellcasting` | T2/T5 |
| U-4 | `content check` 对新增 `spell` 字段的 warning/error 容忍度 | 旧内容是否会被误报 | T1 后用两版 `content check` 与旧 pack 跑 | T1/T5 |
| U-5 | FULL 包是否包含职业法术表/法术 `classes` | 决定 FULL 存档能否严格校验准备 | 用本机 full pack 的 `classes`/`abilities` 检查；仓库没有 full pack | D-2/T2 |
| U-6 | `chargen.json` 在确认后是否保留、是否与 sheet 一致 | 决定 migrate 能否用它 adopt | 用真实存档检查；迁移测试用合成 fixture | T2 |
| U-7 | `talkdnd` 能否在非战斗命令中加载 `ContentRegistry`（只读、无 state） | 决定职业表/法术元数据是否复用引擎内容而不是复制 | 用 `ContentSources { libraries: [Rules/combat/<edition>] }.load()` 的库调用验证；不联网 | T2/T3 |

## 9. 参考

- 协作契约：[../AGENTS.md](../AGENTS.md)
- 项目配置：[PROJECT.md](PROJECT.md)
- 设计目标：[PLAN.md](PLAN.md)
- 项目章程：[PROJECT-CHARTER.md](PROJECT-CHARTER.md)
- 版本规则：[versioning.md](versioning.md)
- 变更日志：[../CHANGELOG.md](../CHANGELOG.md)
- 引擎施法：[../tools/combat-sim/src/engine.rs](../tools/combat-sim/src/engine.rs)
- 引擎 CLI：[../tools/combat-sim/src/cli.rs](../tools/combat-sim/src/cli.rs)
- 内容 schema：[../tools/combat-sim/src/content/schema.rs](../tools/combat-sim/src/content/schema.rs)
- 内容加载：[../tools/combat-sim/src/content/loader.rs](../tools/combat-sim/src/content/loader.rs)
- 状态与能力查找：[../tools/combat-sim/src/state.rs](../tools/combat-sim/src/state.rs)
- SRD 转换：[../tools/combat-sim/src/srd.rs](../tools/combat-sim/src/srd.rs)
- SRD pack 导入：[../tools/combat-sim/src/srd_pack.rs](../tools/combat-sim/src/srd_pack.rs)
- 现有 Ready 设计（专注/预付参考）：[../tools/combat-sim/docs/COMBAT-READY.md](../tools/combat-sim/docs/COMBAT-READY.md)
- `talkdnd` 建卡：[../tools/talkdnd/src/chargen.rs](../tools/talkdnd/src/chargen.rs)
- `talkdnd` 职业数据：[../tools/talkdnd/src/classdata.rs](../tools/talkdnd/src/classdata.rs)
- `talkdnd` 角色卡：[../tools/talkdnd/src/charsheet.rs](../tools/talkdnd/src/charsheet.rs)
- `talkdnd` 升级重算：[../tools/talkdnd/src/progress.rs](../tools/talkdnd/src/progress.rs)
- `talkdnd` 存档迁移：[../tools/talkdnd/src/migrate.rs](../tools/talkdnd/src/migrate.rs)
- 2014 法术数据：[../LetsPlay/Rules/combat/2014/spells.yaml](../LetsPlay/Rules/combat/2014/spells.yaml)
- 2024 法术数据：[../LetsPlay/Rules/combat/2024/spells.yaml](../LetsPlay/Rules/combat/2024/spells.yaml)
- 2014 职业/法术 SRD pack：[../LetsPlay/Rules/combat/2014/srd/srd-5.1-classes.pack.json](../LetsPlay/Rules/combat/2014/srd/srd-5.1-classes.pack.json)、[../LetsPlay/Rules/combat/2014/srd/srd-5.1-spells.pack.json](../LetsPlay/Rules/combat/2014/srd/srd-5.1-spells.pack.json)
- 2024 职业/法术 SRD pack：[../LetsPlay/Rules/combat/2024/srd/srd-5.2.1-classes.pack.json](../LetsPlay/Rules/combat/2024/srd/srd-5.2.1-classes.pack.json)、[../LetsPlay/Rules/combat/2024/srd/srd-5.2.1-spells.pack.json](../LetsPlay/Rules/combat/2024/srd/srd-5.2.1-spells.pack.json)
- DM 战斗参考：[../LetsPlay/docs/combat.md](../LetsPlay/docs/combat.md)
- 角色数据参考：[../LetsPlay/docs/character-data.md](../LetsPlay/docs/character-data.md)
- 建卡参考：[../LetsPlay/docs/chargen.md](../LetsPlay/docs/chargen.md)
- 存档布局：[../LetsPlay/docs/layout.md](../LetsPlay/docs/layout.md)
