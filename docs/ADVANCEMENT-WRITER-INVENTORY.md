# 新版本存档写者盘点（ADV-P1-1 第一步）

> 状态：**静态盘点**，另见 §5.1 的接入状态。本文的 §2 行号仍以 §0 的基线为准，不改命令业务语义、不迁移存档、不读取真实存档、书籍或秘密。§5.1 记录跨 CLI 过渡门与写者注册表的当前状态（`ADV-P1-1pre` 交付过渡门，`ADV-P1-1a` 将组 A 接入 SAVE 锁），注册表以代码为准。
> 基线：`main` 的 `63cf4fe9317f1385317ab606f37a7a0c23550f76`（TASK-0148 派发时）；行号与行为以该提交的公开源码为准。
> 依据：[首阶段计划](ADVANCEMENT-PHASE-1-PLAN.md)、[锁设计](ADVANCEMENT-LOCK-DESIGN.md) §1/§2、[安全审计](ADVANCEMENT-SAFETY-AUDIT.md)、[身份契约](ADVANCEMENT-IDENTITY-CONTRACT.md) §5、#279、#281、#285。
> 标记约定：**事实** = 当前公开代码/测试可直接读出；**推断** = 由静态事实推出、未实测；**UNKNOWN** = 本次不能判定，不等于安全。现有测试通过**不是**锁安全证据；#264 未解决前，任何“现有锁已够用”的结论都不成立。

## 0. 结论摘要（只写事实与限制）

1. **组 A 的 `talkdnd` 写入口已接入 SAVE 锁；其余写入口仍未接入。** `tools/talkdnd/src/advancement_lock.rs` 的 SAVE 锁 v1 已存在；自 `ADV-P1-1a` 起，盘点 §2.1、§2.2、§2.6 中会写存档的 `talkdnd` 入口（含 `start`；`new` 仍只创建新槽）在写目标前取得该锁，锁不可用时失败关闭且不写目标。`ADV-P1-1pre` 的过渡门仍然拒绝尚未接入的写入口（1b–1e 与 `combat-sim`，以及 `handout list --dm`/`build`/`mark`）。**没有提交/预览调用方，也未启用 D 协议或 schema 11**（事实）。见 §5.1。
2. **现有两把锁都不保护角色/party/进度/日志内容。** checkpoint 的 `.talkdnd.lock` 只用于 `checkpoint snapshot` / `checkpoint restore`，且不是 OS 锁；完整性 `.talkdnd.integrity.lock` 只在 `integrity.yaml` 的读改写期间持有。它们**不能**阻止两个写者先后写同一 `sheets/*.yaml`、`Characters/*.md`、`combat/party-state.yaml`、`progress.yaml`、`rolls.log` 或 `Sessions/*.md`（事实；#264 未解决）。
3. **写存档的入口远多于旧 `progress::level_up`。** 除结构化与 Markdown 两条升级路径外，角色卡 `char`、建卡/同伴、XP/钱袋、场景、识别/揭示/原创/实体/关系/世界时间/记忆/设置、checkpoint/migrate、日志与完整性、`start`/`new`、以及 `combat-sim` 的遭遇与 party 命令都会写存档内文件。本文按组逐项列出。
4. **只读路径必须按命令而不是按模块判断。** `guard` 与 `secret list` 会写 `secret-terms.cache.json`，不得列为只读；`char show` 会走 `charsheet::run` 的 `commit()`，不是只读；`char <who> spells` 的普通视图才在 `commit()` 之前提前返回。`adventure index build` 可在存档内写 `structured.json`，也不是只读。
5. **未来目标大多尚不存在。** schema 11、选择 sidecar、候选树、恢复工作证据与逐文件发布目标**未实现**；`admit` 操作前快照库代码已实现（`advancement_lock.rs:1787-1815`）并有合成子集，但**无 CLI/提交调用方**；当前只有 `save-meta.yaml`（`migrate::write_meta`）是既有的 schema 元数据写者。不得把本文的“建议接入”读成“已接入”。

## 1. 盘点字段与“并发”口径

每条写入口记录：

- **命令 / 代码定位**：CLI 命令与 Rust 函数（文件:行）。
- **写入对象**：存档内相对路径；存档外写入单独标注。
- **现有锁**：`无`、`完整性锁（仅清单读改写）`、`checkpoint 锁`、`advancement 锁（未接线）`。
- **可与成长提交并发**：以[锁设计](ADVANCEMENT-LOCK-DESIGN.md) §3 的 D 发布目标为口径——`sheets/*.yaml` → `combat/party-state.yaml` → `Characters/*.md` → `progress.yaml` → 追加日志 → 完整性清单；未来再加选择 sidecar 与主 schema 元数据。D 流程尚未实现，这里只是**设计目标**，不是现状。
- **建议接入方式**：`SAVE 锁`、`拒绝写入`、`声明只读` 三者之一；未决的语义在 §6 标 `NEEDS_HUMAN_DECISION`。
- **未接入时的行为**：当前真实行为，不写“应该”。
- **备注**：边界、失败形态、是否与 #264/#279/#281/#285 相关。

## 2. 写入口清单

### 2.1 旧升级、进度与钱袋

| 命令 / 代码定位 | 写入对象 | 现有锁 | 可与成长提交并发 | 建议接入 | 未接入时的行为 | 备注 |
|---|---|---|---|---|---|---|
| `talkdnd level-up <who>` 结构化路径：`progress::level_up`（`tools/talkdnd/src/progress.rs:793`）→ `charsheet::party` → `charsheet::commit`（`charsheet.rs:993`） | `sheets/<id>.yaml`、`combat/party-state.yaml`（仅当 `combat/` 存在）、`Characters/*.md` | 无保存级锁；`PartyState::save` 写后取完整性锁更新清单 | **是**（三个目标都在 D 发布集合内） | **SAVE 锁** | 读旧值后整文件 `atomic_write`；三步非事务，中途失败可留下“YAML 新、Markdown 旧”或反之；随后 `Progress::store`、日志追加。 | 旧升级功能必须保留；不得删除或静默改变。`--milestone` 还会写 `progress.yaml`。 |
| `talkdnd level-up <who>` Markdown 路径：`progress::level_up`（`progress.rs:793`）→ `atomic_write(&s.file, …)`（`progress.rs:1047`） | `Characters/*.md`（旧式角色无 `sheets/<id>.yaml`） | 无 | **是**（`Characters/*.md` 在 D 发布集合内） | **SAVE 锁** | 直接整文件原子写 Markdown；`--milestone` 随后 `Progress::store`；最后追加升级日志。 | 与结构化路径同一命令，必须分别盘点与测试。 |
| `talkdnd level-up --hp roll`：`progress.rs:878` → `rolls::append`（`rolls.rs:20`）→ `save::append`（`save.rs:242`） | `rolls.log`（追加） | 无 | **是**（日志是 D 发布/追加目标） | **SAVE 锁** | 在解析/持久化角色**之前**先追加骰记录；后续步骤失败时骰记录已写入，无法判断“是否已应用”。 | 平均值或指定值不追加该骰记录；追加不是原子替换，不能用于幂等判定。 |
| `talkdnd xp add`：`progress::xp_add`（`progress.rs:336`）→ `p.store`（`progress.rs:430`）→ `log`（`progress.rs:304`） | `progress.yaml`、最新 `Sessions/*.md`（追加） | 无 | **是** | **SAVE 锁** | 先写进度再追加日志；两步非事务；重复 `--reason` 默认拒绝，`--again` 可重复。 | 与成长提交的 XP/日志目标重叠。 |
| `talkdnd loot add/spend/give/split`：`progress::loot`（`progress.rs:1168`）→ `p.store`（`progress.rs:1225`）→ `log` | `progress.yaml`、最新 `Sessions/*.md`（追加） | 无 | **是** | **SAVE 锁** | 单文件进度写 + 日志追加；失败可能只写其一。 | `loot show` 只读，见 §3。 |
| `talkdnd scene <name>`：`known::scene_change`（`known.rs:194`）→ `progress::set_scene`（`progress.rs:1107`） | `progress.yaml`、最新 `Sessions/*.md`（追加） | 无 | **是** | **SAVE 锁** | 先写进度再追加日志。 | `scene` 不是只读命令，尽管输出像 `known`。 |

### 2.2 角色卡、建卡与同伴

