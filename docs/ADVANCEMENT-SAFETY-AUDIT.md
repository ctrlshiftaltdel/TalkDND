# 单职业成长：安全与接口事实审计

基线 `bef014e9bc0b3f9c1d88e014c732a8c04a5e9127`；以下为对公开跟踪代码与测试的静态核查，不包含玩家数据、私有书籍或规则正文。位置均相对于仓库根目录。UNKNOWN 表示本次未证实，而不是不存在或安全。测试名是可复核入口，不表示本次运行了该测试。

所有者已批准未来 schema 11、9/10 自动迁移、迁前完整备份、保留手改与历史未知；完整备份和持久操作标识/阶段记录，失败停止并显式恢复；仅可靠数值自动更新；活动遭遇可预览不可提交（设计 `docs/superpowers/specs/2026-11-10-character-advancement-design.md` §3–5）。**这些是未来目标，不是现有实现或本次新决定**。遭遇识别算法、数据授权接入和恢复细节仍未定；本审计不提出新的授权或产品决定。

## 升级写入与边界

目前 `tools/talkdnd/src/lib.rs::run` 先对大多数命令捕获完整性快照，再 `dispatch`；即使 dispatch 返回错误也会对存在的存档调用 `audit::after_command`，最后返回原结果（若审计先出错则先返回审计错误）。`dispatch` 的 `Cmd::LevelUp` 只解析存档、将 `all`/逗号名单逐人顺序传入 `progress::level_up`；某人失败会中止后续人，先前人的写入不会自动回滚。该入口未调用 `migrate::ensure`。这是已核实的特定入口，不证明其他所有入口行为相同。

`tools/talkdnd/src/progress.rs::level_up` 的顺序：读取 Markdown 角色 combat 块、进度、规则版本与职业；若存在结构化 `sheets/<id>.yaml` 则载入结构化角色；校验等级、XP/里程碑及生命骰。`--hp roll` 在后续解析和持久化角色前调用 `rolls::append` 追加 `rolls.log`；平均值或指定值不追加该骰记录。之后生成新 combat 块和提示。结构化路径调用 `charsheet::party`、`charsheet::commit`，依次 `charsheet::store` 写 YAML、若 combat 目录存在则 `PartyState::save` 写 party state、渲染并原子写 Markdown（`charsheet.rs::commit`、`party.rs::save`）；party 保存还自行更新完整性。旧 Markdown 路径只原子写该 Markdown。里程碑路径随后 `Progress::store` 写进度；两种路径最后在存在最新 session 时追加升级日志（`progress.rs::log`、`save.rs::append`）。外层 `audit::after_command` 最后执行。`rolls.rs::append` 和 session 日志是追加，不应当作原子替换。`charsheet::party` 从既有 party state 加载或构造成员；不可据此假设活动遭遇状态已同步。

以上调用可能中途失败，甚至在最终返回错误时已有骰子、YAML、party、Markdown、进度或日志写入；每文件的 `fsutil::atomic_write` 不是跨文件事务，完整性更新本身也可能失败。升级的 ASI 提示仅检查职业显式 `asi`；`classdata.rs::generate` 对缺省列表采用共同默认值（约 1004 行），两者并非同一资格判定接口。当前没有本设计所述的选择/校验/提交预览流程；其他潜在调用者及所有失败点覆盖 UNKNOWN。

## 现有 schema 与入口

- `migrate.rs::CURRENT_SCHEMA` 为 10；`load` 拒绝大于 10 的主 schema；`detect` 调用它，无元数据时按有无缺结构化文件的 Markdown 角色推断 schema 1 或 2。`ensure` 经 `detect`，对当前版本不迁移；9→10 由 `plan_spells_10`/`apply_spells_10` 处理结构化法术列表，先记录 dirty 文件、备份，再按计划写、写 meta、追加可用 session 日志和更新完整性。无完整性基线时所有结构化 sheet 对该步视为 dirty；已有的 dirty sheet 不做 serde 重写，避免丢注释或未知字段（`migrate.rs::integrity_dirty`、`ensure`、`sheet_yaml_paths`）。旧 Markdown 与缺结构化角色不等于自动获知历史选择；`tools/talkdnd/tests/spells.rs` 的 `migration_skips_dirty_sheets_in_both_shapes`、`migration_without_integrity_manifest_keeps_every_sheet_byte_identical`、`migration_backup_restores_a_schema_9_save_that_migrates_again` 是现有合成测试入口。
- `lib.rs::Cmd::Migrate` 非 dry-run 时先 `save::migrate_ruleset`，再 `ensure`；不能把整个命令无条件描述为先备份后任何写入。`start.rs::digest` 也先调用 `migrate_ruleset` 后调用 `ensure`。未来 schema 在到达 `migrate::load` 的路径拒绝；`Cmd::LevelUp` 路由未直接调用 `ensure`，且 `progress::level_up` 本体所示路径未调用它。必须逐入口、逐旧发布二进制验证；完整旧版本入口清单及其拒绝行为 UNKNOWN，迁移后不能依赖旧程序自动保护。
- `charsheet.rs::parse_char_file`、`progress.rs::Progress::load`、`party.rs::PartyState::load` 分别检查它们自己的文件版本；这不等于主存档 schema 门禁。未来 11 的迁移、完整备份与旧版兼容均尚未实现或验证。

