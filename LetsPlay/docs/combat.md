# 战斗：combat-sim 速查

> 本文件是 `AGENTS.md` 的参考章节，按需查阅。路径相对于 `LetsPlay/`。


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
- 引擎对越回合行动只**警告不阻止**；看到 `WARNING ... acting out of turn` 时确认是不是反应，否则先 `next`。
- 睡眠术（2024 第二次豁免；2014 的 HP 池整个是手动）、冒险书的特殊机关、增援到场时机等是**手动**的：用 `save`、`condition add`、`activate`。
- 状态文件是自包含的快照；改了角色卡/怪物数据后，**正在进行的**战斗不会自动更新，需要新建 state。
- 引擎结果 = 规则结算的建议，DM 仍要检查是否符合叙事与规则（例如潜行、视线、幻象遮挡是否允许借机攻击）。
- 战斗中每个掷骰同样要抄进会话日志（引擎输出已经是完整算式，可直接贴）。
- 程序文件是 `bin/combat-sim`（Windows 是 `bin\combat-sim.exe`）。没有它时按 `README.md` 放好再开局，不要改用心算。

规则数据分层：引擎按清单的 `ruleset` 加载 `Rules/combat/common/` + `Rules/combat/<版本>/`。只把 SRD 内容提交进仓库；非 SRD 数据（如只在 MM 里的怪物）放到 `Rules/combat/local/` 或加进 `.gitignore`，并在清单的 `library:` 里另列。清单模板是 `example/save/combat/campaign.yaml`。