| 命令 / 代码定位 | 写入对象 | 现有锁 | 可与成长提交并发 | 建议接入 | 未接入时的行为 | 备注 |
|---|---|---|---|---|---|---|
| `talkdnd char <who> <op>`（除 `spells` 只读视图）：`charsheet::run`（`charsheet.rs:3672`）→ `apply` → `commit`（`charsheet.rs:993`） | `sheets/<id>.yaml`、`combat/party-state.yaml`、`Characters/*.md`，并追加最新会话日志（`charsheet.rs:2952`） | 无保存级锁；party 保存后取完整性锁 | **是**（三个目标都在 D 发布集合内） | **SAVE 锁** | 所有 op 先在内存应用，再 `commit()` 一次写 YAML → party → Markdown；批量中任一 op 失败则不写（注释声明），但跨文件仍非事务。 | **`char show` 也走这条路径**，不是只读；#285 已明确排除“char show 是只读候选”。 |
| `talkdnd char <who> spells`（普通视图）：`charsheet::run` 提前返回 `spells_view`（`charsheet.rs:3696`） | 无（不写 sheet/party/Markdown） | 无 | 否（命令本身不写；外层审计可能写清单） | **声明只读** | 不写 sheet、party、Markdown；`spells adopt/sync/terrain` 不在此提前返回范围内，仍会写。 | 只读判定必须区分 `SpellsCmd` 子命令；`Cmd::Char` 仍走 tracked 外层审计，可能改写 `integrity.yaml` baseline（§3.1）。 |
| `talkdnd chargen *`：`chargen_cmd`（`lib.rs:1599`）→ `chargen::store`（`chargen.rs:173`，路径 `chargen.rs:149`） | `chargen.json` | 无 | **是**（存档内文件；不在 D 发布集合，但会与迁移/提交交错） | **SAVE 锁** | `status`/`roll`/`array`/`pointbuy`/`assign`/`asi`/`set`/`validate` 等都会在命令末尾写回 `chargen.json`；`roll` 还会写会话日志/`rolls.log`。 | 建卡状态是存档内容的一部分；不是“只读查询”。 |
| `talkdnd chargen sheet`：`lib.rs:1860` → `atomic_write(Characters/<id>.md)` → `add_to_party`（`lib.rs:1560`）→ `charsheet::from_chargen`（`charsheet.rs:1226`） | `Characters/<id>.md`、`combat/campaign.yaml`（party 列表）、`sheets/<id>.yaml`（`from_chargen` 内 `commit`/`store`） | 无 | **是** | **SAVE 锁** | 写 Markdown → 改 campaign party → 写结构化 sheet；分步执行，非事务。 | 与升级/角色卡写者共享 `Characters/`、`sheets/`、`campaign.yaml`。 |
| `talkdnd chargen confirm`：`lib.rs:1882` → 可能 `progress::loot` → `chargen::store` | `progress.yaml`、`Sessions/*.md`、`chargen.json` | 无 | **是** | **SAVE 锁** | 首次确认可能发放起始金币（写进度 + 日志），然后写回 `chargen.json`。 | 不能只按“建卡状态”处理。 |
| `talkdnd companion random` / `npc random`：`npc_cmd`（`lib.rs:1959`） | `npc-drafts/<n>.json`、`rolls.log`、最新 `Sessions/*.md`（`log_rolls`） | 无 | **是**（日志） | **SAVE 锁** | 有存档时创建 draft JSON、追加属性掷骰日志与 `rolls.log`；无存档时只打印。 | 存档外/内混合；draft 不是 D 发布目标，但日志是。 |
| `talkdnd companion sheet` / `npc sheet`：`lib.rs:2007` | `Characters/<id>.md`、`combat/campaign.yaml`（party） | 无 | **是** | **SAVE 锁** | 先检查同 ID 玩家角色，再写 Markdown 并加入 party；不调用 `from_chargen`。 | 与角色卡写者共享 `Characters/`、`campaign.yaml`。 |
| `lib.rs::add_to_party`（`lib.rs:1560`） | `combat/campaign.yaml` 的 `party:` 行 | 无 | **是**（campaign 是活动遭遇/队伍判定输入） | **SAVE 锁** | 读整文件、替换一行、原子写；不是 D 发布目标，但会影响成长提交的 party 发现。 | 被建卡/同伴命令调用。 |

### 2.3 party / combat 状态（`combat-sim`）

| 命令 / 代码定位 | 写入对象 | 现有锁 | 可与成长提交并发 | 建议接入 | 未接入时的行为 | 备注 |
|---|---|---|---|---|---|---|
| `combat-sim` 会改状态的命令：`init`、`attack`、`grapple`、`shove`、`damage`、`heal`、`save`、`check`、`cast`、`use`、`condition add/remove`、`move`、`ready`、`trigger`、`ready-clear`、`fly`、`fall`、`interact`、`deathsave`、`next`、`rest`、`activate`、`add`；统一走 `run_state_command`（`tools/combat-sim/src/cli.rs:1947`） | `combat/*.state.json`（`EncounterState::save`，`state.rs:1667`）、`combat/party-state.yaml`（`write_party_state` → `PartyState::save`，`party.rs:79`）、`rolls.log`/会话日志（`player::append_combat_lines`，`player.rs:208`） | 无保存级锁；`state.save`/`party.save` 各自在写后取完整性锁更新清单 | **是**（party、日志；`.state.json` 不在 D 发布集合内，但活动遭遇判定会读它） | **SAVE 锁** | 顺序为 load → 执行 → `state.save` → `write_party_state` → 追加日志；三步非事务；失败可能只写其中一部分。 | `combat-sim` 目前**没有**接入 SAVE；锁设计已记录“接入前把遭遇写入当并发风险”。 |
| `combat-sim new` / `quick`：`new_state`（`cli.rs:2192`）/ `quick_state`（`cli.rs:2172`） | `combat/*.state.json`、可能 `combat/party-state.yaml` | 无 | **是**（party/日志） | **SAVE 锁** | 创建/覆盖状态文件；`new` 仅当路径位于 manifest `state_dir` 才关联 party。 | `quick` 不写 scene/encounter YAML，但仍写 state/party。 |
| `combat-sim party rest/set-hp/sync`：`run_party`（`cli.rs:2070` 起） | `combat/party-state.yaml` | 无 | **是**（party 是 D 发布目标） | **SAVE 锁** | `party rest`/`set-hp` 直接改 party；`sync` 从 state 写回 party。 | `party show` 只读，见 §3。 |
| `combat-sim roll`：`cli.rs:1044` | 无 `--state` 时只追加 `rolls.log`（`player::append_roll`，`player.rs:225`）；`--state` 指向已存在文件时走 `run_state_command_quiet`（`cli.rs:1061`），还会写 `combat/*.state.json`、`combat/party-state.yaml` 与会话日志 | 无 | **是**（日志/party；state 不在 D 发布集合） | **SAVE 锁** | 两条分支行为不同：无 state 只写 `rolls.log`；有 state 还会保存 state/party 并追加日志。 | 不能把 `roll` 整条命令当只读或只写日志。 |
| `combat-sim status/scene/log/simulate/content list/show/check/pack list/show/verify` | 无（只读） | 无 | 否（不写） | **声明只读** | 不写 state/party/日志；`--player` 只读 `identified.yaml`。 | `content import-srd-prose`/`content import-srd-pack`/`content import-srd` 与 `content pack build/release` 按 `--out` 位置区分；目标在存档内时是写者，见 §2.7。 |

### 2.4 checkpoint / migrate

| 命令 / 代码定位 | 写入对象 | 现有锁 | 可与成长提交并发 | 建议接入 | 未接入时的行为 | 备注 |
|---|---|---|---|---|---|---|
| `talkdnd checkpoint <text>`：`checkpoint::append`（`checkpoint.rs:162`） | 最新 `Sessions/*.md`（追加）；外层 tracked `audit::after_command` 还可能更新 `integrity.yaml` | 无（不取 checkpoint 锁） | **是**（日志是 D 发布目标） | **SAVE 锁** | 直接追加 `## 存档点`；不创建快照、不取 `.talkdnd.lock`；命令后外层审计可能重写清单 baseline。 | 与 `checkpoint snapshot` 不同；不能因为同属 checkpoint 就认为已加锁。 |
| `talkdnd checkpoint snapshot`：`checkpoint::snapshot`（`checkpoint.rs:179`）→ `Lock::acquire`（`checkpoint.rs:1243`） | `checkpoints/<id>/`（复制覆盖文件 + `manifest.yaml`）、删除超出保留数的旧 checkpoint；外层 tracked `audit::after_command` 还可能更新 `integrity.yaml` | **checkpoint 锁**（`create_new`，非 OS 锁） | **是**（快照读取整树；与 D 发布/恢复交错会得到混合内容） | **SAVE 锁**（外层）→ checkpoint 锁 → 完整性锁 | 取得 `.talkdnd.lock` 后复制文件；`create_new` 冲突一律失败关闭；锁不是 OS 锁，不阻止非协作写者；命令后外层审计可能重写清单 baseline。 | 总序按锁设计：`SAVE → checkpoint .talkdnd.lock → integrity .talkdnd.integrity.lock`。 |
| `talkdnd checkpoint restore --yes`：`restore_inner`（`checkpoint.rs:427`）→ `Lock::acquire`（`checkpoint.rs:442`） | `Saves/_backups/<slot>-…`（恢复前整份备份）、存档覆盖文件（逐文件 rename）、删除快照后新增文件、`migrate::ensure`、会话日志、`integrity.yaml` | **checkpoint 锁** + 完整性锁（迁移/重定基） | **是**（整树替换，与 D 发布/日志/完整性直接冲突） | **SAVE 锁**（外层）→ checkpoint 锁 → 完整性锁 | 先整份备份，再暂存/rename；失败尝试镜像回滚；提交后阶段失败不回滚并报告。锁不保护内容，也不约束非协作写者。 | #264 未解决；不得声称恢复安全。 |
| `talkdnd migrate`（非 dry-run）：`lib.rs:1033` → `save::migrate_ruleset`（`save.rs:154`）→ `migrate::ensure`（`migrate.rs:838`） | `combat/campaign.yaml`（ruleset 行）、`save-meta.yaml`、`sheets/*.yaml`（9→10 法术迁移）、`Characters/`/`NPCs/` 移动、`Saves/_backups/<slot>-…`、最新会话日志、`integrity.yaml` | 无保存级锁；`after_write` 取完整性锁 | **是**（整树写入 + 备份，与 D 发布直接冲突） | **SAVE 锁**（并在 advancement state 非 `ready` 时拒绝） | 先写 ruleset 行，再 `ensure`：快照 → 备份 → 迁移 → 写 meta → 日志 → 完整性；分步非事务，失败可能留下部分迁移。 | `--dry-run` 只读，见 §3；`migrate::backup_save`（`migrate.rs:1484`）名称非原子分配，特殊文件报错中止。 |
| `talkdnd start` 的迁移/内容同步：`start::digest`（`start.rs:503`）→ `migrate::ensure`（`start.rs:552`）→ `library::sync_campaign_packs`（`library.rs:969`） | `save-meta.yaml`、`sheets/*.yaml`、`combat/campaign.yaml`、`digest.md`、`.cache/`、书库目录、`.talkdnd/books.yaml`、`Rules/` 默认裁定与 `.talkdnd/installed-files.txt` | 无保存级锁；迁移内 `after_write` 取完整性锁 | **是**（迁移/campaign 是存档写入） | **SAVE 锁** | `start` 不是纯只读：旧 schema 会触发整档迁移；FULL 规则集可能重写 `library:` 行；还会整理书库、扫描登记、同步默认裁定/已安装文件，最后写 `digest.md` 与缓存。 | 入口事实见计划；不得把 `start` 当只读。 |

