# WEB-MIGRATION-PLAN — 独立 Web 游玩迁移计划

> 本文是 [#351](https://github.com/ctrlshiftaltdel/TalkDND/issues/351) 建议的迁移计划文档。当前只交付 **P0 首个合成 SRD 竖屏场景的现有 CLI 基线**（[#355](https://github.com/ctrlshiftaltdel/TalkDND/issues/355)）；迁移路线、存档权威细化、测试与发布章节由后续任务另加。本文只做现状取证，不做设计决定，不把任何候选当成已批准。
>
> 证据约定：**实测** = 命令或测试在记录的环境中真实执行并有退出码；**审阅** = 对当前提交的源码 / 测试逐行核对（file:line 落点）；**推演** = 基于源码同一性或文档推导的行为推断，未在本机执行。凡推演必须写明依据与缺口。
>
> 基线：本文以规划基线 `3aaf02ebf82a10c8ffaaa1c252dc72afd44785b2`（= v1.7.1 之后、#353 已合并的 main）作对照取证；证据矩阵按 PR 最新完整 head 在 Issue #355 登记。

## 1. 文档定位

- 挂靠：[#351](https://github.com/ctrlshiftaltdel/TalkDND/issues/351)（独立 Web Epic）、[docs/PLAN.md](PLAN.md)「独立 Web 游玩方向（#351）」、[docs/PROJECT-CHARTER.md](PROJECT-CHARTER.md) Decision Log 2026-11-27 条目、#183 规则总路线。
- 本章节回答 P0 需要的三件事：现有 CLI 在首个场景里**怎么做**（AC-1）、**写了哪些权威文件、玩家能看到什么**（AC-2）、**两个故障窗口下现在能判定什么、不能判定什么**（AC-3）。
- 非目标：不接入 Web、模型或外部服务；不决定认证、权限模型、前端栈、事务/事件权威、建议数量与版本；不修改任何代码、测试、CI、章程或存档 schema。

## 2. 首个场景的现有 CLI 基线（AC-1）

### 2.1 场景定义（可重跑）

**场景**：竖屏首场景「法师的飞弹」。敌方法师（施法者）对玩家角色施放魔法飞弹（SRD 法术，3 枚飞弹），玩家在任意飞弹结算前收到护盾术（Shield）反应待决提示；玩家显式 take 或 pass；确认后引擎用**原骰值**续跑；飞弹伤害与法术位在整个过程中至多结算一次。这是 [#351](https://github.com/ctrlshiftaltdel/TalkDND/issues/351) P1「刷新、重连或服务重启后恢复同一待决反应」的现有 CLI 对照物。

**规则集与存档**：

- 存档层：计划指定 v1.7.1 schema 10 合成 SRD 档（[docs/PLAN.md](PLAN.md)「P1 合成档的 schema」）。当前 main 上 `talkdnd new` 创建的新档即从 schema 10 起步：`tools/talkdnd/src/migrate.rs:949`（`CHAIN_SCHEMA = CURRENT_SCHEMA - 1 = 10`）与 `tools/talkdnd/src/migrate.rs:624-631`（新档写 `schema: CHAIN_SCHEMA`，理由注释：「schema 11 意味着每个队伍角色都有已核对的身分绑定……新档不得在没有任何可绑定时自称 11」）。审阅。
- v1.7.1 对照：`git show v1.7.1:tools/talkdnd/src/migrate.rs` 中 `CURRENT_SCHEMA: u32 = 10`（v1.7.1:43），v1.7.1 新档同为 schema 10。审阅。
- 规则集：存档 `ruleset` 取 `5e-srd` 或 `5.5e-srd`（对应 edition 机制层 2014/2024）。战斗引擎按 campaign 清单的 ruleset 加载 `LetsPlay/Rules/combat/common` + 版本层（[campaign-continuity.md](../LetsPlay/docs/campaign-continuity.md) §1.1「战役清单与战斗状态」行）。
- 战斗接线：`COMBAT_SIM_CAMPAIGN=<存档>/combat/campaign.yaml`（[LetsPlay/AGENTS.md](../LetsPlay/AGENTS.md) 桌规 12；`tools/combat-sim/src/cli.rs:73` campaign 参数回退到该环境变量；`tools/combat-sim/src/player.rs:186-203` 从 `--save` / `TALKDND_SAVE` / campaign 路径反推存档目录）。待探测：本机无 cargo 且 `LetsPlay/bin` 为 Linux 二进制，存档层端到端序列未在本机执行；下述回归在 GitHub CI 实测。

**角色与输入（回归夹具，合成数据，无版权文本）**：

- `tools/combat-sim/tests/combat_ready.rs:124-244` `content_yaml()`：合成 Hero（PC，AC 14、HP 40、法术位 2）、Source（敌怪物，爪击）、Bystander；场景 zones（alpha-beta-gamma）；遭遇 zones-fight。
- 飞弹测试 `tools/combat-sim/tests/combat_ready.rs:525-688`：Source 加法术位与 `magic-missile` 特性，Bystander 改为带 Shield 特性的第二 PC；法术定义取自真实 SRD 包（`rules.join("common")` 与 `rules.join(edition).join("spells.yaml")`，547-553），`shielded` 条件为合成注入（528-544）。
- 爪击测试 `tools/combat-sim/tests/combat_ready.rs:691-874`：完全合成夹具（Shield 特性直接写入 content_yaml 替换块，699-717），不加载 SRD 包。

### 2.2 现有命令 / 测试位置

三个既有回归覆盖场景全程（每个 `run_cli` 都是独立进程，`tools/combat-sim/tests/combat_ready.rs:345-353`；2014/2024 双版本循环）：

| 回归（combat_ready.rs） | 覆盖 |
| --- | --- |
| `magic_missile_offers_one_shield_per_target_before_resolving_any_dart`（525-688） | 多目标飞弹逐目标给 Shield；take/pass 跨进程；原飞弹冻结；资源与伤害至多一次 |
| `cli_hit_reaction_saves_the_hit_roll_and_commits_cost_and_damage_at_most_once`（691-874） | 原攻骰保存进待决状态；确认时换 seed 不能改原骰；take/pass 恰好一次结算 |
| `cli_ready_reaction_waits_across_processes_and_pass_preserves_the_ready`（404-468） | 待决跨进程等待；`--player` 视图；待决期间 `next` 被拒；pass 保留已准备的动作 |

关键实测断言（审阅 + CI 实测，见 §2.6）：

- 每个目标一枚 Shield，先答完才结算任何飞弹：第一个待决 `pending_reaction.who == "Hero"`、`reaction == "Shield"`（600-607）；take 后出现第二个待决 `who == "Bystander"`（613-616）。
- 原飞弹在待决前已掷定并冻结：`pending["pending_command"]["rolls"] == []`（617-619，飞弹伤害骰已在 cast 时掷出，只等是否 +5 AC 的判定）；待决文件字节在未决期间不变。
- 显式选择跨进程：确认用独立 CLI 调用 `reaction <id> take|pass`（609-610、627-630）；旧 ID 复用被拒且文件字节不变（623-626、641-644）。
- 资源至多一次：待决期间施法者槽位未扣（605 `slot(Source) == 2`，待决保存的是命令前状态）；最终结算的带答案重放里恰好扣一次（636 `slot(Source) == 1`）；take 时 Hero 盾槽 2→1（634），pass 的 Bystander 不扣（635 保持 2）。
- 伤害至多一次：take 后 Hero HP 保持 40（632），Bystander 15–18（633，吃掉自己的飞弹）；重取已结算 ID 失败（641-644）。
- 原骰续跑：攻骰保存进 `pending_command.rolls`（759-765），确认时传 `--seed 999` 也不能改变原骰（767-776，日志含 `1d20(<原骰>)`）；伤害从同一 RNG 流续算（777-781）。
- 无资源则不待决：法术位为 0 时攻击直接结算、无待决（792-818）；2024 同回合不能花第二个槽位（666-686）。
- 待决是兼容性标记：待决状态 `schema_version == 2`（450），旧版 combat-sim 加载即拒绝（`PENDING_SCHEMA_VERSION = 2`，`tools/combat-sim/src/reactions.rs:13`）。

**存档层场景序列**（计划的可重跑步骤，本机待探测；命令契约见 [LetsPlay/docs/combat.md](../LetsPlay/docs/combat.md)）：

1. `talkdnd new --ruleset 5e-srd --scope 0`（新档 schema 10）→ 建卡确认 → 写会话日志开场。
2. `export COMBAT_SIM_CAMPAIGN=$PWD/Saves/<存档>/combat/campaign.yaml`。
3. `combat-sim --state <存档>/combat/<遭遇>.state.json init …`（或 `quick`）建遭遇。
4. `combat-sim --state <同上> cast <施法者> magic-missile <目标…>`（或 `attack <攻方> <守方> <武器>`）→ 出现 `pending_reaction`。
5. `combat-sim --player status --json --hide-dc`（玩家投影）→ `combat-sim reaction <id> take|pass`（显式选择，独立进程）。
6. `talkdnd checkpoint` 交接清单核对（`Saves/<存档>/combat/*.state.json` 在 integrity 覆盖内）。

### 2.3 Shield 待决与显式 take/pass（不同进程）

- 引擎在需要玩家选择的窗口挂起：`ReactionReplay::ask` 无预存答案时记录 `waiting` 并返回 false（`tools/combat-sim/src/reactions.rs:100-126`）；`finish` 只保留提问前已掷的骰（128-139），整个待决 trial 丢弃、不提交（`tools/combat-sim/src/cli.rs:2890-2917`：trial 先行、确认前 `state.pending_command = Some(...)` 后保存原状态文件并打印提示）。
- 提示与确认是纯 CLI 协议：`reaction <id> take|pass`（`reactions.rs:54-59` prompt；`cli.rs:1331-1366` Reaction 分发）。待决期间其它状态写命令被拒（`require_no_pending`，`cli.rs:2841-2849`；测试 451）；待决 ID 不匹配拒绝（`cli.rs:1343`「reaction ID is no longer current」）；确认必须与原命令同一 save 上下文（`cli.rs:1352-1358`）；原始命令必须仍是遭遇写命令（`cli.rs:1361-1365`）。
- 跨进程续跑 = 新进程读盘上状态、核对 `pending_command` 逐字段相等后带原 seed/nonce/骰值重放（`cli.rs:2865-2877`：`(pending.seed, pending.nonce, choices, pending.rolls.clone())`；不相等则「pending reaction changed; read status and try again」2866-2871）。重放骰值由 `ReplayDice` 前缀回放（`reactions.rs:142-197`），分歧即「pending reaction no longer matches the recorded combat event; state unchanged」（128-131）。

### 2.4 原骰续跑

- 原骰保存在 `PendingCommand.rolls`（`reactions.rs:16-25`），每颗 `{sides, value}`（68-72）；状态 JSON 可核对（测试 759-765）。
- 确认时传入的 seed 不参与已冻结骰：ReplayDice 先回放前缀骰（`reactions.rs:156-163`），测试断言换 `--seed 999` 后日志仍是 `1d20(<原值>)`（767-776）。
- pass 与 take 都在原骰流上续算；未决窗口后的 trial 骰被截断丢弃（`reactions.rs:132-139`、173-179 注释「Nothing after the first unanswered choice is committed」）。

### 2.5 资源与伤害至多结算一次

- 法术位：待决保存命令前状态，施法者槽位在待决期间未扣（605 `slot(Source) == 2`）；确认后整条命令带原骰重放恰好一次，施法者槽位在最终结算扣一次（636 `slot(Source) == 1`）；Hero take 扣盾槽一次 2→1（634），pass 不扣（635）；已结算后重确认不再改任何数值（641-644 字节不变）。触发设置即扣、设置失败不扣、触发不再扣是准备动作的既有语义（[docs/PLAN.md](PLAN.md) 战斗最低回归第 3 条；回归 `ready_spends_the_action_and_stores_one_response_for_both_editions`，880 起）。
- 伤害：飞弹骰在待决前已掷定并冻结（617-619），take → +5 AC 全挡、HP 不变（632）；pass → 只对 pass 者按原骰结算一次（633）；每枚飞弹只结算一次（641-644 重确认拒绝）。

### 2.6 版本对照：v1.7.1 实测 / 当前 head 实测 / 推演

| 层 | v1.7.1 | 当前基线/PR head | 依据 |
| --- | --- | --- | --- |
| 反应路径源码 | 与基线字节相同 | `git diff --name-only v1.7.1 3aaf02eb… -- tools/combat-sim/src/{reactions,cli,player,state}.rs tools/combat-sim/tests/combat_ready.rs` 为空（实测 diff） | 审阅（实测的 git diff） |
| dice.rs | 差异仅为提取 `try_fill_os_random` 辅助函数，无语义变化（实测 diff） | 同左 | 审阅 |
| combat_ready 三回归在 v1.7.1 提交 | GitHub CI 实测：tag `v1.7.1` = `9c1896294575989fcb6de0e3af13bc3a8761e912`，CI run 45755769762 / 45755769576 success（含 Ubuntu/Windows/macOS `cargo test`） | — | 实测（CI） |
| 三回归在当前 head | — | 规划基线 `3aaf02eb` CI run 46024748507 success（[#355](https://github.com/ctrlshiftaltdel/TalkDND/issues/355) 依赖节）；PR head 结果见 Issue 证据矩阵 | 实测（CI） |
| 本机双版本实测 | 未执行：本机无 cargo，`LetsPlay/bin` 为 Linux 二进制不可在 Windows 运行 | 未执行（同因） | 缺口，如实登记 |
| 存档层端到端（schema 10 档 + 6 步序列） | 推演：命令契约来自文档与源码，v1.7.1 无本机执行 | 推演 | 缺口，见 §2.1 |

**等价性结论**：未发现 v1.7.1 与当前 head 在反应路径上有任何源码差异；两版本的三平台 CI 都跑过同一测试文件并通过。该等价性是「源码同一性 + CI 实测」的推演结论，不是双版本本机 A/B 实测；若 P1 需要更强的发布版对照（例如用 v1.7.1 游玩包二进制跑 §2.1 六步序列并 diff 存档），仍需补测并记录环境。

## 3. 首场景读写入口与数据权威（AC-2）

### 3.1 权威矩阵相关行（[campaign-continuity.md](../LetsPlay/docs/campaign-continuity.md) §1.1）

| 数据 | 文件 | 唯一写入者 | 场景内角色 |
| --- | --- | --- | --- |
| 战役清单 / 战斗状态 | `combat/campaign.yaml`、`combat/*.state.json` | talkdnd new（清单）；combat-sim（*.state.json） | 遭遇权威；`*.state.json` 存在时纳入 integrity |
| 队伍战斗状态 | `combat/party-state.yaml` | combat-sim 战斗命令；talkdnd char 回写 | HP/槽位/条件的持久投影 |
| 角色数据块 | `sheets/<id>.yaml` | talkdnd char 等 | 建卡与休息类写入 |
| 会话日志 | `Sessions/*.md` | 各记录命令 append | 每步一行 |
| 掷骰记录 | `rolls.log` | 掷骰命令 | 只含带骰记法的行 |
| 已识别怪物/物品 | `identified.yaml` | talkdnd identify | `--player` 遮罩输入 |
| 完整性清单 | `integrity.yaml` | 工具写入后自动更新 | 写入侧账 |

### 3.2 写入入口（遭遇 → 队伍 → 日志的更新边界）

命令分类来自共享 gate 注册表（`tools/advancement-lock/src/lib.rs:486-` WRITERS，行镜像 `docs/ADVANCEMENT-WRITER-INVENTORY.md` §2；combat-sim 侧 `tools/combat-sim/src/cli.rs:907-960` gate_entry 穷尽 match）：

- `combat-sim:state-write`（`tools/advancement-lock/src/lib.rs:1230-1237`）：targets `combat/*.state.json, combat/party-state.yaml, rolls.log, Sessions/*.md`；「state.save, party.save and log append run while the SAVE lock is held; a failed step reports which stage was written」。Shield 场景的 `cast` / `attack` / `reaction` 都属此类（`cli.rs:911-935`）。
- 实际写入顺序：`persist_state_party_log`（`tools/combat-sim/src/cli.rs:2950-2971`）：`state.save` → `write_party_state` → `append_combat_lines`，逐步停并在错误信息里报告已写阶段（三段消息逐字给出「尚未写入 party，尚未追加日志」/「已写入遭遇状态，尚未写入 party 或追加日志」/「已写入遭遇状态和 party，尚未追加日志」），注释明言「This is not a multi-file transaction」。
- 每步的原子性与完整性：`state.save` 走 `fsutil::atomic_write` 并在成功后刷新 integrity 基线（`tools/combat-sim/src/state.rs:1724-1755`，注释：「The integrity lock only serializes that manifest update. It is not content protection」）；遭遇级互斥是 `<state>.lock` 文件锁（`cli.rs:2811-2839`）。
- talkdnd 侧：`talkdnd:char`（sheets/party-state/Characters/Sessions）、`talkdnd:start`（save-meta、sheets、campaign.yaml、digest.md、.cache、Sessions）等行均 AdoptedWrite；SAVE 锁在写任何目标前获取（`tools/talkdnd/src/lib.rs:1398-1410` `hold_ordinary_save_lock` → `advancement_lock::open` + `check_ordinary_write`，锁不可用即拒写）。`rolls.log` 追加前校验 SAVE 锁（`tools/combat-sim/src/player.rs:205-225`「协议违规……未写入任何目标文件」）。
- 战斗/角色两侧对 party-state.yaml 的并发由各自 SAVE 锁持有期串行化；两侧均为「协作写者」协议（[docs/PLAN.md](PLAN.md)「写者采用同一协作协议」；`tools/advancement-lock/src/save_lock.rs:7`）。

### 3.3 读取入口（玩家投影与 DM 视图）

- combat-sim：`status` / `log` / `scene`（`tools/combat-sim/src/cli.rs:3714-3792` print_status/print_log/print_scene）；`--player` 名称遮罩、`--hide-dc` DC 遮罩；`status --json` 输出 round/turn/active/outcome/combatants/pending_reaction（`cli.rs:3799-3877`）。
- talkdnd：`start`（digest.md 开局摘要）、`known`（known.rs，叙述唯一事实源）、`knowledge search/get/cite/context/related` 五入口（`tools/talkdnd/src/knowledge.rs:87-121`，默认玩家已知过滤）、`guard`（秘密词检查）、`char <谁>` 不带子命令的状态视图、`check`（完整性审计）。
- `known` / knowledge / `--player` 是当前仅有的玩家可见性过滤面；Web 权限模型必须另建（章程明文）。

### 3.4 玩家名称遮罩、知识与公开字段：证据与缺口

证据：

- `--player` 遮罩读 `identified.yaml`（`tools/combat-sim/src/player.rs:41-50`）；未识别怪物打印外貌覆盖或「一只大型野兽」式泛称（`player.rs:61-110`、137-156）；PC/NPC 不遮（85-87）；「Wolf 1」类实例号转写为「…… #1」（69-81）；`mask_status_json` 只遮 `active` 与 `combatants[].name`（350-367）；`hide_dcs` 只遮显示层 DC（305-347），原骰与 encounter log 保留数值。
- 反应窗口玩家输出刻意最小化：`ReactionNotice` 只有 `{id, who, reaction}`；内部 `context` 字段注释明令不进玩家输出（「it can name an unidentified creature」，`reactions.rs:27-36`）。测试断言 `--player` 输出不含敌名 "Source"（combat_ready.rs:443、447）。

缺口（现 CLI 遮罩 ≠ Web 权限）：

1. `mask_status_json` 只做名称替换：`combatants[]` 其余字段（`conditions[].source/linked`、`senses` 原文、`ready`、资源名等）与 `outcome` 不在遮罩范围；文本 log 遮罩是逐行字符串替换，未识别名出现在别的字段形状时可能漏遮（`player.rs:115-134` 最长优先替换能降低但不能消除）。
2. `--player` 只作用于 status/log 的展示层；状态文件、DM 视图输出与 `dm-secrets.md`/原创 hidden 面的既有边界靠纪律（[LetsPlay/AGENTS.md](../LetsPlay/AGENTS.md) 信息纪律）与 guard 保持，不是结构化权限。
3. 章程已定为设计约束：不得把 `status --json --player` 名称遮罩当 Web 权限边界，Web 权限与玩家投影须单独定义、测试并留证据（[docs/PROJECT-CHARTER.md](PROJECT-CHARTER.md) 第 67、130 行）。
4. 知识侧：knowledge 五入口默认玩家已知过滤（`knowledge.rs:87-121`）与 `known` 台账是结构化的；但首段叙事与行动建议尚无统一投影层（#351 要求「同一份字段投影和公开事实快照」，未交付）。

### 3.5 玩家明确选择点与待决状态 / 原骰 / 资源的证据位置

| 事项 | 现状证据位置 |
| --- | --- |
| 玩家必须明确选择 | 反应窗口：`pending_reaction` 出现时状态写命令全部被拒（`cli.rs:2841-2849`）；选择只能 `reaction <id> take|pass`（`cli.rs:1331+`）；自动化边界「玩家选择与反应不得设为 auto」（[docs/PLAN.md](PLAN.md) 游玩/自动化节）。测试：待决期间 `next` 拒绝（combat_ready.rs:451） |
| 待决状态 | 状态文件 `pending_command` 字段（`reactions.rs:16-25`）；`--player status --json` 只投影 `{id, who, reaction}`（reactions.rs:38-43；测试 440-442）；文本 status 尾行 prompt（`cli.rs:3750-3752`） |
| 原骰 | `pending_command.rolls`（`{sides, value}` 列表，`reactions.rs:68-72`）；重放前缀回放（142-197）；测试 759-776 |
| 资源 | 遭遇内 `combatants[].resources`（status JSON `ResourceStatus{current,max,recharge}`，`cli.rs:3872-3877`）与 `combat/party-state.yaml`（`combat-sim/src/party.rs:21`）；两侧回写边界见 §3.2 |
| 候补窗口排队 | 多待决排队（Shield → Ready，combat_ready.rs:820-873）；nonce+序号生成 ID（`reactions.rs:105`） |

## 4. 两条合成故障轨迹（AC-3，推演）

> 本节全部是**基于源码的推演**，不是故障注入实测；「当前 CLI 已有判定边界」引用的回归断言是实测，两者分开标注。故障注入实验归 P1（[#351](https://github.com/ctrlshiftaltdel/TalkDND/issues/351) AC-3/AC-4）。权限、权威归属与幂等候选一律不写成已决定。

### 4.1 轨迹 A：遭遇已写、队伍未写

- **前像**：命令前状态文件字节 S0（含 integrity 基线）、party-state P0、rolls.log L0。
- **写入顺序**（实测依据 `cli.rs:2950-2971`）：① `state.save`（原子写 + integrity 基线刷新，`state.rs:1746-1752`）→ ② `party.save`（`cli.rs:3019-3026`）→ ③ `rolls.log` 追加（`player.rs:231-241`）。
- **注入点（推演）**：进程在 ① 完成后、② 完成前终止（kill / 掉电 / panic 路径外的硬终止）。
- **可判定事实**：状态文件已是新值 S1（原子写保证无半写文件）；integrity 基线已含 S1（写成功即刷新，`state.rs:1748-1752`）；party-state.yaml 仍 P0；rolls.log 无本命令骰行；错误消息可定位阶段（②③ 的失败分支有逐段中文错误，`cli.rs:2965-2969`）——但硬终止时消息不落盘，只能事后比对。
- **未知项**：party-state 与遭遇状态从此不一致（HP/槽位投影落后一步）；integrity 各自覆盖两文件但**不能发现这种逻辑不一致**——两文件都没有被手改，`talkdnd check` 的 Modified 报告不触发（推演；check 的对账对象是会话日志与 integrity 清单）。
- **禁止盲重试**：直接重跑同一写命令 = 新 RNG 重新掷骰（无 seed 时），会对已扣过资源的遭遇再次结算——重复扣费/伤害/日志。带 `--seed` 重跑也会把同一效果再写一遍日志与 party。除非先判定「该命令的持久化是否完成」，否则禁止重试（推演；CLI 现无该判定入口）。
- **后续恢复设计需提供的证据**：命令级标识与状态版本（#351 移动要求第 6 条 command_id/状态版本）；本命令在遭遇日志中的行（`state.log`）与 party 字段的逐项比对；integrity 清单时间线；用于判定「party 是否已写」的落盘凭据。这些当前都不存在，P0/P1 冻结项（章程 Open decisions (c)）。

### 4.2 轨迹 B：结算已完成、响应丢失

- **前像**：同上；命令为 `attack/cast`（或 `reaction take/pass` 确认）。
- **写入顺序**：同 §4.1 三步；响应 = stdout 的日志行 / 待决 prompt。
- **注入点（推演）**：三步全部完成、stdout 未到达客户端（断线/进程输出丢失）。CLI 语义下还有一个变体：确认命令 ① 写完成、客户端未收到输出即断开。
- **可判定事实**：权威文件已含最终结算（S1/P1/L1）；重读 `status --json` 与 `log` 能查回原结果（现有只读命令，实测于 CI 回归的断言依赖）；待决确认后旧 ID 重用被拒且字节不变（combat_ready.rs:465-466、641-644、787-789）；「结算已完成」在文件层是自明的——不存在需要回滚的半状态。
- **未知项**：客户端无法从「无响应」区分「结算完成」与「命令从未到达/失败」；当前 CLI 无请求级回执（无 command_id、无事件流）；`playtest::classified_error`（`cli.rs:1334-1358` Refused 分类）只服务自动游玩反馈（#322），不是可复用重试判定。
- **禁止盲重试**：任何「完成状态未知」的状态写命令不得重试；正确动作是先查权威状态再决定。待决确认在已结算后重放被拒（实测），但普通命令（attack/cast）无幂等保护（推演：重放 = 新命令新骰）。
- **后续恢复设计需提供的证据**：与轨迹 A 相同的命令标识 + 状态版本；玩家事件流（#351 SSE 事件序号补齐）；「已完成请求可查回原结果」的查询入口；无法判定提交结果时暂停并给显式恢复路径（[#351](https://github.com/ctrlshiftaltdel/TalkDND/issues/351) AC-3）。

### 4.3 待决项（不构成决定或保证）

1. 事务/事件权威：命令、领域状态、玩家事件各自的权威记录、状态版本、事件重放、备份恢复边界——所有者待决（[docs/PROJECT-CHARTER.md](PROJECT-CHARTER.md) Open decisions (c)；#351 Storage 节只列候选）。
2. 部分写入的核对与重试判定：P0 必须明确「如何核对部分写入、判定是否可重试、何时暂停并提供显式恢复路径」（#351 Storage 节原文）。本基线只证明现有 CLI 有**阶段化错误消息与写顺序**，没有**机器可判定的重试协议**。
3. 幂等候选（命令去重、nonce 复用、state 版本 CAS）：均未实现、未实测，任何候选都不等于已批准。
4. Web 权限与玩家投影：`--player` 遮罩与 knowledge 过滤只是输入（§3.4 缺口 3）。
5. 遭遇锁 `<state>.lock` 是协作性文件锁；#264 跟踪的 checkpoint 锁 WouldBlock 风险仍在独立跟踪，本单不外推其影响。

## 5. 与 Issue 五个结构化场景字段的对照

- `invariants`（玩家自决反应；原骰与已结算效果至多一次）：§2.3–2.5 的回归断言为实测证据；四规则集与隐私边界按现行契约未动。
- `negative_cases`（pass、重复确认、未知提交结果、遮罩之外字段、部分写入/响应丢失）：pass 语义实测（§2.2）；重复确认拒绝实测；未知提交结果 = §4.2 未知项；遮罩缺口 = §3.4；部分写入 = §4.1。
- `migration_cases`：本单不触发迁移。新档 schema 10（§2.1）；schema 11 身份迁移归 [#330](https://github.com/ctrlshiftaltdel/TalkDND/issues/330)，成长提交归 [#333](https://github.com/ctrlshiftaltdel/TalkDND/issues/333)。
- `integration_paths`：CLI 状态/动作 → 待决反应 take/pass → 遭遇/队伍/日志（§3.2）→ P0 权限、投影、恢复设计（§4.3 待决项）。
- `unverified_assumptions`：发布版 fixture 原样复跑（§2.6 等价性缺口）；故障注入结果可判定性（§4 未知项）；手机真机与主机文件系统证据分开记录（章程既定）。

## 6. 复现命令与执行状态

已在本会话环境（Windows 10 x64、Git Bash、uv 0.13.10、无 cargo）真跑的命令及退出码记录在 Issue #355 证据矩阵与 PR 评论。定向回归命令：

```bash
cargo test --manifest-path tools/combat-sim/Cargo.toml --test combat_ready
```

- 本机：**未执行（待探测）**——无 cargo；需 GitHub CI 或 cargo 环境。该测试文件在基线 `3aaf02eb` 的 CI（run 46024748507）与 v1.7.1 tag（9c189629，run 45755769762）均实测通过；PR head 结果以 Issue 证据矩阵为准。
- §2.1 存档层六步序列：**待探测**——需要 cargo 构建或游玩包二进制 + 合成存档（不使用真实玩家数据）。

## 7. 风险与回退

- 本单只新增本文件；回退 = 单文件 revert（`git revert` 或删除文件，无代码/schema/CI 影响）。
- 误读风险已对冲：所有行为结论都给了 file:line 落点；推演条目逐条标注；版本等价性缺口在 §2.6 明示。
- 后续设计若引用本文，必须先核对 head 是否仍与本基线一致；head 变化后本文结论需在新 head 复核。
