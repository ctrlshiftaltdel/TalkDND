# 战斗：combat-sim 速查

> 本文件是 `AGENTS.md` 的参考章节，按需查阅。路径都相对于游戏文件夹（解压出来的那个文件夹，名字随意）。


先设置清单（每个 shell 会话一次）：

```bash
export COMBAT_SIM_CAMPAIGN=$PWD/Saves/<存档>/combat/campaign.yaml
CS=bin/combat-sim
# Windows：CS=bin\combat-sim.exe
```

```bash
$CS content check                                   # 改了数据后必跑，必须 0 error
$CS --state v13 new --encounter v13-gemstone-guards  # 省略 --party 时用清单里的 party
$CS --state v13 init --surprised "Museum Guard 1"   # 2024：先攻劣势；2014：失去第一回合（引擎按 ruleset 自动处理）
$CS --state v13 status                              # 当前回合、HP、状态、资源
$CS --state v13 scene                               # 区域、距离、高度、可交互物
$CS --state v13 attack Will "Museum Guard 2" longsword [--adv|--dis] [--nonlethal]
$CS --state v13 cast Sera cure-wounds Sera          # 施法自动扣法术位
$CS --state v13 cast Milo sleep "Museum Guard 3"    # 新专注自动结束旧专注
$CS --state v13 move Will tables                    # 高度差自动加攀爬消耗
$CS --state v13 condition add Will carrying --permanent   # 扛人：速度减半
$CS --state v13 save "Museum Guard 3" wis 13
$CS --state v13 check Rina stealth 12
$CS --state v13 damage "Museum Guard 2" 6 --nonlethal
$CS --state v13 heal Will 1d10+1                    # 第二阵风等非法术治疗
$CS --state v13 next                                # 结束当前回合
$CS --state v13 log --tail 30
$CS simulate --encounter v13-gemstone-guards --runs 2000   # 胜率预估（只给 DM 参考）
$CS roll 1d20+7 --adv --dc 12                       # 战斗外的任意掷骰
```

**已知坑（务必注意）**

- `simulate` 只有一方时会报错；清单里没写 `party:` 时必须传 `--party`。
- 非致命：只有**近战**可以击昏；要击昏就加 `--nonlethal`（或生物数据带 `tags: [nonlethal]`）。不加就是致死。
- 越回合的 `cast` / `use`：反应能力照常执行；其他能力**直接报错**，先 `next`。准备动作或 DM 裁定时加 `--force`（会打印 WARNING）。`attack`、`move`、`interact` 越回合仍只警告（借机攻击等）。
- 休息：`rest short <名字|all> [--hd N]` 花 N 个生命骰（每个掷骰 + 体质调整值）恢复 HP；`rest long <名字|all>` 回满 HP 和资源、恢复一半生命骰（至少 1）并结束专注。`all` 只包括 party 一方活着的角色，敌人和死者不休息。
- 同伴 NPC（`kind: npc`、`side: party`）和 PC 一样在 0 HP 时倒地并做死亡豁免（`deathsave`）。
- 睡眠术（2024 第二次豁免；2014 的 HP 池整个是手动）、冒险书的特殊机关、增援到场时机等是**手动**的：用 `save`、`condition add`、`activate`。
- 状态文件是自包含的快照；改了角色卡/怪物数据后，**正在进行的**战斗不会自动更新，需要新建 state。
- 引擎结果 = 规则结算的建议，DM 仍要检查是否符合叙事与规则（例如潜行、视线、幻象遮挡是否允许借机攻击）。
- 战斗中每个掷骰同样要抄进会话日志（引擎输出已经是完整算式，可直接贴）。
- 程序文件是 `bin/combat-sim`（Windows 是 `bin\combat-sim.exe`）。没有它时按 `README.md` 放好再开局，不要改用心算。

规则数据分层：引擎按清单的 `ruleset` 加载 `Rules/combat/common/` + `Rules/combat/<版本>/`。只把 SRD 内容提交进仓库；非 SRD 数据（如只在 MM 里的怪物）放到 `Rules/combat/local/` 或加进 `.gitignore`，并在清单的 `library:` 里另列。清单模板是 `example/save/combat/campaign.yaml`。

## 队伍状态跨战斗保留（party-state）

- 文件：`Saves/<存档>/combat/party-state.yaml`，记录每个 party 成员（PC 和同伴）的当前 HP、临时 HP、资源（法术位、生命骰、限次能力）、持续状态和死亡豁免。
- `combat-sim new`（状态放在存档的 `combat/` 里时）自动读取它，所以新战斗不再满血满位；之后每条战斗命令都会把 party 的数值写回。放在别处的临时 state 不会读写它。
- 战斗外：
  - `combat-sim party show`：看当前值（`talkdnd start` 的摘要里也会显示）。
  - `combat-sim party rest short <id|all> --hd N` / `party rest long all`：战斗外休息。
  - `combat-sim party set-hp <id> N`：战斗外治疗或 DM 修正。
  - `combat-sim party sync --from-state <名字>`：手动从某场战斗写回。