### 2.5 日志与完整性

| 命令 / 代码定位 | 写入对象 | 现有锁 | 可与成长提交并发 | 建议接入 | 未接入时的行为 | 备注 |
|---|---|---|---|---|---|---|
| `talkdnd log <text>`：`lib.rs:1218` → `save::append`（`save.rs:242`） | 最新 `Sessions/*.md`（追加） | 无 | **是**（日志是 D 发布/追加目标） | **SAVE 锁** | 追加一行；不读全文，只读末字节决定换行。 | `log rolls` 只读，见 §3。 |
| `talkdnd check <who> <what>`：`lib.rs:1065` → `audit::audit`（`audit.rs:294`）→ `check::run`（`check.rs`） | 可能创建/更新 `integrity.yaml` 与 baseline；`rolls.log`；最新会话日志 | 完整性锁（审计与清单） | **是**（清单/日志） | **SAVE 锁** | 先审计（可能为旧存档创建清单/baseline），再掷骰并写日志/rolls；`audit` 在完整性锁内。 | 即使没有未解释变化，旧存档也可能被创建清单。 |
| `talkdnd check`（无参数）：`lib.rs:1075` → `print_audit` → `audit::audit` | `integrity.yaml`（旧存档首次创建清单/baseline） | 完整性锁 | **是**（清单） | **SAVE 锁** | 输出审计；在旧存档上可能写清单。 | 不能列为纯只读。 |
| `talkdnd integrity status`：`lib.rs:779` → `print_audit` → `audit::audit` | 同上 | 完整性锁 | **是**（清单） | **SAVE 锁** | 旧存档首次运行会创建清单/baseline。 | 与 `check` 无参数同路径。 |
| `talkdnd integrity accept --reason`：`audit::accept`（`audit.rs:372`） | `integrity.yaml`、最新会话日志 | 完整性锁 | **是**（清单/日志） | **SAVE 锁** | 在完整性锁内接受当前文件/值为新 baseline，并记日志。 | 会改变清单 baseline；不是只读。 |
| 普通命令外层 `audit::after_command`（`audit.rs:355`；`lib.rs:740`） | `integrity.yaml`（清单指纹/baseline） | 完整性锁 | **是**（清单） | 随所属命令接入；外层保持完整性锁 | 在 `dispatch` 后取完整性锁，按前后快照更新清单；clean 时可能重写 baseline 日期。 | #279/#281 已把若干只读命令移出 `tracked`；不得把“未跟踪”当成“无写入”。 |
| `combat-sim` / `talkdnd` 的日志追加：`save::append`、`rolls::append`、`player::append_combat_lines`、`player::append_roll` | `Sessions/*.md`、`rolls.log` | 无 | **是** | **SAVE 锁**（随调用方） | 追加不截断；同一操作重复执行会重复追加；无法用追加语义判断“是否已应用”。 | 幂等由 `ADV-P1-4` 设计，不在本文验收。 |

### 2.6 其它存档写入

| 命令 / 代码定位 | 写入对象 | 现有锁 | 可与成长提交并发 | 建议接入 | 未接入时的行为 | 备注 |
|---|---|---|---|---|---|---|
| `talkdnd identify` / `identify --appearance`：`identify::identify`（`identify.rs:93`）/ `set_appearance`（`identify.rs:137`）→ `store`（`identify.rs:65`） | `identified.yaml`、最新会话日志 | 无 | **是**（日志；`identified.yaml` 影响玩家视图） | **SAVE 锁** | 整文件原子写 + 日志追加；两步非事务。 | 被 `combat-sim --player` 读取。 |
| `talkdnd reveal add`：`reveal::add`（`reveal.rs:69`）/ `add_original`（`reveal.rs:131`）→ `store`（`reveal.rs:58`） | `reveals.yaml`、最新会话日志 | 无 | **是**（日志） | **SAVE 锁** | 写揭示账本 + 日志。 | `reveal list/check` 只读。 |
| `talkdnd original add`：`original::add`（`original.rs:389`）→ `store`（`original.rs:254`） | `original.yaml`、最新会话日志 | 无 | **是**（日志） | **SAVE 锁** | 写原创内容账本 + 日志。 | `original list/status` 只读。 |
| `talkdnd original plan`：`original::plan`（`original.rs:578`） | `dm-plan.md`（不存在时创建） | 无 | **是**（存档内文件；不在 D 发布集合） | **SAVE 锁** | 仅当文件不存在时写模板；存在时只打印。 | DM-only 文件仍不得提交。 |
| `talkdnd entity add/rename/retire`：`entity::run`（`entity.rs:91`）→ `store`（`entity.rs:146`）→ `entity::log_line`（`entity.rs:227`） | `entities.yaml`、最新会话日志 | 无 | **是**（日志） | **SAVE 锁** | 写实体账本 + 日志。 | `entity list/show` 只读。 |
| `talkdnd relation add/end`：`graph::run`（`graph.rs:120`）→ `store`（`graph.rs:179`）→ `entity::log_line`（`graph.rs:300`/`424`） | `relations.yaml`、最新会话日志 | 无 | **是**（日志） | **SAVE 锁** | 写关系账本 + 日志。 | `relation list/show/context` 只读。 |
| `talkdnd time enable/disable/set/advance/event add/done/skip`：`worldtime::run`（`worldtime.rs:283`）→ `store`（`worldtime.rs:157`）→ `log_line`（`worldtime.rs:272`） | `world-time.yaml`、最新会话日志 | 无 | **是**（日志） | **SAVE 锁** | 写世界时间 + 日志；写命令在未启用时拒绝。 | `time show/event list` 只读。 |
| `talkdnd memory add/remove`：`memory::run`（`memory.rs:195`）→ `migrate::ensure`（`memory.rs:199`）→ `store`（`memory.rs:122`） | `memory.yaml`、可能整档迁移（旧 schema）、`save-meta.yaml` | 无保存级锁；迁移内完整性锁 | **是**（迁移可能写角色/进度） | **SAVE 锁** | 首次使用会先迁移旧存档（整份备份 + 写入）；然后写记忆。 | `memory list` 只读。 |
| `talkdnd settings dice on/off`：`settings::set_dice`（`settings.rs:53`）→ `migrate::ensure`（`settings.rs:60`）→ `write_meta`（`settings.rs:65`）→ 追加日志（`settings.rs:72`） | `save-meta.yaml`、最新会话日志、可能整档迁移 | 无保存级锁；迁移内完整性锁 | **是**（迁移写角色/进度；日志是 D 目标） | **SAVE 锁** | 先确保 schema，再写设置并追加日志。 | `settings` 无子命令 / `settings dice` 不带值只读，见 §3。 |
| `talkdnd settings style set`：`memory::run_style`（`memory.rs:386`）→ `set_style` → `migrate::ensure`（`memory.rs:359`）→ `write_meta`（`memory.rs:371`）→ 追加日志 | `save-meta.yaml`、最新会话日志、可能整档迁移 | 无保存级锁；迁移内完整性锁 | **是**（迁移；日志） | **SAVE 锁** | 写叙事风格到 `save-meta.yaml` 并追加日志；先迁移。 | 不写 `memory.yaml`；`settings style show` 只读。 |
| `talkdnd settings automation ...`：`settings::run_automation`（`settings.rs:657`）→ `migrate::ensure`（`settings.rs:455`/`542`）→ `write_meta`（`settings.rs:509`/`616`）→ `log_line` | `save-meta.yaml`、最新会话日志、可能整档迁移 | 无保存级锁；迁移内完整性锁 | **是**（迁移；日志） | **SAVE 锁** | 写自动化设置并追加日志；先迁移。 | `settings automation` 无子命令只读。 |
| `talkdnd secret add/remove`：`secret::add`（`secret.rs:96`）/ `remove`（`secret.rs:124`）→ `store_manual`（`secret.rs:82`）→ `invalidate`（`secret.rs:92`） | `secret-terms.yaml`（手动秘密词）、删除 `secret-terms.cache.json` | 无 | **是**（存档内文件；不在 D 发布集合） | **SAVE 锁** | 写手动词并删除缓存；不追加日志。 | `secret import --apply` 同路径。 |
| `talkdnd secret import --apply`：`import_lines`（`secret.rs:658`）→ `store_manual` → `invalidate` | `secret-terms.yaml`、删除缓存 | 无 | **是**（存档内文件） | **SAVE 锁** | 默认 dry-run 只读；`--apply` 写词并失效缓存。 | dry-run 见 §3。 |
| `talkdnd handout show`：`handout::show`（`handout.rs:391`） | `handouts/<id>.<ext>`、`handouts.yaml`（shown 记录）、`reveals.yaml`、最新会话日志 | 无 | **是**（日志/reveals；图片不在 D 发布集合） | **SAVE 锁** | 导出图片、记录 shown、写 reveal 或日志；分步非事务。 | 只导出 player-safe 条目；DM-only 拒绝。 |
| `talkdnd adventure section`（超前/DM-only 且带 `--reason`）：`adventure::section`（`adventure.rs:549`） | 最新会话日志（`adventure.rs:617`） | 无 | **是**（日志） | **SAVE 锁** | 普通读取不写；只有超前/DM-only 带原因时追加日志。 | 不能整组声明只读；`--list` 只读。 |
| `talkdnd adventure index build`：`bookindex::run`（`bookindex.rs:49`）→ `build_one` → `atomic_write`（`bookindex.rs:208`） | 书目录内 `structured.json`（若 `--book` 指向存档内目录，则写存档内） | 无 | **是**（若目标在存档内） | **SAVE 锁**（目标在存档内）；否则声明只读（存档外） | 写/刷新索引；可能跳过未变来源。 | #281 已明确 build 不是只读；`show/search` 只读。 |
| `talkdnd start`：`start::digest`（`start.rs:503`） | `digest.md`、`.cache/`（`start.rs:93`）、`save-meta.yaml`/`sheets`/`combat/campaign.yaml`（迁移/内容同步）、书库目录（`library::tidy`）、`.talkdnd/books.yaml`、`Rules/` 默认裁定与 `.talkdnd/installed-files.txt`（`update::sync_*`） | 无保存级锁；迁移内 `after_write` 取完整性锁 | **是**（迁移/campaign 是存档写入；书库/缓存为存档外） | **SAVE 锁** | `start` 不是纯只读：会整理书库、扫描登记、同步 campaign 内容源、迁移旧 schema、重指书本路径、同步默认裁定/已安装文件，最后写 `digest.md` 与缓存。 | 入口事实见计划；不得把 `start` 当只读。 |
| `talkdnd new`：`new_save`（`lib.rs:1454`）；成功后 `lib.rs:745` 调 `audit::audit` | 新 `Saves/<slot>/`：`combat/campaign.yaml`、`canon.md`、`dm-secrets.md`、`Sessions/*-session-00-setup.md`、`chargen.json`、`save-meta.yaml`、`integrity.yaml`（成功后审计创建）、`Rules/` 默认裁定副本 | 无（新存档） | **不适用**（创建新存档；未初始化语义未决） | **拒绝写入 / 新建时初始化**（`NEEDS_HUMAN_DECISION`） | 直接创建目录与文件；成功后 `audit::audit` 可能创建 `integrity.yaml` baseline；不取任何保存级锁。 | 新存档是否自动 `init` 成长锁、以及 `new` 与未来 SAVE 的关系未决。 |