## 备份、锁与恢复比较

`migrate.rs::backup_save` 使用时间戳和 `exists` 递增名称，`copy_tree` 递归复制普通文件/目录，跳过名为 `_backups`、`.cache`、`.talkdnd` 的项；对既非普通文件也非目录的条目没有报错分支，故特殊文件可被忽略。名称存在检查和复制并非原子分配；本次未证明并发、崩溃、元数据或全部文件可恢复。`ensure` 路径内未见 checkpoint 的锁或阶段恢复，不应把备份称作已验证的完整事务。

`checkpoint.rs::restore_inner` 要求确认，先取得 checkpoint `Lock`、验证 checkpoint 和 schema，再以 `allocate_restore_backup` 的 `create_dir` 分配新目录并复制。`copy_tree` 跳过 checkpoint 锁和 `.cache`，拒绝符号链接及其他无法复制的特殊项；因此其“整份”也有明确排除。随后逐文件暂存、逐文件 rename；提交阶段失败尝试从本次备份镜像回滚并报告残留/回滚失败；成功替换后删除覆盖范围内多余文件，再按需迁移、追加日志和完整性重定基，后置阶段失败报告而不自动回滚（`restore_inner`、`post_commit_failure`）。`Lock::acquire` 以 `create_new` 协调 checkpoint 操作并可移除超时锁，不能据此声称所有普通升级/外部编辑均被锁住。checkpoint 校验与恢复重定基、完整性自身锁（`restore_rebaseline`）也不能证明未来升级有跨进程冲突保护。锁覆盖、备份期间非协作写入的完整一致性及恢复覆盖后续手改的保护 UNKNOWN。

现有合成故障入口见 `tools/talkdnd/tests/checkpoint.rs`：`restore_refuses_entries_the_whole_save_backup_cannot_copy`、`migration_failure_after_commit_names_the_stage_checkpoint_and_backup`、`rebaseline_failure_after_commit_names_the_stage_checkpoint_and_backup`、`restore_rebaseline_keeps_a_concurrent_writers_fingerprints` 和 `same_second_restore_rolls_back_to_this_operation_not_the_earlier_backup`。它们针对 checkpoint，不证明成长恢复安全。未来合成故障注入至少覆盖备份失败/特殊文件、每次写入前后中断或磁盘满、锁竞争与过期、外部手改、重复请求和恢复重试、日志及完整性更新失败；核对保留原字节、阶段与恢复位置、冲突拒绝及不重复奖励。具体协议仍待设计及审查。

## 遭遇状态生命周期

`tools/combat-sim/src/cli.rs::new_state` 以内容构建 `EncounterState::from_encounter`，仅当路径位于 manifest state_dir 才读取并关联 party state，然后 `EncounterState::save`；`resolve_state_path` 支持命名状态及显式路径，无名默认路径不等于枚举全部状态。`run_state_command` 先 load、执行、更新内存日志、保存状态、写 party state，再追加 combat 日志；它们也非跨文件事务。`state.rs::EncounterState` 包含可选 `outcome`、round、initiative 和内容快照；`engine.rs::log_outcome` 记录战斗结果，`init_with_modes` 可将 outcome 重置。结果保存后状态文件仍可存在，故文件存在不能证明活动；outcome 缺省也不能单独证明应阻止升级。

`EncounterState::load` 对读取失败、JSON 解析失败、状态 schema 不匹配及空间校验失败返回错误；`save` 原子写单文件并对存档所属状态更新完整性。缺失：某一指定路径 load 失败，但是否另有状态未知；损坏/不支持版本：不能从错误推出没有活动战斗；多个命名状态、显式路径及存档外 scratch 状态如何可靠发现与归属 UNKNOWN；结束状态可保留且 outcome 可重置。未发现已批准的全局活动判定算法，也不将这些事实当作算法决定。未来测试须覆盖活动可预览且提交拒绝、完成状态、缺失、损坏、多个状态、外部路径、并发变化以及预览到提交间状态变化；对无法确定的处理需设计及授权，不能猜测安全。

**静态推断，未实测**：`engine.rs::combat_outcome` 依据 `CombatantState::is_active`（非 dormant、HP 大于零且未死亡）按阵营计算结果；`log_outcome` 才将结果写入 `state.outcome`。`cli.rs` 的 `Command::Heal` 通过 `run_state_command` 调用 `engine::heal`；`heal` 更新 HP，符合条件时调用 `revive_from_zero`，两者本身均不调用 `log_outcome`，而该命令包装器随后直接保存状态。因此，原先记录非空 outcome 的遭遇若治疗使另一阵营的一名非死亡、非 dormant 成员从零 HP 恢复，保存的 outcome **可能**仍是旧结果，即使重新计算的结果不同。待运行的合成场景：构造两阵营、其中一方仅存非死亡的零 HP 成员且已记录胜方，heal 后保存并重新加载，比较持久化 outcome 与重新计算值；另覆盖未改变活动成员的治疗及后续刷新。此处不声称已运行该场景，也不以非空 outcome 单独决定结束或选择活动识别算法。

