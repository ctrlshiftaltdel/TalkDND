# 战斗条件：血怒、力竭、魅惑、恐惧

这页给 DM 用。规则值来自当前存档的 `ruleset`：`5e-*` 读 2014，`5.5e-*` 读 2024；战斗数据在 `Rules/combat/{common,2014,2024}/`，不要手改 `Rules/defaults/`。所有命令都在 `LetsPlay/` 里运行，`CS=bin/combat-sim`。

## 血怒（Bloodied）

- 当前 HP **小于或等于有效最大 HP 的一半**时，角色就是血怒。受伤、治疗、临时 HP 变化后立即成立，不需要额外命令。
- 2014 只作为可选状态/谓词，不自动给机械效果；2024 同样由数据决定。特性、法术或触发器在数据里写 `when: {bloodied: true}`（自己）或 `{target_bloodied: true}`（目标）。
- 临时 HP 不影响血怒判断；它只先吸收伤害。HP 为 0 时仍算血怒。
- 力竭 2014 4 级会把有效最大 HP 减半，血怒阈值按减半后的最大 HP 算。
- `$CS --state <战斗状态> status --json` 会给出 `bloodied` 和 `effective_max_hp`，需要机器可读时用它。

## 力竭（Exhaustion）

力竭是 1–6 级。等级存在战斗状态里，旧存档没有这个字段时按 0 级（无力竭）加载。

```sh
# 设置/调整到 3 级（1..6；超过本版上限或 0 会报错）
$CS --state fight.state.json condition add "角色名" exhaustion --level 3

# 旧式手动标记：不带 --level 只加标记，等级为 0，不产生机械效果
$CS --state fight.state.json condition add "角色名" exhaustion

# 查看
$CS --state fight.state.json status
$CS --state fight.state.json status --json

# 完全移除
$CS --state fight.state.json condition remove "角色名" exhaustion
```

- 2014：1 级属性检定劣势；2 级速度减半；3 级攻击与豁免劣势；4 级 HP 上限减半；5 级速度 0；6 级死亡。
- 2024：每级所有 d20 检定 -2（攻击、豁免、属性检定），速度每级 -5 尺；6 级死亡。
- 长休降 1 级（`$CS --state ... rest long "角色名"`）；2014 4 级降级后有效最大 HP 恢复，但当前 HP 不会自动补满，长休本身会按恢复后的最大值回满。
- 6 级会直接死亡，并走正常死亡清理：被该生物魅惑/恐惧的持有者会解除，它自己的专注与被它施加的专注条件会清理。死亡后不能靠长休降级复活；要改判，先与玩家确认，再用 `condition remove` 并另行处理复活/回 HP。重复设置 6 级不会重复触发死亡清理。
- 日志会写等级、当前效果和移除原因；不要把 DM 秘密写进日志。

## 魅惑（Charmed）

```sh
$CS --state fight.state.json condition add "角色名" charmed --source "施法者名"
```

- 被魅惑者不能攻击施加者，也不能用伤害性能力指定施加者；擒抱（Grapple）和推撞（Shove）也算攻击，对施加者会被拒绝。攻击其他目标、治疗施加者不受限制。
- 施加者死亡时自动清除；`condition remove` 也立即清除。
- 施加者对社交检定有优势。**引擎不结算社交检定**：由 DM 按桌上规则手动处理，并在叙事/日志里说明；不要伪造一个引擎掷骰。
- `--source` 必填才有目标限制；没有来源的旧式 `charmed` 仍可加，但只能当手动标记，不会阻止攻击。
- `--source` / `--linked` 必须指向一个现有战斗者；未知或歧义的名字会报错，不会加条件。

## 恐惧（Frightened）

```sh
$CS --state fight.state.json condition add "角色名" frightened --source "恐惧源名"
```

- 恐惧源**存活、未隐藏、且在视线内**时：攻击检定与属性检定有劣势。
- 不能主动移动到更靠近恐惧源的位置；移动命令会被拒绝并说明原因。被推、被拉等强制移动不受限。
- 恐惧源死亡、隐藏/隐形、被墙或遮挡挡住时，劣势消失、移动限制解除。视线使用引擎既有的网格/区域查询，不另算几何。
- 没有 `--source` 时保持旧行为（一直有劣势），但没有“不能靠近”的限制；要完整规则请总带 `--source`。`--source` / `--linked` 必须指向一个现有战斗者，未知或歧义的名字会报错。
- 移除：`condition remove "角色名" frightened`。

## 兼容与边界

- 条件、等级、来源都可追溯；非法等级/未知来源会失败，不会改坏状态。
- 旧 `condition add/remove` 不带新参数仍按原样工作；`condition add exhaustion` 不带 `--level` 仍是手动标记（0 级、无机械效果），要机械力竭必须显式给 `--level`。
- 战斗状态 `STATE_SCHEMA_VERSION` 仍为 1；旧状态缺新字段按默认加载，不需要 `talkdnd` 迁移。
- 条件数据在两版都通过 `content check`；规则书正文不入库、不进日志。
- 详细实现与数据字段在仓库的开发文档里；玩家只需要本页和 `bin/combat-sim`，不需要读开发代码。