### 2.7 存档外写入与按输出位置区分的写入

| 命令 / 代码定位 | 写入对象 | 现有锁 | 可与成长提交并发 | 建议接入 | 未接入时的行为 | 备注 |
|---|---|---|---|---|---|---|
| `talkdnd library tidy/merge`：`library::tidy`（`library.rs:2037`）/ `merge_parts`（`library.rs:1618`） | `Adventures/`、`Rules/<edition>/` 书籍目录 | 无 | 否（存档外） | **声明只读（就存档内容而言）** | 移动/合并书籍目录；`--dry-run` 不写。 | 不得宣称“整个命令只读”；它写书库。 |
| `talkdnd library scan/packs/choose`：`scan_and_register`（`library.rs:632`）/ `choose_book`（`library.rs:655`） | `.talkdnd/books.yaml`（书库识别记录） | 无 | 否（存档外） | **声明只读（就存档内容而言）** | `scan`/`packs` 会写识别记录；`packs` 只是打印状态。 | 不写存档内容。 |
| `talkdnd data import`：`data_cmd`（`lib.rs:2090`）→ `classdata::run_import`（`classdata.rs:578`） | `Rules/chargen/local/imported-<ruleset>.yaml` | 无 | 否（存档外） | **声明只读（就存档内容而言）** | 从转换后的 Markdown 生成本地数据文件。 | `data status` 只读。 |
| `talkdnd handout build/mark`：`handout::rebuild`（`handout.rs:564`）/ `mark`（`handout.rs:600`）→ `store`（`handout.rs:275`） | 书目录内 `handouts.yaml`（可能位于 `Adventures/`/`Rules/`） | 无 | 否（通常存档外；若书目录在存档内则按写存档处理） | **声明只读（存档外）；目标在存档内时拒绝写入** | 刷新/标记 registry；`mark` 不写存档内容。 | 不能因为 `handout` 有 `show` 写存档就把整组当只读。 |
| `talkdnd` 的 `update::sync_defaults`（`update.rs:115`，由 `new` 调用） | `Rules/rulings*.md`、`Rules/.talkdnd/...` | 无 | 否（存档外） | **声明只读（就存档内容而言）** | 复制出厂裁定默认。 | 不是 CLI 命令，随 `new` 执行。 |
| `combat-sim content import-srd-prose` / `content import-srd-pack` / `content import-srd` / `content pack build/release`：`cli.rs:1554` 起 | 按 `--out` 实际位置：目标在存档内时写存档内内容文件；目标在存档外时写书籍/内容文件 | 无 | 目标在存档内时 **是**；存档外否 | 目标在存档内：**SAVE 锁或拒绝写入**；存档外：声明只读（就存档内容而言） | 三个 `import-*` 命令没有 dry-run，非 dry-run 写内容文件；`content pack build/release` 的 `--dry-run` 不写，非 dry-run 写 pack/内容文件。 | 不得整组列为只读；`ADV-P1-1b` AC-6 负责接入或拒绝并做合成测试，提交入口覆盖门包含它们。 |

### 2.8 缓存 / 派生写入

| 命令 / 代码定位 | 写入对象 | 现有锁 | 可与成长提交并发 | 建议接入 | 未接入时的行为 | 备注 |
|---|---|---|---|---|---|---|
| `talkdnd guard`：`secret::guard_terms`（`secret.rs:493`）→ `collect`（`secret.rs:458`）→ `store_cache`（`secret.rs:237`） | `secret-terms.cache.json` | 无 | 否（缓存不在 D 发布目标；但会读半提交状态） | **SAVE 锁**（保守）或改为不写缓存（`NEEDS_HUMAN_DECISION`） | 首次/失效时收集秘密词并写缓存；`guard` 本身不改角色/进度。 | **不得列为只读**；#285 已提醒 `guard` 会写缓存。 |
| `talkdnd secret list`：`list_lines`（`secret.rs:145`）→ `collect` → `store_cache` | `secret-terms.cache.json` | 无 | 否（缓存） | **SAVE 锁**（保守）或改为不写缓存（`NEEDS_HUMAN_DECISION`） | 列词时可能写缓存。 | 与 `guard` 同一缓存路径；不得列为只读。 |
| `talkdnd start` 的 `digest.md` / `.cache/`：`start.rs:798` / `start.rs:93` | `digest.md`、`.cache/` | 无 | 否（缓存/派生；不在 D 发布目标） | **SAVE 锁**（随 `start`）或声明为控制元数据 | 每次 `start` 重写 digest 与缓存。 | 设计上属于控制元数据；清理责任未定。 |

### 2.9 未来目标（尚不存在）

