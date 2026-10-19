# combat-sim（v2）

无界面的 D&D 5e（2024 规则）战斗引擎，供 TalkDND 文字团使用。

v2 的核心理念：**引擎只懂通用 5e 规则，所有战役内容都是数据**。代码里没有法术名、
怪物名、职业能力名、状态名或战役地名；这些东西全部放在战役目录的 YAML/JSON/Markdown
数据块里，由 `--campaign` 清单加载。同一套引擎可以跑任意战役，也可以在引擎升级后
继续读取旧存档。

- 引擎：d20 检定、攻击/AC、豁免、伤害类型与抗性、临时 HP、重击、先攻、回合与
  动作/附赠动作/反应、移动、死亡豁免、借机攻击、掩体、光照、专注、基础动作。
- 数据：能力/法术/怪物动作由通用效果原语组合；状态和武器专精也是数据；AI 行为
  模板、场景和遭遇同样在内容里。
- 无法自动化的部分：内容作者写 `manual:` 或 `note` 效果，运行时打印 `MANUAL: ...`，
  由 DM 用通用命令裁定。

## 构建与检查

```bash
cargo build --release          # target/release/combat-sim
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt -- --check
```

## 内容来源

内容按以下顺序加载，后加载的同 id 条目覆盖先前的：

1. `--lib DIR`：通用库（可重复），如 `Rules/combat`
2. `--adventure DIR`：冒险内容（可重复）
3. `--chars PATH`：角色卡文件或目录（可重复）
4. `--campaign FILE`：战役清单；也可用环境变量 `COMBAT_SIM_CAMPAIGN`

清单示例（TalkDND 存档 `Saves/2026-10-19-01/combat/campaign.yaml`）：

```yaml
name: TalkDND save 2026-10-19-01
library: [../../../Rules/combat]
adventures: [../../../Adventures/keys-from-the-golden-vault/combat]
characters: [../Characters, ../NPCs]
state_dir: .
```

路径相对于清单文件解析。支持 `.yaml`、`.yml`、`.json`，以及 Markdown 中的
` ```combat yaml ` / ` ```combat json ` 代码块（角色卡就是这种形式）。

TalkDND 的典型布局：

```text
Rules/combat/                 通用库：规则接线、状态、专精、法术、职业能力、怪物、AI 模板
Adventures/<剧本>/combat/     冒险内容：场景、特殊生物、遭遇、机关
Saves/<存档>/Characters/      玩家角色卡（Markdown + combat 数据块）
Saves/<存档>/NPCs/            队友/重要 NPC 角色卡
Saves/<存档>/combat/          清单与战斗状态文件
```

内容 id 统一为小写连字符字符串（如 `v12-satyr-and-guards`），引用一律按 id。

## 数据词汇速查

**效果（effects）**：`attack`、`save`、`damage`、`heal`、`apply_condition`、
`remove_condition`、`forced_move`、`use`、`activate`、`check`、`basic_action`、
`choice`、`note`、`toggle_trigger`。

**修正（modifiers）**：`applies` 为 `attack_roll`、`incoming_attack`、`save`、
`check`、`ac`、`damage_roll`、`damage_taken`、`speed`、`healing_done`、
`healing_received` 之一；`effect` 为 `advantage`、`disadvantage`、`bonus`、
`auto_fail`、`auto_crit`、`resistance`、`immunity`、`vulnerability` 之一。
可带 `filter`、`when` 谓词、`once_per`、`consume`。

**谓词（predicates）**：`all`/`any`/`not` 组合，叶节点如 `attack_has_property`
（列表表示“任意一个”）、`attack_kind`、`target_has_condition`、
`self_hp_below_pct`、`ally_within_5_of_target`、`would_miss_with_ac_bonus` 等。

**触发（triggers）**：`on_hit`、`on_miss`、`on_hit_taken`、`on_damage_taken`、
`start_of_turn`、`end_of_turn`、`on_enter_area`、`on_interact`、`on_heal_cast`；
可设 `reaction: true`、`once_per`、`when`、`cost`。

**资源（resources）**：生物上的命名池，例如法术位、每短休/长休恢复的次数。
`recharge` 支持 `turn`、`round`、`short`、`long`、`never`、`recharge 5-6`。

**状态与专精是数据**：状态用 `flags` + `modifiers` 描述；引擎只认识通用 flag
（`incapacitated`、`speed_zero`、`no_reactions`、`no_opportunity_attacks`、
`no_opportunity_attacks_against`、`stand_up_half_speed`、`unaware`）。武器专精就是
带触发器的数据条目。

