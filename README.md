# combat-sim

无界面的 D&D 5e（2024 规则）战斗工具，供 TalkDND 文字团跑团使用。
它把 `data/` 里的角色/怪物/遭遇数据加载成一场遭遇，逐步执行先攻、攻击、法术、
伤害、治疗、状态、死亡豁免等规则，并把完整过程写入一个可持久化的 JSON 状态文件。
另外提供一个简单的自动战斗 AI，可以跑单场逐步播放或并行蒙特卡洛模拟。

## 构建

```bash
~/.cargo/bin/cargo build --release
```

生成的可执行文件在 `target/release/combat-sim`。测试与静态检查：

```bash
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt -- --check
```

## 数据布局

```
data/party/       玩家角色（Will、Rina、Milo、Sera）
data/monsters/    怪物（guard、animated-armor、scarecrow、mimic、allosaurus 等）
data/encounters/  遭遇文件（区域 + 成员）
```

### 战斗单位 JSON 摘要

```json
{
  "name": "Will", "side": "party", "kind": "pc",
  "hp": 12, "max_hp": 12, "temp_hp": 0, "ac": 18,
  "init_bonus": 2, "speed": 30,
  "abilities": {"str":18,"dex":14,"con":14,"int":16,"wis":10,"cha":12},
  "saves": {"str":6,"con":4}, "skills": {"athletics":6},
  "zone": "front",
  "attacks": [{
    "name": "longsword", "kind": "melee",
    "to_hit": 6, "damage": "1d8+4", "damage_type": "slashing",
    "extra_damage": [{"dice":"1d8","type":"acid"}],
    "reach": 5, "range": [30,120],
    "mastery": "sap", "sneak_eligible": true, "count": 1,
    "cost": {"resource":"slot1","amount":1},
    "save": {"ability":"dex","dc":13,"half_on_success":false},
    "on_hit_conditions": [{"condition":"grappled","escape_dc":13}],
    "on_fail_conditions": [{"condition":"frightened","duration":{"until":"end_of_source_next_turn"}}],
    "vs_condition": {"condition":"grappled","advantage":true,"damage":"2d8+3"}
  }],
  "multiattack": ["slam","slam"],
  "heals": [{"name":"second_wind","amount":"1d10+1","range":0,"action":"bonus",
             "cost":{"resource":"second_wind","amount":1},"self_only":true}],
  "reactions": [{"name":"shield","ac_bonus":5,"cost":{"resource":"shield_free","amount":1}}],
  "resources": {"slot1":{"current":2,"max":2,"reset":"long"}},
  "features": {"sneak_attack":"1d6"},
  "resistances": [], "immunities": [], "vulnerabilities": [], "condition_immunities": [],
  "conditions": []
}
```

要点：

- `kind` 为 `melee` / `ranged` / `spell_attack` / `save` / `auto`。
- `range` 是 `[常规射程, 最远射程]`，单位英尺；近战用 `reach`。
- `mastery` 为 `sap` / `slow` / `nick` / `vex` / `topple` / `graze` / `push` / `cleave`。
- `count` 用于魔法飞弹等多发攻击；`cost` 用于法术位/资源消耗。
- `conditions` 可以带 `duration.rounds` 或 `duration.until`。

遭遇文件：

```json
{ "name": "party vs 2 guards",
  "zones": {"front": 20, "hall": 40},
  "members": [{"file":"../party/will.json","zone":"front"},
              {"file":"../monsters/guard.json","count":2,"zone":"hall"}] }
```

距离 = 两个区域位置之差的绝对值；同区域为 0 英尺（视为 5 英尺内）。

## 命令速查

全局参数（可放在子命令前或后）：

```bash
--state <FILE>   状态文件，默认 ./combat-state.json
--seed <N>       固定随机种子（测试/复盘用）
--json           对 status / simulate 输出机器可读 JSON
```

逐步战斗示例：