| 目标 | 当前事实 | 现有锁 | 可与成长提交并发 | 建议接入 | 未接入时的行为 | 备注 |
|---|---|---|---|---|---|---|
| `save-meta.yaml`（schema 元数据） | 当前写者 `migrate::write_meta`（`migrate.rs:623`），由 `init`/`ensure`/设置写入 | 无保存级锁 | **是**（D 未来主 schema 元数据目标） | **SAVE 锁** | 直接原子写；`ensure` 先备份再写。 | 既有文件；不是“未来才出现”，但未来会作为成长提交的发布目标。 |
| 选择 sidecar（身份/选择） | **不存在**；无字段、路径、随机源或写入代码 | 无 | **是（设计目标）** | **SAVE 锁** + 由 `ADV-P1-4` 注册为发布目标 | 无写入路径；不得声称已保护。 | 具体方案未授权；由 `ADV-P1-6` 落地，`ADV-P1-10` 接线。 |
| schema 11 元数据 | **未实现**；`CURRENT_SCHEMA` 为 10 | 无 | **是（设计目标）** | **SAVE 锁**（随迁移/提交） | 9/10→11 迁移未实现；未来/未知 schema 拒绝。 | 不得写成已完成。 |
| `admit` 操作前快照（prestate） | 库代码已实现（`advancement_lock.rs:1787-1815`），有合成子集；**无 CLI/提交调用方** | 无 | **是（设计目标）** | **SAVE 锁** + 控制元数据单独保留 | 无 CLI 写入路径；不得声称已接线或已保护。 | 控制元数据不并入用户数据完整备份（OD-7）；保留期未定。 |
| 候选树 / 恢复工作证据 | **不存在**（锁设计 §2.9 仅为设计；恢复工作证据未实现） | 无 | **是（设计目标）** | **SAVE 锁** + 控制元数据单独保留 | 无写入路径。 | 控制元数据不并入用户数据完整备份（OD-7）；保留期未定。 |
| D 完整备份目录 | 当前 `migrate::backup_save` / checkpoint restore 备份已存在；D 的“完整备份”未实现 | 无（名称非原子分配） | **是**（整树复制与 D 发布冲突） | **SAVE 锁** | 现有备份不是锁内快照；特殊文件报错中止。 | 不得把“备份目录存在”当成完整备份。 |

## 3. 只读声明与 #279/#281/#285 对照

### 3.1 已确认只读（本次静态核对）

| 命令 / 代码定位 | 只读依据 | 外层审计（当前） | 备注 |
|---|---|---|---|
| `talkdnd oracle`（`oracle::run`） | #279 已把 `Cmd::Oracle` 移出 `tracked`（`lib.rs:717`）；不读写存档。 | `None`（#279） | 不取快照、不审计。 |
| `talkdnd known`（`known::lines`） | #281 移出 `tracked`（`lib.rs:720`）；只读。 | `None`（#281） | 输出 `known`。 |
| `talkdnd knowledge search/get/cite/context/related`（`knowledge::run`，`knowledge.rs:121`） | 只读知识 API；`Cmd::Knowledge` 在 `tracked=None` 列表（`lib.rs:712`）。 | `None`（`Cmd::Knowledge`） | 不写存档。 |
| `talkdnd snapshot diff`（`diff::run`） | #281 移出 `tracked`（`lib.rs:720`）；只读存档与 checkpoint。 | `None`（#281） | `snapshot` 其他子命令未在本行声明。 |
| `talkdnd adventure search`（`adventure::search`） | #281 移出 `tracked`（`lib.rs:732`）；只读。 | `None`（#281） | 不写日志。 |
| `talkdnd adventure index show/search`（`bookindex::run` Show/Search） | #281 移出 `tracked`（`lib.rs:729`）；只读。 | `None`（#281） | `index build` 不是只读（§2.6）。 |
| `talkdnd adventure section --list` | `adventure::section` 的 `list` 分支只列标题；不写日志。 | `tracked`（`Section` 保留） | 非 `--list` 且超前/DM-only 带 `--reason` 会写日志（§2.6），不能整组声明只读。 |
| `talkdnd log rolls`（`lib.rs:1220`） | `rolls::show` 只读 `rolls.log`。 | `tracked` | 不追加。 |
| `talkdnd xp show`（`progress::xp_show`，`progress.rs:447`） | 仅 `check::sheets` + `Progress::load`。 | `tracked`（#285 未闭合） | 只读。 |
| `talkdnd loot show`（`progress::loot_show`，`progress.rs:1246`） | 仅 `Progress::load`。 | `tracked` | 只读。 |
| `talkdnd memory list`（`memory::run` List，`memory.rs:195`） | 仅 `load`。 | `tracked`（#285 未闭合） | 只读。 |
| `talkdnd entity list/show`（`entity::run`，`entity.rs:91`） | 仅 `load`；`show` 读取关系但只读。 | `tracked`（#285 未闭合） | 只读。 |
| `talkdnd relation list/show/context`（`graph::run`，`graph.rs:120`） | 仅 `load`。 | `tracked`（#285 未闭合） | 只读。 |
| `talkdnd time show/event list`（`worldtime::run`，`worldtime.rs:283`） | 仅 `load`。 | `tracked`（#285 未闭合） | 只读。 |
| `talkdnd settings`（无子命令）/ `settings dice`（不带值）/ `settings automation`（无子命令） | `settings::lines`（`settings.rs:38`）/ `automation_lines`（`settings.rs:248`）只读 `migrate::load`。 | `tracked`（#285 未闭合） | 只读。 |
| `talkdnd settings style show`（`memory::run_style` Show，`memory.rs:386`） | 仅读取 memory/设置。 | `tracked` | 只读。 |
| `talkdnd checkpoint list/show`（`checkpoint::run` List/Show，`checkpoint.rs:150`） | 只读 checkpoint 目录与 manifest。 | `tracked` | 不取锁、不写 checkpoint。 |
| `talkdnd migrate --dry-run`（`lib.rs:1033` → `ensure(..., true)`） | `migrate::ensure` 的 dry-run 分支只读。 | `None`（`Cmd::Migrate`） | 不写备份、不迁移。 |
| `talkdnd secret scan`（`secret::scan_lines`，`secret.rs:542`） | 只读指定文件并报告数量/行号。 | `tracked` | 不写缓存。 |
| `talkdnd secret import`（默认 dry-run，`secret.rs:658` 的 `!apply` 分支） | 只读来源文件与手动词，输出预览。 | `tracked` | `--apply` 会写（§2.6）。 |
| `talkdnd handout list`（`handout::list`，`handout.rs:519`） | 只读 registry。 | `tracked` | 仅 player 视图只读；`--dm` 会在 registry 缺失时 build/store 书目录 `handouts.yaml`，按写者处理。 |
| `talkdnd original list/status`（`original::list`/`status_lines`） | 只读 `original.yaml`。 | `tracked` | `original add/plan` 会写。 |
| `talkdnd reveal list/check`（`reveal::list`/`check`） | 只读 `reveals.yaml`。 | `tracked` | `reveal add` 会写。 |
| `talkdnd options`（`print_options`） | 只读规则数据。 | `None`（`Cmd::Options`） | 不写存档。 |
| `talkdnd data status`（`lib.rs:2050`） | 只读本地数据文件。 | `None`（`Cmd::Data`） | `data import` 会写存档外文件。 |
| `combat-sim status/scene/log/simulate` | 只读 state/日志；`--player` 只读 `identified.yaml`。 | `N/A`（无 talkdnd 审计） | `simulate` 不写 state。 |
| `combat-sim content list/show/check` / `pack list/show/verify` | 只读内容文件。 | `N/A` | `content import-srd-prose`/`content import-srd-pack`/`content import-srd` 与 `content pack build/release` 按 `--out` 位置区分；目标在存档内时是写者。 |
| `combat-sim party show` | 只读 party-state。 | `N/A` | `party rest/set-hp/sync` 会写。 |

**只读命令 ≠ 整次运行不写存档。** 上表中 `tracked` 的只读命令仍会走 `lib.rs::run` 的外层 `audit::after_command`（`lib.rs:740`），在干净存档上可能重写 `integrity.yaml` 的分钟级 baseline；#285 正是跟踪这些副作用。本文把“声明只读”作为**接入目标**；在 #285 闭合或这些命令被移出 `tracked` 之前，不得把当前实现说成“已只读”或“不写任何文件”。`tracked=None` 的条目由 #279/#281 修复或本来就在 `None` 列表，仍以各自 Issue 与代码为准。

**`char <who> spells` 的只读视图**：`charsheet::run` 在 `commit()` 之前提前返回（`charsheet.rs:3696`），本身不写 sheet/party/Markdown；但 `Cmd::Char` 仍走 `tracked` 外层审计，可能改写 `integrity.yaml` baseline。接入时必须同时处理这层副作用，否则“只读视图”仍会改变存档字节。

### 3.2 明确**不是**只读的易混路径

