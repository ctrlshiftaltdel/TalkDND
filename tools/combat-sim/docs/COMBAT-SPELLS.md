# 法术元数据（v1.6 T1 / TASK-0122）

> 状态：T1 已实现——`Ability` 的可选 `spell` 元数据块、SRD 导入填充与 `content check` 规则。
> **这不是 v1.6 施法结算**：本文只描述元数据层。`cast`/`use` 的执行行为没有变化；升环结算、成分硬校验、契约槽位（T4）、法术列表与存档迁移（T2/T3）尚未实现。
> 设计权威是 `docs/SPELLCASTING.md` §2（本文是其 T1 子集的操作说明，不另立规则）。

## 1. 模型：`Ability` + 可选 `spell` 块

法术在 combat-sim 中仍然是普通的 `Ability`（`tools/combat-sim/src/content/schema.rs`）。法术**不**有独立类型、独立 id 空间或第二套执行器；v1.6 采用的是在 `Ability` 上加一个可选嵌套块：

```yaml
id: fireball
activation: action
cost: {resource: spell-slot-3, amount: 1}
slot_level: 3
spell:
  level: 3
  school: evocation
  casting_time: {amount: 1, unit: action}
  components: {verbal: true, somatic: true}
  ritual: false
  range_text: "150 feet"
  duration_text: Instantaneous
  upcast:
    text: "...using a higher-level slot..."
    dice_by_level: {4: 9d6, 5: 10d6}
```

要点：

- `spell: Option<SpellMeta>`，`#[serde(default, skip_serializing_if = "Option::is_none")]`。没有 `spell` 的旧 YAML、旧 pack、旧角色卡按 `None` 原样加载，序列化不出现空 `spell` 字段；执行路径（`use`/`cast`/`resolve_ability`）完全不读这个字段。
- `Ability` 的既有字段继续承担执行语义：`activation`（动作经济）、`cost`/`slot_level`（法术位成本与基础环级）、`range`（引擎距离，Touch=5、Self=0）、`concentration`、`effects`、`manual`。`spell` 块只存描述性元数据，不复制执行字段。
- 所有 `SpellMeta` 字段都有 serde 默认值；可选字段（`school`、`range_text`、`duration_text`、`upcast`、`cantrip_scaling`、`classes`）省略时不出现在序列化输出里。`components.material` 缺省时整个 `material` 键省略。
- 新结构体带 `deny_unknown_fields`：旧二进制读含 `spell` 的新 pack 会直接报错（失败关闭）。pack 与二进制同版本发布；FULL pack 由玩家本机构建。

## 2. 字段定义

| 字段 | 类型 | 含义 |
|------|------|------|
| `level` | `u8` | 0 = 戏法，1–9 = 有环法术 |
| `school` | `Option<SpellSchool>` | 八大学派（abjuration/conjuration/divination/enchantment/evocation/illusion/necromancy/transmutation）；未知学派省略 |
| `casting_time` | `CastingTime { amount: Option<u32>, unit }` | `unit` ∈ action / bonus_action / reaction / minute / hour / special；`amount` 为数字部分（`special` 无数字） |
| `components` | `SpellComponents { verbal, somatic, material }` | V/S 布尔；`material: MaterialComponent { text, cost_gp?, consumed }` |
| `ritual` | `bool` | 是否可仪式施法（资格判定属 talkdnd/版本规则，不在引擎里） |
| `range_text` | `Option<String>` | 展示用范围文本（`Touch`、`Self (60-foot cone)`、`Sight`） |
| `duration_text` | `Option<String>` | 展示用持续时间文本（`Instantaneous`、`Up to 1 minute`） |
| `upcast` | `Option<UpcastSpec>` | 升环表：`text`（展示）+ `dice_by_level: BTreeMap<u8, String>`（槽位环级 → 主伤害/治疗骰） |
| `cantrip_scaling` | `Option<CantripScaling>` | 戏法成长表：`text` + `dice_by_character_level: BTreeMap<u8, String>`（角色等级 5/11/17） |
| `classes` | `Vec<String>` | 收录该法术的职业 id 列表；空 = 未按职业收录 |

约定：

- 分钟/小时施法时间写在 `casting_time`（`unit: minute` / `hour`），而 `activation` 仍按旧规则映射（bonus/reaction，其余一律 `action`）。分钟/小时法术的 `activation: action` 只是目录值，不是战斗动作；战斗内施法是否拒绝长施法是所有者决策 D-11，T4 之前不强制。
- `upcast`/`cantrip_scaling` 的骰子表达式做了与 effect 骰一致的规范化：去空格、`MOD` 写成 `{spell_mod}`。T4 施法结算将直接按 `slot_level` / 角色等级查表替换主伤害/治疗骰。
- `dice_by_level` 的 key 应覆盖 `[level, 9]`；`cantrip_scaling` 的 key 覆盖成长等级（5/11/17）。表缺环时 T4 按失败关闭处理，不猜。