```bash
# 创建遭遇（也可以用 file.json[:count][@zone] 形式）
combat-sim new --encounter data/encounters/party-vs-2-guards.json

# 掷先攻；--set 可固定某人先攻总值
combat-sim init
combat-sim init --set Will=30

# 查看状态 / 状态 JSON
combat-sim status
combat-sim status --json

# 攻击：Will 从 front 走到 hall 再砍 Guard 1
combat-sim move Will hall
combat-sim attack Will "Guard 1" longsword
combat-sim attack Milo "Guard 2" magic_missile --adv

# 伤害 / 治疗 / 豁免 / 检定 / 泛掷
combat-sim damage "Guard 2" 3
combat-sim damage "Guard 2" 2d6+1 --type fire --crit
combat-sim heal Will 1d8+3 --from Sera
combat-sim save Will dex 13 --adv
combat-sim check Will athletics 15 --bonus 1
combat-sim roll 2d6+3 --dc 12

# 施法 / 使用资源
combat-sim cast Sera cure_wounds Will
combat-sim use Will second_wind
combat-sim use Milo slot1 --amount 1

# 状态 / 移动 / 死亡豁免 / 回合推进
combat-sim condition add Will prone --rounds 2
combat-sim condition add "Guard 1" frightened --until-end-of-turn Will --source Will
combat-sim condition remove Will prone
combat-sim move Will hall
combat-sim deathsave Will
combat-sim next
combat-sim log --tail 50
combat-sim rest short all
combat-sim rest long Will
```

自动模拟：

```bash
# 单场：打印完整逐步日志 + 胜负/剩余 HP 摘要
combat-sim simulate --encounter data/encounters/party-vs-2-guards.json --seed 42

# 蒙特卡洛：并行跑 10000 场，输出胜负率、平均回合、剩余 HP 等
combat-sim simulate --encounter data/encounters/party-vs-2-guards.json --runs 10000
combat-sim simulate --encounter data/encounters/party-vs-2-guards.json --runs 10000 --json

# 单场也可以用 --json：只输出一个 JSON 对象（runs/outcome/rounds/log/combatants）
combat-sim simulate --encounter data/encounters/party-vs-2-guards.json --runs 1 --json --seed 42

# 其他来源
combat-sim simulate data/monsters/guard.json:2 data/party/will.json
combat-sim simulate --from-state --runs 500 --max-rounds 50 --threads 4
```

## 随机数说明

- 逐步模式：每一步都使用系统熵（OsRng），不可复现。
- `simulate`：每个工作线程一个由系统熵播种的快速 CSPRNG（StdRng）。
- `--seed N`：为测试/复现使用确定性随机流；只应在需要复现时使用。
- 蒙特卡洛在 `--seed` 下按场次索引派生确定性种子，便于复现统计结果。

## 限制与待办

来自 `docs/DATA.md` 的数据问题：

- Guard 的 AC：OCR 读作 15，但按链甲衫 + 敏捷 + 盾牌计算为 16（当前用 16）。
- Scarecrow / Mimic / Allosaurus 的 OCR 文本里没有 Multiattack 行，需要在实体书中核对。
- 尚未自动化的能力：Allosaurus 的 pounce、Milo 的 Sleep、Sera 的 Bless/Sanctuary、
  Rina 的 Cunning Action、Will 的 Heroic Inspiration。
- Rina 短弓的武器精通（second mastery）在角色卡上未注明。

实现层面的限制：

- 武器精通中 Nick / Push / Cleave 目前只写一行提醒，不自动结算额外攻击或推撞。
- 没有借机攻击、掩体、擒抱挣脱、AOE 形状、地图/走位可视化。
- AI 是简单策略：按最近/最低 HP 选目标、按期望伤害选攻击、资源攻击需达到无消耗
  选项期望伤害的 1.5 倍；不会做复杂的团队协同。
- `simulate` 的单场模式默认打印完整日志；加 `--json` 时只输出一个 JSON 对象（含 `runs`、`outcome`、`rounds`、`log` 和每个战斗单位的结束状态）。
- 多场蒙特卡洛默认输出文本摘要；`--json` 时输出完整统计报告；为了性能不保留逐行日志。

## 快捷入口

`/workspace/projects/talkdnd/tools/bin/combat-sim` 是指向 `target/release/combat-sim` 的软链接；重新 `cargo build --release` 后自动生效。