| 命令 / 路径 | 为什么不是只读 | 依据 |
|---|---|---|
| `guard` | `secret::guard_terms` → `collect` → `store_cache` 写 `secret-terms.cache.json` | #285 提醒；`secret.rs:237`/`458`/`493`。 |
| `secret list` | 同上，`list_lines` → `collect` → 缓存写入 | `secret.rs:145`/`458`。 |
| `char show` | `Op::Show` 走 `charsheet::run` 的 `commit()`，写 YAML/party/Markdown；普通 `spells` 视图在 `commit()` 前提前返回 | #285 已明确；`charsheet.rs:3672`/`993`/`3696`。 |
| `char <who> spells adopt/sync/terrain` | 不满足只读提前返回条件，仍走 `commit()` | `charsheet.rs:3696`。 |
| `adventure index build` | 可在书目录（含存档内目录）写 `structured.json` | #281 已明确；`bookindex.rs:208`。 |
| `adventure section`（超前/DM-only + `--reason`） | 追加会话日志 | #281 已明确；`adventure.rs:617`。 |
| `check`（无参数）/ `integrity status` | `audit::audit` 可能为旧存档创建清单/baseline | `audit.rs:294`。 |
| `start` | 迁移旧 schema、同步 campaign 内容源、写 digest/缓存 | `start.rs:503`/`552`；`library.rs:969`。 |
| `scene` | 写 `progress.yaml` + 日志 | `progress.rs:1107`。 |
| `new` | 创建整份新存档 | `lib.rs:1454`。 |

## 4. 与成长提交的并发关系（事实与推断）

**事实**：组 A 的 `talkdnd` 写入口与组 B 的 `combat-sim` 写入口在写目标前取得 SAVE 锁（见 §5.1）；其余写入口仍不取 SAVE 锁，由过渡门在协议存档上拒绝。完整性锁只覆盖 `integrity.yaml` 的读改写，不保护 party/state 内容；checkpoint 锁只覆盖 snapshot/restore，且不是 OS 锁。组 A / 组 B 的锁序是 `SAVE → checkpoint .talkdnd.lock → integrity .talkdnd.integrity.lock`。**推断**：两个未接入或非协作的写者仍可能先后写同一目标；日志追加通常不截断彼此，但重复操作会重复追加，无法用追加语义判断“是否已应用”。

按 D 发布目标分组：

| D 目标（设计） | 当前写者（本文对应行） | 风险 |
|---|---|---|
| `sheets/*.yaml` | `level-up`、`char`、`chargen sheet`、`migrate`、`start`、`memory/settings` 的迁移 | 整文件覆盖；无锁；迁移可能 serde 重写（dirty 保护见审计）。 |
| `combat/party-state.yaml` | `charsheet::commit`、`combat-sim` state 命令、`party rest/set-hp/sync` | 三步非事务；party 保存后才更新清单。 |
| `Characters/*.md` | `level-up` 两条路径、`char`、`chargen sheet`、`companion sheet`、`migrate` | 与 YAML 非事务；外部编辑器不受约束。 |
| `progress.yaml` | `level-up --milestone`、`xp add`、`loot`、`scene`、`chargen confirm` | 无锁；与日志两步非事务。 |
| 追加日志（`Sessions/*.md`、`rolls.log`） | 几乎所有写命令与 `combat-sim` | 追加可交错；重复请求重复追加；无操作 ID 去重。 |
| `integrity.yaml` | 所有 `tracked` 命令的 `after_command`、`party/state save`、`migrate`、`restore_rebaseline`、`integrity accept` | 完整性锁只保护清单本身，不保护内容；clean 时可能重写 baseline 日期。 |
| 未来 sidecar / `save-meta.yaml` | `save-meta.yaml` 由 `migrate::write_meta`；sidecar 不存在 | 未来目标必须由 `ADV-P1-4` 注册并纳入 D 协议；当前不得声称已保护。 |
| 存档内任意 `--out` 内容文件 | `combat-sim content import-srd-prose`、`content import-srd-pack`、`content import-srd`、`content pack build/release` | 可能写进 D 目标或未知路径；目标在存档内时必须 SAVE 或拒绝写入（`ADV-P1-1b` AC-6）。 |

## 5. 建议接入分组（对应计划子块）

| 组 | 覆盖的入口 | 计划子块 |
|---|---|---|
| 跨 CLI 过渡门 / 写者注册表 | §2 全部写存档入口（`talkdnd` 与 `combat-sim`） | `ADV-P1-1pre` |
| A. CLI 角色 / 进度 / 建卡 / 账本 / 设置 | §2.1、§2.2、§2.6（identify/reveal/original/entity/relation/time/memory/settings/secret/handout/adventure section/index build） | `ADV-P1-1a` |
| B. party / combat | §2.3、§2.7 的 `content import-srd-prose`/`content import-srd-pack`/`content import-srd`/`content pack build/release`（当 `--out` 在存档内时） | `ADV-P1-1b` |
| C. checkpoint / migrate | §2.4 | `ADV-P1-1c` |
| D. 日志 / 完整性 | §2.5、§2.8 的日志相关路径 | `ADV-P1-1d` |
| E. sidecar / schema 元数据 / 缓存 | §2.8、§2.9 | `ADV-P1-1e` |

分组只表示接入顺序，不表示组内每个入口已经安全；每组必须逐条验收。

### 5.1 过渡门与接入状态（`ADV-P1-1pre` / `ADV-P1-1a` / `ADV-P1-1b`）

`ADV-P1-1pre` 交付了一个共享的**过渡门**：存档存在 `.talkdnd.advancement.lock` 或 `.talkdnd.advancement.state` 时，尚未接入成长协议的写存档入口失败关闭（退出码 1、明确文案、不写目标），只读入口保持可用。`ADV-P1-1a` 将组 A（§2.1、§2.2、§2.6 的 `talkdnd` 写入口，含 `start`）从“过渡门拒绝”改为“已接入 SAVE 锁”。`ADV-P1-1b` 将组 B（§2.3 的 `combat-sim` 状态/`new`/`quick`/`party rest|set-hp|sync`/`roll`，以及 §2.7 中 `--out` 可能落入存档的 `content import-*` 与 `content pack build|release`）同样改为“已接入 SAVE 锁”，并把 SAVE 锁原语从 `talkdnd` 迁入共享 crate `tools/advancement-lock`（两套 CLI 共同依赖；`combat-sim` 不依赖 `talkdnd`）。这些入口在写目标前取得已有的 SAVE 锁 v1（`open` + `check_ordinary_write`），锁不可用（state 缺失、不可写、`active` 或 `in_doubt`）时失败关闭且不写目标。缺失 SAVE state 时的拒绝是接入期的保守处理，不自动 `init`。`--out` 在存档外时按存档外写入允许；`pack build|release --dry-run` 只读。**没有启用提交入口、没有新增 schema/sidecar、没有实现 D 提交。** 1c–1e 与 §2.7 的 `handout list --dm`/`build`/`mark` 仍走过渡门拒绝。

**实现位置（代码为准）：**

- 共享 crate `tools/advancement-lock`：控制文件名 `LOCK_FILE`/`STATE_FILE`、`has_protocol`、`enclosing_protocol_dir`、写者注册表 `WRITERS`、分类 `Class`（含 `AdoptedWrite`）、统一拒绝错误 `Refused` 与 `--help` 文案，以及 SAVE 锁原语（`init`/`open`、`check_ordinary_write`、`check_writable`）。普通写入必须用 `check_ordinary_write`（不要求目录 sync，以免 Windows 上拒绝所有普通写入）；D 提交仍应使用 `check_writable`。依赖 `serde`/`serde_json`/`sha2`/`chrono`。
- `talkdnd`：`tools/talkdnd/src/lib.rs` 的 `gate_entry`（对 `Cmd` 穷尽分类）、`run()` 中的门检查，以及对组 A `AdoptedWrite` 在 dispatch 前取得 SAVE 锁（`open` + `check_ordinary_write`，持锁至命令后审计结束）；`tools/talkdnd/src/advancement_lock.rs` 再导出共享 crate 的 v1 原语，组 A 行为不变。
- `combat-sim`：`tools/combat-sim/src/cli.rs` 的 `gate_entry`、`write_targets`/`content_out_dirs`、`check_transition_gate` 与 `run_cli` 中对组 B `AdoptedWrite` 在 dispatch 前取得 SAVE 锁（同样 `open` + `check_ordinary_write`，持锁至命令与完整性清单更新结束）。`PartyState::save` 与 `EncounterState::save` 仍在写后更新完整性清单，但完整性锁只序列化清单、不再被当作内容保护；内容写入受 SAVE 锁约束。单次状态命令的 `state.save` → `party.save` → 日志追加在 SAVE 内执行，失败时报告已写/未写阶段，不声称多文件原子性。`combat-sim` 不依赖 `talkdnd`。
- 测试：`tools/advancement-lock` 单元测试与 `tests/save_lock.rs`、`tools/talkdnd/tests/transition_gate.rs`、`tools/talkdnd/tests/advancement_save_lock.rs`、`tools/talkdnd/tests/advancement_lock.rs`（经再导出回归原语）、`tools/combat-sim/tests/transition_gate.rs`、`tools/combat-sim/tests/advancement_save_lock.rs`（CLI 测试直接调用真实二进制，不是只测包装函数）。

**门的语义（两套 CLI 相同）：**