**场景**：`mode: zones` 用区域图（区域、边距离、困难地形、攀爬、锁门、物件、
触发器、光照、掩体、高度）；`mode: grid` 用 5 尺格子（墙、困难地形、掩体、高度、
区域标签）。两种模式共享一个 `Space` 接口，所以攻击距离/掩体/移动/范围效果不必
区分实现细节。

**AI 模板**：`ai_templates` 定义可选参数，生物用 `ai: {template: ..., params: {...}}`
引用，`params` 按字段覆盖模板。固定参数包括：`targeting`（nearest/lowest_hp/
highest_damage/random）、`range_pref`（melee/ranged/any）、`keep_distance`、
`self_heal_below_pct`、`heal_ally_below_pct`、`heal_downed`、`resource_threshold`、
`flee_below_pct`、`flee_zone`、`guard_zone`、`leash`、`attack_downed`、`opening`、
`avoid_targets_with`。AI 只按能力的 `ai.role`（damage/heal/self_heal/buff/
control/defense/none）和实际效果决策，不按名字。

### 添加一个新战役（最小示例）

`my-campaign/combat/campaign.yaml`：

```yaml
name: My Campaign
library: [../../Rules/combat]
adventures: [.]
characters: [./characters]
state_dir: .
```

`my-campaign/combat/creatures.yaml`（一个生物）：

```yaml
creatures:
  - id: training-dummy
    name: Training Dummy
    kind: monster
    side: enemy
    ac: 12
    hp: 20
    speed: {walk: 30}
    attacks:
      - {id: padded-strike, name: Padded Strike, kind: melee, ability: str,
         to_hit: 3, damage: [{dice: "1d6+1", type: bludgeoning}], reach: 5}
    ai: {template: brute}
```

`my-campaign/combat/scenes.yaml`（一个区域场景）：

```yaml
scenes:
  - id: training-hall
    name: Training Hall
    mode: zones
    zones:
      - {id: entrance}
      - {id: mat, cover: half}
    edges:
      - {a: entrance, b: mat, distance: 10}
```

`my-campaign/combat/encounters.yaml`：

```yaml
encounters:
  - id: first-drill
    scene: training-hall
    party_zone: entrance
    members:
      - {creature: training-dummy, zone: mat}
```

然后：

```bash
combat-sim --campaign my-campaign/combat/campaign.yaml content check
combat-sim --campaign my-campaign/combat/campaign.yaml \
  new --encounter first-drill --party my-hero --state first-drill
```

## 状态文件

`new` 会把这次战斗需要的内容快照（生物、能力、状态、专精、模板、场景）和战斗
单位一起写进状态文件。之后的 `init/attack/cast/move/...` 只需要 `--state`，不需要
`--campaign`；状态文件可以独立传递和存档。

`--state NAME` 不含 `/` 时：

- `new` 解析到清单 `state_dir/NAME.state.json`；
- 其他命令若带 `--campaign`/环境清单，优先用其 `state_dir`，否则用
  `./NAME.state.json`；
- 含 `/` 的路径按字面使用。

## CLI 速查

```bash
# 建战斗、掷先攻
combat-sim --campaign campaign.yaml new \
  --encounter v12-satyr-and-guards --party will,rina,milo,sera --state v12
combat-sim --campaign campaign.yaml --state v12 init

# 查看
combat-sim --campaign campaign.yaml --state v12 status [--json]
combat-sim --campaign campaign.yaml --state v12 scene
combat-sim --state v12 log --tail 30

# 攻击 / 施法 / 使用能力
combat-sim --state v12 attack "Will" "Guard 1" longsword
combat-sim --state v12 attack "Guard 1" "Will" spear --adv
combat-sim --state v12 cast "Milo" magic-missile "Guard 1"
combat-sim --state v12 use "Rina" cunning-action --option 2

# 移动（区域或格子）
combat-sim --state v12 move "Will" center
combat-sim --state v12 move "Will" --to 3,2
combat-sim --state v12 move "Will" center --no-oa

# 交互、状态、反应等
combat-sim --state v12 interact "Milo" console --option 1
combat-sim --state v12 condition add "Rina" hidden --rounds 2
combat-sim --state v12 condition remove "Rina" hidden
combat-sim --state v12 damage "Guard 1" 2d6 --type fire
combat-sim --state v12 heal "Will" 1d10+1 --from "Sera"
combat-sim --state v12 save "Guard 1" dex 13
combat-sim --state v12 check "Milo" arcana 10
combat-sim --state v12 next
combat-sim --state v12 rest long all

# 内容检查
combat-sim --campaign campaign.yaml content list --kind creature
combat-sim --campaign campaign.yaml content show guard
combat-sim --campaign campaign.yaml content check
```