## 内容接口与授权事实

`tools/talkdnd/data/classes-2014.yaml` 与 `classes-2024.yaml` 提供职业特性、可选 `asi` 等数据；`classdata.rs::Book::srd` 选择嵌入版别，`Book::load` 再按规则集筛选并合并本地 chargen 数据。它不是 `combat_sim::content::ContentRegistry`，不能假定专长资格已由该 Book 载入。

公开跟踪的 SRD 专长包分别位于 `LetsPlay/Rules/combat/2014/srd/srd-5.1-feats.pack.json` 与 `LetsPlay/Rules/combat/2024/srd/srd-5.2.1-feats.pack.json`。`content/schema.rs::DomainEntity` 有 id、source、section、tags、text、refs 和开放 `data` 等字段；`srd_pack.rs::import_feats` 导入原始前提、重复等数据并生成通用实体，字段存在不证明完整、规范化、可执行或已接入升级。

FULL 必须区分三个事实，不能互相替代：

1. **包格式/版本合格**：`tools/combat-sim/src/content/loader.rs::ContentSources::load` 在有 campaign 的严格 FULL 路径检查包的 distribution、edition，`apply_pack` 检查合法 full 书型 ID 等，并要求加载至少一个匹配的 FULL 包；按 SRD、FULL、手写、冒险/角色顺序加载。它处理的是输入源和包元数据，不查询 talkdnd 的玩家书籍识别记录，非严格调用路径也不同。合格包不证明玩家解锁。
2. **玩家解锁状态**：`tools/talkdnd/src/library.rs::pack_statuses` 读取本机书籍识别记录并扫描 `Rules/packs`；拒绝重复 ID、错误元数据和内容包解析失败，匹配 edition 与书型的 `Identified` 记录加有效包才记 `Unlocked`。缺书、缺包、错误版、识别不明或无效包各有状态；`require_full_packs` 对 FULL 要求当前版至少一个 Unlocked。此检查是 talkdnd 的本机状态判定，不是 loader 自行验证玩家解锁，也不是对私有内容覆盖或许可证的外部证明。
3. **manifest 接入**：`start.rs::digest` 先扫描并登记，再调用 `require_full_packs`，扫描成功后才尝试 `sync_campaign_packs`；同步只为 FULL 添加本版 Unlocked 的受管理包，对 SRD 移除受管理 FULL 包并保留非受管理项。扫描失败时不调用同步，且同步失败只记录警告、保留原有内容源；`sync_campaign_packs` 仅在找到可替换的 `library:` 行时重写。因此包合格、判定 Unlocked、实际列入 manifest 是不同结果，不能从任一单独推断另两项或声称所有入口都经 start。

`tools/combat-sim/tests/ruleset_options.rs::full_values_load_their_edition_pack` 直接生成合成 FULL 包并放入 manifest 后调用 loader；它可作为加载/版本行为入口，**不能证明玩家解锁已检查**。`full_without_an_unlocked_pack_fails_and_names_the_book_and_fix` 的名称含 unlocked，但其 fixture 没有 FULL 包，证明范围仍是 loader 缺包错误；`a_full_save_never_loads_the_other_edition_pack` 测错误版本。另有 `tools/talkdnd/tests/book_detection.rs::the_save_manifest_loads_only_the_unlocked_full_pack`，通过合成书籍识别、start 和 loader 串联检查；以上均为静态核查测试入口，本次未运行。`full_pack.rs` 构建测试亦非私有内容覆盖证明。FULL 私有内容覆盖、各专长前提/效果完整性及成长适配均 UNKNOWN；严守既有 SRD/FULL 与版别边界，不通过手动登记绕过已知非法或授权限制。

未来集成核对：用合成数据验证两版职业机会与 SRD/FULL 隔离、错误版本和缺包拒绝、候选身份及同名覆盖、已知非法优先于未知、DM 确认与手动效果不冒充自动实现，并检验角色、party、法术、进度、日志和完整性接口。具体规则表达与授权变更须交 Master 和所有者，不能从此盘点自行决定。

未来授权接口测试另需分别构造：有效 FULL 包但无识别记录、识别成功但缺包、错误版、模糊记录、重复 ID/无效包、匹配记录与包解锁；检查 `require_full_packs` 和 `sync_campaign_packs` 对 FULL/SRD 的区别及非受管理项保留；分别验证直接 loader 输入、start 成功扫描、扫描或同步失败和 manifest 缺少可替换项的实际边界。仅用隔离的合成记录、包与目录，不读取玩家书籍；这些是待运行建议，不改变授权机制。