| 情形 | 行为 |
|---|---|
| 没有 lock/state | 既有行为完全不变；门不创建控制文件，也不自动 `init` |
| 有任一控制文件 + 未接入写入口 | 退出码 1，文案说明“已启用成长协议 / 该入口尚未接入 / 未写入任何目标文件”；在 dispatch 之前拒绝 |
| 有控制文件 + 只读入口 | 命令照常运行；同时关闭该次运行的命令后完整性审计写入，保证整次运行不改存档字节（#285 跟踪无协议时的同类副作用） |
| 有控制文件 + 缓存/控制元数据（`guard`、`secret list` 的 `secret-terms.cache.json`） | 放行；缓存写入按控制元数据，不取 SAVE 锁、不并入用户备份 |
| 有控制文件 + 已接入写入口（`AdoptedWrite`） | 写目标前取得 SAVE 锁；state 缺失、不可写、`active` 或 `in_doubt` 时退出码 1，文案说明原因并声明“未写入任何目标文件”；取得锁后若命令失败，文案说明“目标文件可能已被写入” |
| 有控制文件 + `start` | **已接入 SAVE 锁**（`AdoptedWrite`）：用户数据/迁移写入取 SAVE 锁；`digest.md`/`.cache` 仍是控制元数据，不单独取 SAVE 锁。锁不可用时整条命令拒绝，也不写 digest/缓存 |
| `new`（新存档） | 放行：只创建新槽，不写已启用协议的存档 |
| 控制文件存在但损坏 / 零字节 / 悬空符号链接 | 一律视为“协议存在”，失败关闭（不做任何解析） |
| 写目标无法解析为绝对路径（不可读祖先、符号链接环、缺失段后的 `..`、无工作目录） | 失败关闭（`Unresolved`，退出码 1）：门不会猜测它属于哪个存档 |
| `--book` 指向另一存档内的书目录（`adventure index build`、`handout show/list --dm/build/mark`） | 逐目标解析其所属存档；已接入的 `adventure index build` / `handout show` 对该存档取 SAVE 锁，锁不可用则拒绝且不写；尚未接入的 `handout list --dm`/`build`/`mark` 仍由过渡门拒绝 |
| 最终组件是符号链接 / 会被原子 rename 替换的目标 | 同时检查**目录项路径**（父目录解析、末组件不跟随）与**跟随路径**（末组件存在时 canonicalize）；任一落在协议存档内即拒绝 |
| `combat-sim` 的具体输出文件（`<save>/rolls.log`、`content pack release` 的每个 `<out>/<pack 文件名>` 与 `full-packs-manifest.json`） | 按文件而非只按目录检查目录项与跟随路径 |
| `talkdnd new` 的命令后审计 | 只审计 `new_save` 实际创建的路径；不再用“最新存档”推断，避免审计到更晚日期的旧存档 |
| `combat-sim` 的 campaign `state_dir/party-state.yaml` 或 state 里记录的 `party_state` 指向已启用协议的存档 | 在首次写入前拒绝，即使 `--save` 指向未受保护的目录 |

**§2 行 → 注册表条目映射（`WRITERS` 逐条对应，注册表是权威）：**

| 盘点位置 | 注册表条目 | 类别 | 已接入 SAVE |
|---|---|---|---|
| §2.1 `level-up` 结构化路径 | `talkdnd:level-up` | `AdoptedWrite` | 是（已接入 SAVE 锁） |
| §2.1 `level-up` Markdown 路径 | `talkdnd:level-up`（同一命令的两条路径） | `AdoptedWrite` | 是（已接入 SAVE 锁） |
| §2.1 `level-up --hp roll` | `talkdnd:level-up` | `AdoptedWrite` | 是（已接入 SAVE 锁） |
| §2.1 `xp add` | `talkdnd:xp-add` | `AdoptedWrite` | 是（已接入 SAVE 锁） |
| §2.1 `loot add/spend/give/split` | `talkdnd:loot` | `AdoptedWrite` | 是（已接入 SAVE 锁） |
| §2.1 `scene` | `talkdnd:scene` | `AdoptedWrite` | 是（已接入 SAVE 锁） |
| §2.2 `char` 全部 op | `talkdnd:char` | `AdoptedWrite` | 是（已接入 SAVE 锁） |
| §2.2 `char <who> spells` 普通视图 | `talkdnd:char-spells-read` | `ReadOnly` | 不适用（不写存档） |
| §2.2 `chargen roll/array/pointbuy/assign/asi/set` | `talkdnd:chargen-state` | `AdoptedWrite` | 是（已接入 SAVE 锁） |
| §2.2 `chargen sheet` | `talkdnd:chargen-sheet` | `AdoptedWrite` | 是（已接入 SAVE 锁） |
| §2.2 `chargen confirm` | `talkdnd:chargen-confirm` | `AdoptedWrite` | 是（已接入 SAVE 锁） |
| §2.2 `companion/npc random` | `talkdnd:companion-random` | `AdoptedWrite` | 是（已接入 SAVE 锁） |
| §2.2 `companion/npc sheet`（含 `lib.rs::add_to_party`） | `talkdnd:companion-sheet` | `AdoptedWrite` | 是（已接入 SAVE 锁） |
| §2.2 `lib.rs::add_to_party`（内部） | 归入 `talkdnd:chargen-sheet` / `talkdnd:companion-sheet` 的定位字段 | — | 随调用方 |
| §2.3 `combat-sim` 状态命令 | `combat-sim:state-write` | `AdoptedWrite` | 是（SAVE 锁；`open` + `check_ordinary_write`） |
| §2.3 `new` / `quick` | `combat-sim:new` / `combat-sim:quick` | `AdoptedWrite` | 是（SAVE 锁） |
| §2.3 `party rest/set-hp/sync` | `combat-sim:party-write` | `AdoptedWrite` | 是（SAVE 锁） |
| §2.3 `roll` | `combat-sim:roll` | `AdoptedWrite` | 是（SAVE 锁；按实际写目标：无 `--state` 只追加 `rolls.log`） |
| §2.3 `status/scene/log/simulate` | `combat-sim:status` / `:scene` / `:log` / `:simulate` | `ReadOnly` | 不适用 |
| §2.3 `content list/show/check`、`pack list/show/verify`、`party show` | `combat-sim:content-read` / `:pack-read` / `:party-show` | `ReadOnly` | 不适用 |
| §2.4 `checkpoint <text>` | `talkdnd:checkpoint-text` | `UnadoptedWrite` | 否（过渡门拒绝） |
| §2.4 `checkpoint snapshot` | `talkdnd:checkpoint-snapshot` | `UnadoptedWrite` | 否（过渡门拒绝） |
| §2.4 `checkpoint restore` | `talkdnd:checkpoint-restore` | `UnadoptedWrite` | 否（过渡门拒绝） |
| §2.4 `migrate`（非 dry-run） | `talkdnd:migrate` | `UnadoptedWrite` | 否（过渡门拒绝） |
| §2.4 `start` | `talkdnd:start` | `AdoptedWrite` | 是（已接入 SAVE 锁；digest/缓存不单独取锁，锁不可用时也不写） |
| §2.5 `log <text>` | `talkdnd:log` | `UnadoptedWrite` | 否（过渡门拒绝） |
| §2.5 `check`（含无参数） | `talkdnd:check` | `UnadoptedWrite` | 否（过渡门拒绝） |
| §2.5 `integrity status` | `talkdnd:integrity-status` | `UnadoptedWrite` | 否（过渡门拒绝） |
| §2.5 `integrity accept` | `talkdnd:integrity-accept` | `UnadoptedWrite` | 否（过渡门拒绝） |
| §2.5 外层 `audit::after_command` | 随所属命令；协议存在时对只读/控制元数据命令关闭 | — | 见 §5.1 语义表 |
| §2.5 日志追加（`save::append`、`rolls::append`、`player::append_*`） | 随调用方 | — | 随调用方 |
| §2.6 `identify` | `talkdnd:identify` | `AdoptedWrite` | 是（已接入 SAVE 锁） |
| §2.6 `reveal add` | `talkdnd:reveal-add` | `AdoptedWrite` | 是（已接入 SAVE 锁） |
| §2.6 `original add` / `original plan` | `talkdnd:original-add` / `talkdnd:original-plan` | `AdoptedWrite` | 是（已接入 SAVE 锁） |
| §2.6 `entity add/rename/retire` | `talkdnd:entity-write` | `AdoptedWrite` | 是（已接入 SAVE 锁） |
| §2.6 `relation add/end` | `talkdnd:relation-write` | `AdoptedWrite` | 是（已接入 SAVE 锁） |
| §2.6 `time enable/disable/set/advance/event add/done/skip` | `talkdnd:time-write` | `AdoptedWrite` | 是（已接入 SAVE 锁） |
| §2.6 `memory add/remove` | `talkdnd:memory-write` | `AdoptedWrite` | 是（已接入 SAVE 锁） |
| §2.6 `settings dice on/off` | `talkdnd:settings-dice` | `AdoptedWrite` | 是（已接入 SAVE 锁） |
| §2.6 `settings style set` | `talkdnd:settings-style-write` | `AdoptedWrite` | 是（已接入 SAVE 锁） |
| §2.6 `settings automation set/reset` | `talkdnd:settings-automation-write` | `AdoptedWrite` | 是（已接入 SAVE 锁） |
| §2.6 `secret add/remove` | `talkdnd:secret-write` | `AdoptedWrite` | 是（已接入 SAVE 锁） |
| §2.6 `secret import --apply` | `talkdnd:secret-import-apply` | `AdoptedWrite` | 是（已接入 SAVE 锁） |
| §2.6 `handout show` | `talkdnd:handout-show` | `AdoptedWrite` | 是（已接入 SAVE 锁） |
| §2.6/§2.7 `handout list --dm` | `talkdnd:handout-list-dm` | `UnadoptedWrite` | 否（`--dm` 会 build/store 书目录 `handouts.yaml`） |
| §2.6 `adventure section ... --reason` | `talkdnd:adventure-section-write` | `AdoptedWrite` | 是（已接入 SAVE 锁） |
| §2.6 `adventure index build` | `talkdnd:adventure-index-build` | `AdoptedWrite` | 是（已接入 SAVE 锁；目标在存档内时取该存档的锁） |
| §2.6 `new` | `talkdnd:new` | `NewSave` | 不适用（只创建新槽） |
| §2.7 `library tidy/merge/scan/packs/choose` | `talkdnd:library` | `ReadOnly` | 不适用（只写书库，不写存档） |
| §2.7 `data import` | `talkdnd:data-import` | `ReadOnly` | 不适用（只写 `Rules/chargen/local/`） |
| §2.7 `handout build/mark` | `talkdnd:handout-build` / `talkdnd:handout-mark` | `UnadoptedWrite` | 否（保守：书目录可能在存档内） |
| §2.7 `update::sync_defaults`（随 `new`） | 归入 `talkdnd:new` | — | 不适用 |
| §2.7 `content import-srd` / `import-srd-prose` / `import-srd-pack` | `combat-sim:content-import-srd` / `:content-import-srd-prose` / `:content-import-srd-pack` | `AdoptedWrite` | 是（`--out` 在协议存档内时取 SAVE 锁；存档外允许） |
| §2.7 `content pack build/release` | `combat-sim:pack-build` / `:pack-release` | `AdoptedWrite` | 是（`--out` 在协议存档内时取 SAVE 锁；存档外允许） |
| §2.7 `content pack build/release --dry-run` | `combat-sim:pack-build-dry-run` / `:pack-release-dry-run` | `ReadOnly` | 不适用（不写文件） |
| §2.8 `guard` | `talkdnd:guard` | `ControlMetadata` | 不适用（只写缓存） |
| §2.8 `secret list` | `talkdnd:secret-list` | `ControlMetadata` | 不适用（只写缓存） |
| §2.8 `start` 的 `digest.md`/`.cache` | 归入 `talkdnd:start`（整条命令按写者处理，因为它还会迁移存档内容；控制元数据分类只说明这些文件不单独取 SAVE 锁、将来不并入备份） | — | 随 `talkdnd:start` 接入；锁不可用时不写 digest/缓存 |
| §2.9 未来目标（sidecar、schema 11、候选树、D 备份） | 无条目 | — | 仍不存在，不声称已覆盖 |