## 3. SRD 导入来源

导入链：上游 5e-database JSON → `srd_pack::import_spells` → `srd.rs::spell` → pack 的 `abilities` 列表（pack id `srd-<ver>-spells`）。

`srd.rs::spell` 在上游字段存在时填充 `spell` 块：

| 上游字段 | 写入 |
|----------|------|
| `level` | `spell.level` |
| `school.index` | `spell.school`（仅当是八大学派之一，否则省略） |
| `casting_time` | `spell.casting_time`（分钟/小时映射到对应 unit；旧 `activation` 映射不变） |
| `components` + `material` | `spell.components`（V/S 布尔；`material` 文本进 `material.text`，价格/消耗待 T5 数据任务补） |
| `ritual` | `spell.ritual` |
| `range` / `duration` | `range_text` / `duration_text`（同时保留旧 `manual` 文本与数值 `range`） |
| `higher_level` | `upcast.text`；骰表取 `damage.damage_at_slot_level`（否则 `heal_at_slot_level`） |
| `damage.damage_at_character_level`（仅 level 0） | `cantrip_scaling.dice_by_character_level` |

缺失字段保持默认/省略，导入不 panic。`import_spells` 的输出形状不变（仍是 pack 的 `abilities`；旧字段与 `manual` 不变），条目 id、排序、去重与覆盖顺序都不变。带 `spell` 的 pack 重新生成后 `content_hash` 会变化——这是预期的，不要为保持 hash 而省略字段。

**tracked pack 未重新生成**：仓库里的 `LetsPlay/Rules/combat/<edition>/srd/srd-*.pack.json` 在 T1 不重新生成（T5 数据任务处理）；本任务的元数据行为由合成 fixture 测试覆盖。手写 `spells.yaml` 覆盖层按原样加载（无 `spell` 块，`None`）；要给手写条目补元数据也留到数据任务。

## 4. `content check` 规则

`content check`（`tools/combat-sim/src/content/check.rs`）只对**带 `spell` 块**的 ability 启用法术检查；没有 `spell` 的旧内容不产生任何新问题（0 error、0 warning）。错误等级：

| 检查 | 等级 |
|------|------|
| `spell.level` 与 `slot_level` 不一致（cantrip 期望无 `slot_level` 或为 0） | **error** |
| 有环法术的 `cost.resource` ≠ `spell-slot-{level}`，或缺失 `cost`（契约/特殊资源由角色卡 `AbilityRef.cost` 运行时覆盖表达，数据定义侧不豁免） | **error** |
| `upcast.dice_by_level` 的 key < `level` 或 > 9 | **error** |
| `components.material` 有 `cost_gp` 或 `consumed` 但 `text` 为空 | **error** |
| `spell.school` 与学派 tag 不一致（有学派 tag 且与 `school` 不同） | warning |
| `spell.ritual` 与 `ritual` tag 不一致（两个方向都算） | warning |

tag 是旧兼容层，不承担语义，所以不一致只给 warning；字段与执行语义相关，不一致是 error。

## 5. 与旧内容的关系

- **旧 pack / 旧 YAML**：无 `spell` 字段 → `None`，加载、校验、执行与 v1.5 完全一致。
- **旧二进制读新 pack**：`deny_unknown_fields` 拒绝——失败关闭，发布说明需写明 pack 与二进制同版本。
- **`content show` / `content list`**：`show` 直接输出 ability JSON，带 `spell` 的条目会包含该块；`spell: None` 时不出现。
- **T4 之后**：升环/成分/契约结算只读这里的字段；T2/T3 的法术列表只依赖 `spell.level` 与内容 id。在 T4 落地前，本字段对战斗结果没有任何影响。

## 6. 测试位置

- `tools/combat-sim/tests/srd.rs`：导入映射（完整法术、分钟/小时、戏法成长、未知学派、缺失字段、旧字段不变、反序列化）。
- `tools/combat-sim/tests/srd_pack.rs`：pack 导入后 `abilities` 形状、元数据字段与加载往返。
- `tools/combat-sim/tests/content.rs`：serde 分层（旧 YAML 往返、部分字段默认值、完整往返）与 `content check` 错误/警告矩阵。
