# DM Harness 设计（候选路径）

> 本文件是 [#351](https://github.com/ctrlshiftaltdel/TalkDND/issues/351) P0 的候选设计路径。目前只包含 [#358](https://github.com/ctrlshiftaltdel/TalkDND/issues/358) 交付的「现有 CLI 自动化设置与审计」章节：它只记录现状与实测证据，不决定 UI、Harness 权限、工具协议、认证或持久化。后续章节由后续任务按需补充。

## 现有 CLI 自动化设置与审计

本节盘点现有 `talkdnd` CLI 的自动化开关（#218，TASK-0111）如何解析、拒绝与留痕，供下游 DM Harness 在已批准的玩家控制边界上接线时对照。审阅基线为 main `3aaf02ebf82a10c8ffaaa1c252dc72afd44785b2`：`talkdnd` 源码（`tools/talkdnd/src/settings.rs`、`tools/talkdnd/src/migrate.rs`、`tools/talkdnd/src/start.rs`、`tools/talkdnd/src/lib.rs`）、门注册表（`tools/advancement-lock/src/lib.rs`）、CLI 合成回归（`tools/talkdnd/tests/automation.rs`）与两份现有文档（[LetsPlay 的自动化参考](../LetsPlay/docs/automation.md)、[工具侧 AUTOMATION.md](../tools/talkdnd/docs/AUTOMATION.md)）。所有拒绝路径与隐私断言都来自现有合成回归（fixture 无真实存档、秘密或书籍文本）；本会话本机没有 cargo，cargo 类命令的执行证据取自同基线 main 的 CI run [46024748507](https://github.com/ctrlshiftaltdel/TalkDND/actions/runs/46024748507)（其中 `cargo test --manifest-path tools/talkdnd/Cargo.toml` 含 `--test automation`）。

### 六类固定设置与三个级别

自动化开关是每存档一份的设置，存放在 `Saves/<slot>/save-meta.yaml` 的 `settings.automation` 块（on-disk `version: 1`），没有第二份设置文件。它只决定"何时提示、询问或代结算"，不改变规则、骰子、战斗 AI 默认策略或存档结构语义；所有骰子仍由程序掷出并写入 `rolls.log`。

三个级别是固定枚举（`migrate.rs:226-261`）：`manual`（手动：不主动做，玩家明确要求才做）、`assisted`（辅助：主动推进但把玩家留在环里，先问/先警告）、`auto`（自动：直接做完并照常记日志）。其他取值被 `Level::parse` 拒绝。

六个固定类别、默认值与玩家自有标记（`AUTOMATION_CATEGORIES`，`migrate.rs:293-336`）：

| 类别 | 中文名 | 默认 | 玩家自有（禁 auto） | 管什么 |
|------|--------|------|--------------------|--------|
| `resolution` | 结算 | `auto` | 否 | 检定、伤害、HP/资源记账等没有玩家选择的结算 |
| `combat` | 战斗流程 | `auto` | 否 | 先攻、怪物回合、条件与移动等战斗流程 |
| `npc` | NPC 对话 | `assisted` | 否 | NPC 的台词、语气与临场反应，DM 演绎后可被玩家打断 |
| `player_choice` | 玩家选择 | `assisted` | 是 | 属于玩家角色的决定与选项，只能询问或手动，不能自动代做 |
| `reaction` | 玩家反应 | `assisted` | 是 | 护盾术、幸运、激励、借机攻击等玩家反应，只能询问或手动 |
| `near_limit` | 接近上限 | `assisted` | 否 | HP 见底、法术位/资源将尽、力竭或负重接近上限时先警告 |

默认值遵循章程自动化原则：无选择的结算自动、玩家选择先问、接近上限先警告。`player_choice` 与 `reaction` 的 `player_owned: true` 是数据模型的一部分，`auto` 在写入与加载两侧都被拒绝（见下文「拒绝与失败关闭行为」）。

来源如何展示：`settings automation show` 逐类别写 `- <name>（<中文名>）：<level> — 来源：<存档默认 | 场景 <id> | 遭遇 <id>>`（`settings.rs:151-157` 的 `Source::describe` 与 `settings.rs:252-322` 的 `automation_lines`）；无上下文时另列出存档默认的六类、全部场景/遭遇覆盖及各类别的说明注记。裸 `settings` 与 `talkdnd start` 摘要各输出一行确定性摘要（`automation_digest`，`settings.rs:231-247`）：当前场景的逐类生效值、覆盖数量与查看来源的提示。骰子显示 `settings.dice_display` 是独立的旧开关，默认开，行为保持不变（`settings.rs:33-89`）。

### 三层优先级与缺失回退（同一合成存档）

一个存档有三层：存档默认（`automation.default`）、场景覆盖（`automation.scenes`，按场景 id）、遭遇覆盖（`automation.encounters`，按遭遇 id）；覆盖只写被改的类别，其余继承上一层。生效顺序是**遭遇 > 场景 > 存档默认**（`settings.rs:179-227` 的 `resolve`；`show` 输出首行也标明该顺序）。

以下例子来自 CLI 合成回归 `precedence_is_encounter_then_scene_then_save_default`（`tests/automation.rs:165-329`）使用的同一个合成存档：`Saves/2030-02-01-01`，经由 `combat/campaign.yaml` 挂冒险 `test-adventure`（场景 `crypt-entry`、遭遇 `goblin-ambush` 且声明链接场景 `crypt-entry`，fixture 见 `tests/automation.rs:55-75`）：

1. 设置 `npc=manual`（场景 `crypt-entry`）、`combat=assisted`（遭遇 `goblin-ambush`）、`near_limit=auto`（场景 `crypt-entry`）。
2. `show --encounter goblin-ambush` 逐类给出：`combat` 取遭遇覆盖 `assisted`（来源：遭遇 goblin-ambush）；`npc` 无遭遇覆盖，回落到遭遇链接的场景覆盖 `manual`（来源：场景 crypt-entry）；`resolution` 两层都未覆盖，回落存档默认 `auto`（来源：存档默认）。回归逐行断言这三条。
3. 缺失回退（向下兼容地剥层）：`reset npc --scene crypt-entry` 删除场景覆盖后，`npc` 显示回落到存档默认 `assisted`（回归断言 `manual → 未覆盖` 与回落值）；`reset all --encounter goblin-ambush` 清空遭遇覆盖后，`combat` 回落到存档默认 `auto`。
4. 场景层的取法：展示某个遭遇时，场景层 id 取遭遇声明的链接场景；只有遭遇未声明链接时才用显式传入的 `--scene`（`settings.rs:176-185` 与 `settings.rs:279-281`）。`show` 允许同时传 `--scene` 与 `--encounter`（遭遇链接优先）；`set`/`reset` 则拒绝两者同用。
5. 存档默认缺某类别时，`default_level` 依次回退到类别章程默认（`migrate.rs:392-398`）；`fill_defaults` 在 set/reset 写入前与迁移步骤中把缺的类别补成章程默认（`migrate.rs:384-390`、`migrate.rs:1244`）。整个 `default` 映射为空则加载拒绝（`migrate.rs:449-453`）。无 `save-meta.yaml` 时 `settings::get` 回落到全默认（`settings.rs:25-31`），而 `automation_lines` 显式报错。

单元测试 `precedence_and_player_owned_levels`（`settings.rs:962-993`）以纯数据复核同一条优先链与玩家自有禁 auto 的 `check()` 拒绝。

### 拒绝与失败关闭行为

CLI 错误统一以 `error: …` 写 stderr 并退出码 1（`main.rs:1-8`）。回归 `illegal_values_and_unknown_ids_are_rejected_without_writing`（`tests/automation.rs:442-575`）对每条拒绝路径断言非零退出，且最后断言 `save-meta.yaml` 与最新会话日志逐字节不变（`tests/automation.rs:568-573`）——拒绝不落盘、不记日志。

| 输入 | 程序行为 | 证据 |
|------|----------|------|
| `set player_choice auto` / `set reaction auto` | 退出 1："`<category>` 不能设为 auto：玩家选择/反应绝不能自动代做（可用 assisted 或 manual）；存档未改动" | `settings.rs:448-453`；回归 `tests/automation.rs:477-494` |
| `set <未知类别> …` | 退出 1：列出六个合法类别名 | `settings.rs:437-446`；回归 `tests/automation.rs:449-461` |
| `set npc sometimes`（未知级别） | 退出 1："自动化级别只能是 manual、assisted、auto" | `migrate.rs:253-260`；回归 `tests/automation.rs:463-475` |
| `set … --scene no-such-scene` / `--encounter no-such-encounter`（未知作用域） | 退出 1："未知场景/遭遇 id …；已知…；存档未改动"，并列出已知 id | `settings.rs:393-425`；回归 `tests/automation.rs:495-525` |
| `--scene` 与 `--encounter` 同时给 `set`/`reset` | 退出 1："一次只覆盖一个层级" | `settings.rs:360-363`；回归 `tests/automation.rs:527-543` |
| `--reason` 为空、全空白或缺失 | 退出 1（必填、非空、≤200 字符、无控制字符） | `settings.rs:376-388`；回归 `tests/automation.rs:545-560` |
| `reset <未知类别>` | 退出 1（类别只能是六类之一或 `all`） | `settings.rs:532-541`；回归 `tests/automation.rs:561-566` |
| `set`/`reset` 值与现状相同 | "未改动、未记日志"，不写任何文件 | `settings.rs:481-486`、`settings.rs:606-618` |
| 写入失败（unix 目录只读） | meta 与日志都不变；修复后同一命令成功 | `tests/automation.rs:728-775`（`cfg(unix)`） |

作用域 id 的语法约束由 `check_id` 统一强制：1–80 个字母/数字/`-`/`_`/`.`，拒绝 `.`、`..` 与控制字符（`migrate.rs:457-467`）。

手改配置在加载时失败关闭（回归 `hand_edited_automation_fails_closed`，`tests/automation.rs:578-626`）：未知类别、未知级别、`automation` 块下未知字段、`settings` 下未知字段、重复的 YAML 键，都会让任何加载该存档的命令以非零退出拒绝——`Settings` 与 `Automation` 都带 `deny_unknown_fields`（`migrate.rs:140`、`migrate.rs:350`），`migrate::load` 每次读取都重跑 `automation.check()`（`migrate.rs:593-596`），重复键在反序列化层被拒绝而非静默后者胜（`migrate.rs:487-493`）。`automation.version` 非 1 与存档 schema 比当前二进制新同样拒绝并提示更新 talkdnd（`migrate.rs:405-410`、`migrate.rs:585-592`；回归 `tests/automation.rs:713-726`）。空覆盖映射、非法 id 字符也在 `check()` 拒绝（`migrate.rs:414-423`）。把文件改回原样后存档恢复可用（`tests/automation.rs:623-624`）。写入侧再设一道防线：任何程序化写 `save-meta.yaml` 前都重跑 `check()`（`meta_bytes`，`migrate.rs:641-649`）。

`show` 带未知 `--scene`/`--encounter` 的拒绝路径在代码中存在（`settings.rs:267-277`，与 `set` 共用同一 `check_id` + 已知 id 校验），但 CLI 回归没有覆盖 `show` 的这条分支；其真实退出行为在本节保持未知，待实测后补证。

### 元数据只读与合成隐私扫描

设置命令只从三类来源收集已知场景/遭遇 id（`settings.rs:791-831`）：`progress.yaml` 的当前场景 label（合法 id 时直接收录，`show`/`start` 摘要还会经内容名映射回 id，`settings.rs:723-731`）；存档 `combat/*.state.json`（只读 `scene.id` 与 `encounter_name` 两个字段，`settings.rs:734-747`）；存档清单 `combat/campaign.yaml` 指向的冒险 `combat/` 目录（只读 `scenes[].id/name`、`encounters[].id/scene` 与单条目文件的 `id/name/scene` 和 `members/zones/mode/light` 的存在性，`settings.rs:749-789`）。所有内容承载字段用 `IgnoredAny` 消费且从不保留为通用值树（`settings.rs:749-752`）。

这条保障由合成隐私回归 `metadata_scanning_never_leaks_state_or_adventure_content`（`tests/automation.rs:358-439`）证明：fixture 在冒险文件与合成战斗状态里埋入 `TOP-SECRET-*` 哨兵字段（场景描述、冒险日志/备注、状态场景名/备注、状态日志、DM 备注、战斗者备注），然后断言 `show`（场景与遭遇）、`set`、裸 `settings` 摘要、最新会话日志与持久化后的 `save-meta.yaml` 全部不含哨兵串。审计行只含类别、级别、作用域 id、旧值/新值与 DM 提供的 reason。

注意范围：以上是该工具对**自身输出、会话日志与持久化文件**的保障。未来 Web/Harness 玩家投影的可见性过滤是另一层，不在本节结论之内，须由后续任务单独取证（见 Issue #358 的 `unverified_assumptions`）。

### 接线：settings → save-meta → 会话日志 → start 摘要

- **读取（只读）**：`settings`、`settings dice`、`settings automation show` 经 `settings::lines` / `automation_lines` 读 `migrate::load` 并打印。门注册表把这些入口分类为 `talkdnd:settings-read`（ReadOnly，声明写目标 none；`tools/advancement-lock/src/lib.rs:924-930`）。
- **写入**：`set`/`reset` 按序做类别/级别/原因/作用域/已知 id 校验 → `migrate::ensure`（旧存档先走迁移链）→ `fill_defaults` → 构造新 meta → `automation.check()` → `migrate::write_meta` 原子写 `save-meta.yaml` → `log_line` 把 `- [HH:MM] 自动化 …（旧值 → 新值）（reason）` 追加到最新 `Sessions/*.md`（`settings.rs:427-520`、`settings.rs:522-649`、`settings.rs:651-659`；日志追加走 `save::append`，`tools/talkdnd/src/save.rs:242-280`）。
- **成长协议（SAVE 锁）**：门注册表把 `talkdnd:settings-automation-write` 分类为 `AdoptedWrite`，声明写目标 `save-meta.yaml, Sessions/*.md`，并写明"takes the SAVE lock before writing targets; fails closed and writes nothing when the lock is unavailable"（`tools/advancement-lock/src/lib.rs:915-923`）。启用成长协议的存档上，`write_meta` 在未持有 SAVE 锁时拒绝并写明"未写入任何目标文件"（`migrate.rs:655-662`），日志追加同样被 SAVE 门校验（`save.rs` 的 `require_save_for_log_append`）。没有协议文件的存档保持既有行为，不自动初始化（`settings.rs:109`、`settings.rs:127` 的命令文档）。
- **start 摘要**：`talkdnd start` 的 digest 用 `automation_digest` 写一行 `- 自动化：…`（`start.rs:592`），随整个 digest 落盘 `digest.md`（`start.rs:800`）。回归 `start_digest_shows_the_effective_automation_summary`（`tests/automation.rs:778-806`）断言 stdout 与 `digest.md` 都含当前场景的生效值（含来源）、覆盖计数与查看来源的提示。
- **迁移与回滚**：自动化块随 schema 9 引入；8→9 迁移步骤把六个章程默认显式写入、保留既有局部值、整份备份到 `Saves/_backups/`、追加迁移记录与会话日志行；备份即文档化回滚（`migrate.rs:1241-1251`、`migrate.rs:1297-1304`；回归 `migration_8_to_current_backs_up_logs_and_keeps_old_saves_equivalent`，`tests/automation.rs:629-710`，其中还断言迁移前后无自动化块的行为等价）。当前代码的 `CURRENT_SCHEMA` 是 11、迁移链落在 `CHAIN_SCHEMA`=10，新档直接从 10 起步（`migrate.rs:55`、`migrate.rs:949`）；工具侧文档 `tools/talkdnd/docs/AUTOMATION.md` 的迁移叙述仍写 "`CURRENT_SCHEMA` is 9"，数字滞后于代码（机制描述未变），引用时以代码为准。

### 程序校验的与仍靠纪律的

程序已校验（机器强制）：类别/级别枚举；`player_choice`/`reaction` 禁 `auto`（写入与加载两侧）；作用域 id 的语法与已知性；`set`/`reset` 单作用域；`--reason` 必填、非空、≤200 字符、无控制字符；`automation.version` 与存档 schema 门；重复键与未知字段拒绝；任何拒绝或失败不写 meta、不记日志；成长协议存档的 SAVE 锁纪律。

仍由 DM/调用方遵守（程序不管）：`assisted` 的执行语义——先问玩家角色、接近上限先警告、NPC 照常演绎但可被打断；`manual` 等玩家开口；`near_limit` 提醒的时机与措辞；reason 内容本身不写秘密或存档正文（程序只限长度与控制字符）；自动化值只改变"何时提示、询问或代结算"，不改变规则、骰子或战斗 AI。这些边界写在 [LetsPlay 的自动化参考](../LetsPlay/docs/automation.md)（「边界」节）与工具侧 AUTOMATION.md（"The program never rolls a player's choice or spends a player's reaction for them"）：设置是给主持人的指令板，不是程序可执行的行为流。

### 下游 Harness 复用前需补的证据

- **手动/辅助选择、等待、明确拒绝、结果未知**：现有工具没有 `player_choice`/`near_limit` 的"请求—应答"台账——没有记录"已询问且正在等待玩家"的状态、没有"玩家明确拒绝"的事件、没有"结果未知"的载体。combat-sim 的反应待决（`take`/`pass`）是战斗内的另一套机制，不属于 settings。Harness 若要把这些控制点类型化，每一点都需要单独取证；现有设置的存在不自动构成这些能力或权限。
- **未实测保持未知**：除上节标明的 `show` 未知 id 分支外，本节所有结论都限于既有回归覆盖的路径；回归未覆盖的行为组合（如 `show` 同时传两个作用域、非 unix 平台的写入失败路径）未实测，状态未知。
- **可见性**：设置数据与审计行的未来玩家可见性（Web 投影显示什么、隐藏什么）是独立决定，本节不做。