**只读条目（同样逐条列入注册表并由测试覆盖）：** `talkdnd:xp-show`、`talkdnd:loot-show`、`talkdnd:memory-list`、`talkdnd:entity-read`、`talkdnd:relation-read`、`talkdnd:time-read`、`talkdnd:settings-read`、`talkdnd:settings-style-read`、`talkdnd:checkpoint-read`、`talkdnd:snapshot-diff`、`talkdnd:migrate-dry-run`、`talkdnd:secret-scan`、`talkdnd:secret-import-dry-run`、`talkdnd:handout-list`（player 视图）、`talkdnd:original-read`、`talkdnd:reveal-read`、`talkdnd:chargen-status`、`talkdnd:chargen-validate`、`talkdnd:char-spells-read`、`talkdnd:adventure-search`、`talkdnd:adventure-index-read`、`talkdnd:adventure-section-read`、`talkdnd:known`、`talkdnd:knowledge`、`talkdnd:oracle`、`talkdnd:options`、`talkdnd:data-status`、`talkdnd:library`、`talkdnd:log-rolls`，以及 `combat-sim:status`、`combat-sim:scene`、`combat-sim:log`、`combat-sim:simulate`、`combat-sim:content-read`、`combat-sim:pack-read`、`combat-sim:party-show`。

**证据与已知限制：**

- 两套 CLI 的真实调用路径测试逐条覆盖上表：写入口在 lock/state 存在时退出 1、文案齐全、存档树逐字节不变；只读入口不被拒绝且整树不变；缓存入口只改缓存；无控制文件时既有写命令照常工作；`new` 在协议存在时仍可创建新槽；`start` 整条被拒绝（不跑 digest/缓存）。
- `handout list --dm` 是写者（registry 缺失时 build/store `handouts.yaml`），player 视图只读；`handout show` 与 `list --dm` 的 `--book` 书目录在查找失败前也纳入检查。
- 最终符号链接用例：受保护存档内 `combat/e.state.json` 指向外部有效 state 时，用未受保护 `--save` 的写命令仍被拒绝；`<save>/rolls.log` 与 `pack release` 目标文件链接到受保护文件时同样拒绝，链接与受保护文件内容不变。
- `new` 只审计实际创建的存档：更晚日期的旧存档（缺/有 `integrity.yaml`）在新存档创建前后逐字节不变。
- 跨存档与路径解析另有独立用例：`--book` 指向另一存档内的书目录（绝对路径与 root 相对路径）时 `adventure index build` 与 `handout build/mark` 均退出 1 且目标存档不变；存档外的书目录不被拒绝；`adventure section --list --reason` 成功且整树不变；`combat-sim` 的相对 `--state`、经符号链接的 `--state`、外部 manifest 的 `state_dir`、外部 state 记录的 `party_state` 都按实际写目标拒绝，且不因 `--save` 指向未受保护目录而放行；外部 `party_state` 的目标在存档外时保持原行为。
- 少量只读入口在合成存档上会因缺少书籍内容或施法职业而自行报错（`snapshot diff`、`adventure search`、`adventure index show`、`char <who> spells`、`data import`、`combat-sim simulate`、两个 pack `--dry-run`）。测试断言的是“门没有拒绝且存档不变”，不是这些命令一定成功；它们各自的成功条件不属于本块。
- `--help` 声明覆盖顶层命令与每个写叶子（两套 CLI 各有一张显式清单，测试逐条断言）。`talkdnd` 的 `checkpoint`/`memory`/`entity`/`relation`/`time`/`char`/`adventure index` 等叶子枚举现在也带声明；`diff.rs` 的唯一子命令是只读，不需要声明。
- `tools/advancement-lock` 已加入 `.github/workflows/ci.yml` 的 fmt、clippy 与三平台 test 矩阵；`docs/versioning.md` 说明它是 `publish = false` 的内部库、跟随产品版本、不产生发布产物。
- 门只约束遵守协议的入口；旧二进制、外部编辑器与不遵守协议的进程仍不受约束（#264 未解决，结论不变）。写目标解析是对“检查后、写入前”这一窗口的尽力而为：在检查之后才创建的符号链接仍无法被任何写前检查看见。

## 6. 接入语义（已由决定记录 #292 决定）

以下问题已由决定记录 #292（2026-11-17）确认；逐项映射见 `docs/ADVANCEMENT-PHASE-1-PLAN.md` 的「接入语义」表，`ADV-P1-1pre` 已按这些决定实现过渡门。这里只保留结论索引：

1. **未初始化存档**：显式 `init`，不自动初始化；没有控制文件的存档保持既有行为。
2. **拒绝 UX**：退出码 1 + 明确文案，在受影响命令的 `--help` 与玩家文档中声明；不得只警告后继续写。
3. **CLI 兼容**：`new`/`start`/`migrate`/`chargen` 不自动 `init`；旧存档首次运行不强制 `init`。
4. **缓存写入**：控制元数据，原子写、不进用户数据备份、不取 SAVE 锁。
5. **`combat-sim` 锁实现**：共享 crate `tools/advancement-lock`；`combat-sim` 不依赖 `talkdnd`。
6. **日志并发语义**：活动操作期间独立 `log`/`checkpoint <text>` 失败关闭；只读日志查询不受影响。
7. **`new` 与 SAVE 的关系**：创建新槽时不 `init`，也不写已启用协议的存档。
8. **控制元数据范围**：保留策略只记录、不实现自动清理；未定前不得假定可删除或可排除。

## 7. 验证与限制

- 本文是**静态盘点**：未运行产品测试、未做并发实验、未读取真实存档/书籍/秘密。所有“未接入”行为来自源码阅读，不是实测。
- 行号绑定基线 `63cf4fe9317f1385317ab606f37a7a0c23550f76`；后续提交移动代码时以函数名与路径为准，行号只作定位辅助。
- 本任务真实执行的检查见 Issue #290 / PR 的证据矩阵；本文不把任何未运行项写成通过。
- `#264` 结论不变：checkpoint 锁首次 `WouldBlock` 根因仍 UNKNOWN；现有锁不是内容保护，也不是跨主机/网络文件系统互斥。