- 升级或改角色卡后，maxHP 和资源上限以角色卡为准，当前值会被夹到新上限以内。

## 怪物回合

- 敌人的行动由 DM 执行：轮到敌人时，按怪物数据块用 `attack`、`cast`、`use`、`move` 下命令，然后 `next`。
- 想让引擎按 AI 模板替敌人行动，可用 `combat-sim simulate --from-state <名字> --runs 1` 看一遍推演，再照着执行（推演不改原 state）。
- 敌人做什么由 DM 根据性格和战况决定，AI 推演只是参考；玩家角色永远由玩家决定。

## 临时遭遇：`combat-sim quick`

没有现成遭遇时不用写 YAML：`combat-sim --campaign <存档>/combat/campaign.yaml --state q quick --foe goblin:4 --foe wolf:2@near --distance 40 --light dim`。5 行网格，队伍在 `near`，敌人默认在 `far`。2014 存档可直接用 `Rules/combat/2014/srd/` 里的全部 SRD 5.1 怪物和法术（`content list --kind creature`）。

## 玩家视图：未识别怪物的名称遮蔽

`status`、`log`、`simulate` 默认是 DM 视图，显示真实名字。要给玩家看同一份内容时加 `--player`：

```
$CS --campaign "$COMBAT_SIM_CAMPAIGN" --player log --tail 20
$CS --campaign "$COMBAT_SIM_CAMPAIGN" --player status
```

- 未识别的怪物显示 `appearance`（creature 数据里的字段），或 `$T identify --appearance <东西> "<描述>"` 写的覆盖值，否则用「一只大型野兽」这样的体型+类型描述。
- 识别后用 `$T identify <东西>` 记录，`identified.yaml` 里 `identified: true` 的条目恢复真名；PC 和同伴 NPC 永远显示真名。
- `identified.yaml` 默认从 `--campaign` 指向的存档（`<存档>/combat/campaign.yaml` 的上一级）读取；也可以加 `--save Saves/<存档>` 或设 `TALKDND_SAVE`。
- `combat-sim roll` 的每次掷骰也会追加到 `<存档>/rolls.log`（同上定位存档），`$T log rolls` 可查。
- 战斗状态命令（`attack`、`cast`、`use`、`save`、`check`、`damage`、`init`、`deathsave`、`rest` 等）产生的掷骰行也会自动追加到同一份 `rolls.log`；只写入含骰式（如 `1d20(`、`1d6(`）的行，条件/叙述行不写入，战斗 state 日志保持原样。

## 经验、升级与战利品

账本在 `Saves/<存档>/progress.yaml`（XP、每人钱袋与物品、公共 `party` 钱袋），每次变动都记一行到会话日志。

**升级方式固定为混合制**（没有开关）：小任务、事件给 XP；重大剧情节点或篇章结束由 DM 直接给一级（里程碑）。

- `talkdnd xp add 450 --category combat --reason "博物馆守卫"`：平分（余数报告）。类别：`combat`、`noncombat`、`quest`、`discovery`、`roleplay`、`milestone-minor`。`--reason` 必填；同一理由不会给第二次（真的发生两次加 `--again`）。
- `talkdnd xp add --difficulty hard --category noncombat --reason "与男爵谈判"`：按存档版本的表算（2014 DMG p.82 easy/medium/hard/deadly；2024 DMG 第 4 章 low/moderate/high），按每人等级相加后平分。非战斗挑战只有真有失败风险才给；小里程碑按 easy/low。
- 上限：XP 最多让角色领先卡面一级，多出的记为 `capped` 不发；越过门槛时会提示——在下次长休或本节结束时 `level-up`，或留给即将到来的里程碑。
- `talkdnd level-up <角色|all> --milestone --reason "泥沼石已被中和"`：重大剧情节点直接升级，并把 XP 提到新等级门槛；同一事件同一角色只能升一次。黄金宝库：每个劫案结束、下一个劫案等级更高时用它。
- `talkdnd xp show`：每人等级/XP/下一门槛、冒险进度轨（黄金宝库各劫案等级，标出队伍当前等级）、最近一次奖励。账本 `xp_log` 记每笔（日期、类别、理由、每人、难度、被扣上限）。
- 时机：在长休或本节结束时升级；如果在长休之外升级，当前与最大 HP 一起增加，但已用的法术位和资源不恢复（2024 DMG 第 2 章）。
- `talkdnd level-up <角色> [--hp avg|roll|<玩家掷出的数>] [--class rogue]`（XP 够时，不加 `--milestone`）：改角色卡 combat 块（等级、HP、熟练加值、生命骰、熟练豁免/技能/命中），追加 `## 升级记录`，列出新职业特性、子职与属性提升提示。正文（特性、法术）和玩家一起改；之后 `combat-sim party sync`。
- `talkdnd loot add --gp 50 "治疗药水" --to party`、`loot give --from party --to yier --gp 10 钥匙`、`loot split`（公共钱平分）、`loot spend --from yier --gp 5`、`loot show`。钱统一按铜币记，显示为 gp/sp/cp。