`attack --multi` 按生物的 `multiattack` 列表依次结算。`cast`/`use` 的目标写法与
数据中的 `target` 一致；`--option N` 用于 `choice` 效果。`next` 会重置动作/附赠
动作/每轮反应并结算回合结束/开始效果、死亡豁免和资源恢复。

## 模拟与 Monte Carlo

```bash
# 一场完整回放 + 摘要
combat-sim --campaign campaign.yaml simulate \
  --encounter v12-satyr-and-guards --party will,rina,milo,sera --runs 1

# 从已有状态继续模拟（沿用其内容、先攻与回合）
combat-sim simulate --from-state v12 --runs 1

# 批量模拟；--json 输出机器可读报告
combat-sim --campaign campaign.yaml simulate \
  --encounter v12-satyr-and-guards --party will,rina,milo,sera \
  --runs 10000 --threads 8 --max-rounds 100 --json
```

`--encounter` 会像 `new` 一样构建战斗但不写状态文件；`--from-state NAME` 使用现有
状态；两者都不给时，`--state NAME` 指定的状态也可作为模拟起点（三者互斥）。

- `runs=1`：打印完整引擎/AI 日志，再打印胜者或超时、回合数、双方 HP 和每个单位
  的 HP/倒地/死亡状态。`--json` 输出 `{log, summary}`。
- `runs>1`：rayon 多线程；每个 worker 用 OS 种子（或 `--seed` 派生）初始化
  `StdRng`；每场使用 `NullSink`，热路径不做日志格式化；报告 runs、threads、
  耗时、runs/s、各阵营胜场、平局/超时/错误、平均/中位回合数、双方平均剩余 HP
  与每个单位的平均 HP/倒地率/死亡率。
- `--max-rounds` 到达后仍未分胜负记为 timeout；没有存活单位记为 draw。

## RNG

- 逐步命令：每次掷骰直接取 OS 熵（`OsRng`），不保存随机状态。
- 单场模拟：使用 OS 种子的 `StdRng`。
- Monte Carlo：每个 worker 一个 `StdRng`；`--seed N` 时按 worker 派生确定性种子，
  便于复现和测试。
- `--seed` 只用于测试/回放；正常跑团不需要。

## 空间与网格

区域场景按边距离计算；高度差会加到攻击/法术的射程距离上（同区域但不同高度也
可能够不到），移动仍按边的移动成本（困难地形 ×2）。网格场景每格 5 尺，斜向也
算一步；攻击距离 = 切比雪夫步数 × 5 + 起点/终点高度差绝对值，移动仍用逐格
Dijkstra。掩体、光照、借机攻击、范围效果和场景触发器都通过 `Space` 接口统一
处理。

`scene` 只输出文本列表（区域/边/物件/触发器/位置），不画地图。

## 内容检查与 TODO

`content check` 会验证：

- 所有 id 引用是否存在（能力、生物、状态、专精、场景、遭遇、AI 模板）；
- 表达式能否解析；
- 场景边/区域、物件、触发器、网格图例是否合法；
- `todo` 字段作为 warning 列出，提醒尚未自动化的规则。

当前真实战役内容应为 **0 errors / 12 warnings（全是 TODO）**。`manual:` 和
`note` 是预期内的人工裁定点，不是检查错误。

## 已知限制

- 仅自动化通用规则；职业特性/法术的特殊交互若数据词汇表达不了，保留为
  `manual`/`todo`，由 DM 用通用命令处理。
- 网格空间是简化版：不建模朝向、体积、三维飞行；高度只影响距离，不改变寻路
  的二维路径。
- AI 是启发式：按效果期望值、射程、资源阈值和模板参数决策，不搜索完整博弈树；
  某些复杂法术组合仍可能需要 DM 调整模板或手动接管。
- `ready` 准备动作、部分反应触发顺序、复杂召唤/变形等尚未自动结算。
- Monte Carlo 是独立同分布抽样；`--seed` 只保证同一 worker 派生流可复现，不保证
  跨机器/跨版本的逐次结果完全一致。
- 真实战役内容不在本工具仓库内；本仓库测试只使用 `tests/fixtures/` 的合成数据。
