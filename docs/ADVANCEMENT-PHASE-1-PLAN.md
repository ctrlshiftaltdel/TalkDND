# 首阶段剩余实施计划（单职业成长与专长）

> 本文件是**阶段计划**，只记录阶段目标、任务顺序、依赖与出口条件。
> **任务状态只在 GitHub Issues**：本文件不建第二套状态台账，不记录任务状态、负责人、完成百分比、日期或排期。要判断“做到哪一步”，只看 Issue 的状态、证据矩阵与 PR / CI；本文件只回答“还剩哪些可独立验收的块、按什么顺序、依赖谁、出口条件是什么”。
> 设计目标权威是 [docs/PLAN.md](PLAN.md)；产品决策与活章程是 [docs/PROJECT-CHARTER.md](PROJECT-CHARTER.md)；任务包字段、状态取值、合并门与证据矩阵只在 [kit/orchestration.md](../kit/orchestration.md) 定义，本文件引用不重述。
> 与 `docs/PLAN.md` 的设计目标冲突时，停下对齐，以设计目标为准，不在两处各写一套。

## 计划头部

- 计划名称：首阶段剩余实施计划（单职业成长与专长，目标 schema 11）
- 设计目标来源：[docs/PLAN.md](PLAN.md)；书面设计 [2026-11-10-character-advancement-design.md](superpowers/specs/2026-11-10-character-advancement-design.md)；契约与事实依据 [ADVANCEMENT-IDENTITY-CONTRACT.md](ADVANCEMENT-IDENTITY-CONTRACT.md)、[ADVANCEMENT-LOCK-DESIGN.md](ADVANCEMENT-LOCK-DESIGN.md)、[ADVANCEMENT-WRITER-INVENTORY.md](ADVANCEMENT-WRITER-INVENTORY.md)、[ADVANCEMENT-RECOVERY-OPTIONS.md](ADVANCEMENT-RECOVERY-OPTIONS.md)、[ADVANCEMENT-SAFETY-AUDIT.md](ADVANCEMENT-SAFETY-AUDIT.md)、[CHECKPOINT-LOCK-INVESTIGATION.md](CHECKPOINT-LOCK-INVESTIGATION.md)
- 当前阶段：`Phase 1 — 写者覆盖与 D 前置`（入口条件见「当前状态」；任务状态见 Issue）
- 成立日期：本文件首个提交（见 Git 历史；本文件不记录任务日期或排期）
- 建立依据：`docs/PROJECT.md` 与 `docs/PLAN.md` 都规定阶段计划只在 Master 明确决定时建立；本次由 Master 在 Issue #288 决定。本文件因此不是第二套任务清单：它不承载状态、不替代 Issue。

## 当前状态

**本文件不承载任务状态。** 下面只写三件事：写本计划时的系统入口事实、已交付的前置引用、以及已被批准但尚未实现的边界。任何一条都不得被读成“已完成”。

### 入口事实（写计划时的静态核对；每块实现前必须重新核对代码与 Issue）

- 主存档 schema 为 **10**（`tools/talkdnd/src/migrate.rs::CURRENT_SCHEMA`）；schema 11、9/10→11 自动迁移与选择 sidecar 均**未实现**。
- **现有旧升级命令仍然存在并会写升级**：`progress::level_up` 走结构化与 Markdown 两条写路径，`--hp roll` 还会向 `rolls.log` 追加骰记录。它是既有功能，不属于“未实现的成长提交”，也不得被删除或静默改变。
- **新的属性提升/专长选择与 D 协议提交入口未接入**：`tools/talkdnd/src/advancement_lock.rs` 已存在并注册，但只有 `checkpoint.rs` 与 `migrate.rs` 引用其控制文件名常量，**没有任何提交/预览调用方**。
- 只读预览、选择与校验、候选树、备份验证、逐文件发布、日志/完整性幂等、显式恢复、sidecar 入籍、9/10→11 迁移**均未实现**。
- 因此本文件列出的每个块都尚未开始；其中涉及运行、并发、故障、跨平台的结论按实际情况标 `NOT_RUN` / `UNKNOWN`，不得写成通过。

### 前置已交付项（只引用，不重复其验收细节；状态以 Issue 为准）

| 交付 | Issue | 本计划如何使用 |
|---|---|---|
| 成长身份只读完整发现与诊断 | #275 | 提供只读发现/歧义的既有边界；后续块不得把“诊断能跑”当成身份已验证 |
| 成长协作锁域与恢复协议设计 | #277 | 提供锁域、顺序、恢复与故障矩阵的设计权威（见 [锁设计](ADVANCEMENT-LOCK-DESIGN.md)） |
| 成长协作锁 v1 库与四项决策 | #282 | 提供 `advancement_lock.rs` 原语与 OD-1/2/7/9；**未接线**，不构成“提交可用” |
| 旧 checkpoint 锁归属/抢占与备份控制文件修正 | #286 | 修正 `Drop` 归属、取消 mtime 抢占、备份/回滚排除控制文件；**不修复 #264** |
| 其余只读命令的完整性审计副作用修复 | #281、#279 | 提供“只读路径不写存档”的既有结论；是写者盘点的输入 |

另有两个未闭合的跟踪项，不是本计划的前置交付：**#285**（剩余只读命令的完整性审计副作用，仍跟踪）是写者盘点的输入，其中每个入口要么在 `ADV-P1-1a`–`ADV-P1-1e` 内接入、要么声明只读，两者任一未闭合时对应子块的 AC 不能判定通过；**#264**（checkpoint 锁测试首次 `WouldBlock` 根因）仍未解决，见 `ADV-P1-12`。

### 已批准边界（本计划只承接，不缩窄、不新增）

1. 主存档 **schema 11**，**自动迁移 9/10→11**；迁移前**完整备份**，且完整备份**包含玩家既有用户备份**。
2. 控制元数据（锁文件、操作前快照、恢复工作记录）**单独保留**，不并入用户数据的完整备份；保留期与清理责任仍未定，未定前不得假定可删除或可排除。
3. 仅支持**本地常见文件系统**；网络盘、FUSE、同步目录不支持，且**强制准入门未实现**——不得声称这些位置已被拒绝，跨主机/网络互斥仍 `UNKNOWN`。
4. **所有相关新版本写者被覆盖之前，新的成长提交入口（属性提升/专长选择 + D 协议）不启用**；未接入的入口必须在同一存档上拒绝写入或声明只读（OD-2）。只读预览/诊断可以先上线。现有旧升级命令属于既有功能，必须被盘点并纳入保护，不得删除或静默改变。
5. **Windows 本地提交不支持**（平台无目录 `fsync`：`init` 返回 `state_published_not_durable`，可写谓词与提交一律拒绝）；不得把“`init` 建了文件”当成可提交。
6. **新成长路径的内层锁有界获取**（OD-9）：默认 5 秒 deadline、可显式覆盖、超时失败关闭；**现有完整性锁保持无限阻塞语义不变**，因此整条提交路径的总等待**不承诺有界**，T6b 只就新路径的有界获取取证。
7. **#264 未解决**；现有 checkpoint 锁不是 OS 锁，旧二进制、外部编辑器与不遵守协议的进程不受约束。
8. **不承诺多文件原子性**；不承诺检测共同复制“角色 + sidecar”或整档重放/回滚，也不承诺识别保留标识的整盘镜像。
9. **预览只读**；**活动遭遇只预览、不提交**；损坏、歧义或无法可靠判断的遭遇同样拒绝提交。
10. **仅自动更新来源与派生关系可可靠确定的数值**；不确定的保留并提示 DM，不覆盖手改、不静默重生成。
11. 首阶段全部验收后**暂停**，不进入全量专长自动化。

### 本计划不决定的事（命中即按决策门停下）

具体身份方案（sidecar 字段、随机源、令牌生命周期、旧格式无损写入）与重命名/重绑/克隆/回滚协议；活动遭遇识别算法；控制元数据的保留期与清理责任；网络盘/FUSE/同步目录的强制准入门；#264 根因；跨平台文件系统与持久化语义的结论范围；各版机会、属性上限、前提与重复的规则表达；内容授权接入细节。这些都不是本计划可以顺手拍板的工程细节。

## 阶段

阶段只写“结束时系统具备什么”和可核验的出口条件；不写活动清单，也不写日期。

**并行调查轨**：`ADV-P1-12`（#264 现场采集）不阻塞任何块，也不得替代任何块；`ADV-P1-13`（跨平台/网络文件系统调查）的结论是 `ADV-P1-6` 的前置门——结论为 `UNKNOWN` 时阻塞对应实现，不能因为它是“并行”任务就视为已满足。

### Phase 0 — 已交付基础设施（引用）

- 阶段目标：只读身份诊断、锁域/恢复设计、锁 v1 原语、旧锁行为修正与只读路径修复可查、可复核。
- 交付：上表五个前置 Issue 的已验收产物。
- 出口条件：相关 Issue 在 GitHub 上为 `VERIFIED`，且其证据矩阵绑定各自 head；本计划不重复判定。

### Phase 1 — 写者覆盖与 D 前置

- 阶段目标：任何相关新版本写入入口（含既有旧升级写者）都不再能“静默写入且不持锁”；D 流程能走到“候选已验证、未发布”。
- 交付：`ADV-P1-1`（盘点）、`ADV-P1-1pre`（跨 CLI 过渡拒绝门）、`ADV-P1-1a` … `ADV-P1-1e`（按组接入）、`ADV-P1-2`。
- 出口条件（全部可核验）：① 写入入口清单含旧 `progress::level_up` 的结构化与 Markdown 两条路径，逐项标注持锁/拒绝/只读；② 不存在未接入却仍静默写入的入口；③ **新的**属性提升/专长选择与 D 协议提交入口仍未启用，**现有**旧升级功能仍可用且回归测试未被削弱；④ D 阶段机在合成 fixture 上完成 `admit → backup → candidate`，失败时目标文件字节不变；⑤ 顺序反转用例不死锁；⑥ 两套 CLI 共享 `ADV-P1-1pre` 过渡门：advancement lock/state 存在时未接入写入口失败关闭，只读入口保持，写者注册表覆盖盘点 §2。

### Phase 2 — 有界内层锁、发布/日志/幂等与显式恢复

- 阶段目标：新路径的内层锁按批准语义有界获取；发布、日志/完整性、幂等与显式恢复在合成故障注入下符合已批准边界。
- 交付：`ADV-P1-3`、`ADV-P1-4`、`ADV-P1-5`。
- 出口条件：① 新路径默认 5 秒 deadline、可覆盖、超时失败关闭，T6b 在新路径上取证；② 旧完整性锁仍无限阻塞，既有调用方与测试语义不变；③ 部分发布只能落 `in_doubt`、零写入才可 `failed`、`complete` 只在全部比对后写（`ADV-P1-4`）；④ 重复提交请求不重复增加属性、XP、资源或日志（`ADV-P1-4`）；⑤ 恢复重试不重复应用（`ADV-P1-5`）；⑥ 恢复前先建立工作证据并保护当前树，由人显式选择，无法判定即停止（`ADV-P1-5`）。

### Phase 3 — sidecar 入籍与 9/10→11 迁移

- 阶段目标：在合成 fixture 上完成 schema 11 的选择 sidecar 入籍与 9/10→11 自动迁移，且手改与未知历史不被覆盖。
- 交付：`ADV-P1-6`。
- 前置门：具体身份方案已获所有者授权；`ADV-P1-13` 的跨平台枚举/链接/备份调查结论支持实现（`UNKNOWN` 时阻塞对应实现）。
- 出口条件：① 迁移预览不写；② 完整备份（含既有用户备份）验证通过后才写，失败时目标未改；③ 身份缺失/不一致/同档重复一律停止，不按名称或旧 ID 自动绑定；④ 手改、未知键、注释与历史未知按原字节保留；⑤ 真实存档迁移与跨平台结论按实际执行标 `NOT_RUN` / `UNKNOWN`。

### Phase 4 — 只读预览、玩家显式选择与校验、可靠派生数值

- 阶段目标：预览只读；玩家显式选择本次机会；每个资格条件可判为满足/不满足/未知；派生重算只自动更新可靠值。
- 交付：`ADV-P1-7`、`ADV-P1-8`、`ADV-P1-9`。
- 出口条件：① 预览不创建、不锁定、不写任何文件，并输出 `consistency: unverified` 与 `write_capability`；② 每个条件有“满足/不满足/未知”+ 来源 + 规则版别，已知非法与未知并存时拒绝；③ 未知只在 DM 明确确认且依据非空可审计时继续，并标“未经程序验证”；④ 玩家显式选择存在才可继续，缺失即拒绝，DM 确认不替代玩家选择；⑤ 可可靠派生的数值恰好更新一次，不可靠值保留并提示 DM，不覆盖手改。

### Phase 5 — 提交接线

- 阶段目标：一次玩家显式选择在重新验证绑定、条件与派生边界后经 D 协议恰好提交一次。
- 交付：`ADV-P1-10`。
- 出口条件：① 提交重验绑定、条件与派生边界，任一不一致即拒绝并要求重新预览；② 经 D 协议一次应用，选择 sidecar 与主 schema 元数据目标纳入发布与显式恢复；③ 活动遭遇只预览——**遭遇识别算法未决前该条不能判定通过**，不得自行发明算法；④ 入口只在 `ADV-P1-1a`–`ADV-P1-1e`、`ADV-P1-3`、`ADV-P1-6`、`ADV-P1-9` 就绪后启用。

### Phase 6 — 端到端、跨平台与故障注入收口

- 阶段目标：端到端、跨平台与故障注入有实测证据；调查轨的记录按环境与范围如实归档。
- 交付：`ADV-P1-11`（并汇入 `ADV-P1-12`、`ADV-P1-13` 的记录）。
- 出口条件：① 三平台 CI 的合成套件结果按平台分别记录；② 故障注入矩阵逐项有 `PASS` / `FAIL` / `NOT_RUN`，没有把未跑折算为通过；③ 端到端合成与四规则集隔离有证据；④ 调查轨的未运行项标 `NOT_RUN`、无法判定标 `UNKNOWN`，已运行项绑定环境/范围/head 记录，且没有据此扩展支持范围或承诺跨主机安全。

## 任务块

每个块只写一个可独立验收的结果。块 ID 是稳定标识，用于后续 Issue 与依赖引用；**状态不写在这里**。

顺序与依赖：`1 → 1pre → 1a → 1b → 1c → 1d → 1e → 2 → 3 → 4 → 5 → 6 → 7 → 8 → 9 → 10 → 11`；`1` 是盘点，`1pre` 是跨 CLI 过渡拒绝门，`1a`–`1e` 按入口组接入；`12`、`13` 为并行调查轨，`13` 的结论是 `6` 的前置门。

### Task: `ADV-P1-1` — 新版本写者盘点（OD-2 第一步，本任务交付）

- **Goal:** 静态盘点所有会写存档的新版本入口，逐项标注命令/函数定位、写入对象、现有锁、与成长提交的并发关系、建议接入方式与未接入行为；把“确认只读”与“会写”分开，并据此给出按组接入的子块。
- **建议允许路径：** `docs/ADVANCEMENT-WRITER-INVENTORY.md`（新增）、`docs/ADVANCEMENT-PHASE-1-PLAN.md`（只登记盘点结果与子块）；不修改源码、测试、CI、依赖、既有设计/章程、规则数据、MANIFEST、存档或秘密。
- **Depends on:** #282、#286（均已验收）；#285 的剩余只读路径是盘点输入；#288 已验收。
- **Acceptance criteria:**
  - [ ] `AC-1`: 清单逐项覆盖旧 `progress::level_up` 的结构化与 Markdown 两条路径、`--hp roll` 的 `rolls.log`、`char`/建卡/同伴、party/combat 状态、checkpoint/migrate、日志、完整性清单、缓存/派生写入与未来 sidecar/schema 元数据目标；每项含命令与代码定位、写入文件/目录、现有锁、是否可与成长提交并发、建议接入方式（SAVE 锁 / 拒绝写入 / 声明只读）、未接入时的行为与备注。
  - [ ] `AC-2`: 只读与会写路径按 #279/#281/#285 的结论区分；`guard`/`secret list` 的缓存写入不得列为只读；`char show` 与 `char <who> spells` 只读视图分别标注；`adventure index build` 与 `adventure section` 的写入条件明确。
  - [ ] `AC-3`: 清单与计划不把未实现/未验证能力写成已完成；不声称现有 checkpoint/完整性锁能保护角色/party/进度/日志内容；不改变 #264 结论；未决接入语义标 `NEEDS_HUMAN_DECISION`。
  - [ ] `AC-4`: 仅修改两个允许文档；清单与计划不含真实存档、书籍或秘密；证据矩阵绑定实际 head。

**分批过渡门（OD-2）**：`ADV-P1-1pre` 与 `ADV-P1-1a`–`ADV-P1-1e` 各自是独立可交付块；每块发布时必须满足自己的过渡门 AC——同一存档上其余未接入写入口要么失败关闭拒绝写入，要么经合成测试证明**整个 CLI 运行**真正只读（不是只证明某个函数体不写）。该门是每个子块的发布条件，**不要求等 `ADV-P1-1e` 或全部组完成**；没有该证据不得发布该部分接入。测试归属写在各自 AC 中，覆盖清单引用 [写者盘点](ADVANCEMENT-WRITER-INVENTORY.md) §2/§5。

### Task: `ADV-P1-1pre` — 跨 CLI 过渡拒绝门与未接入写者注册表

- **Goal:** 让 `talkdnd` 与 `combat-sim` 两套 CLI 的所有写存档入口共享一个过渡门：当存档存在 advancement lock/state 时，未接入写入口失败关闭拒绝写入；只读入口保持可用。建立覆盖盘点 §2 的写者注册表，并直接在两套 CLI 的真实调用路径上测试拒绝，不通过包装层代替。
- **建议允许路径：** `tools/talkdnd/src/`、`tools/combat-sim/src/`、`tools/talkdnd/tests/`、`tools/combat-sim/tests/`、`docs/ADVANCEMENT-WRITER-INVENTORY.md`（只记录注册表与证据位置）
- **Depends on:** `ADV-P1-1`（盘点）；未初始化存档、拒绝 UX、CLI 兼容等 `NEEDS_HUMAN_DECISION` 已决定。**未决前只能设计，不得实现。**
- **Acceptance criteria:**
  - [ ] `AC-1`: 写者注册表覆盖盘点 §2 的全部写存档入口（两套 CLI），逐项给出命令/函数定位、写入对象、是否已接入过渡门；未接入入口不得静默写入。
  - [ ] `AC-2`: 当存档存在 advancement lock/state 时，未接入写入口在 `talkdnd` 与 `combat-sim` 的真实调用路径上失败关闭拒绝写入；只读入口保持可用。测试必须直接调用两套 CLI 的实际入口（不是只测共享包装函数），归属 `tools/talkdnd/tests/` 与 `tools/combat-sim/tests/`。
  - [ ] `AC-3`: 不改变既有业务语义、不启用新的属性提升/专长选择或 D 协议提交入口；拒绝只发生在写存档之前。
  - [ ] `AC-4`: 合成测试覆盖注册表中每个写入口的拒绝或只读声明；没有证据的入口不得被登记为“已覆盖”。
  - [ ] `AC-5`: 该过渡门是 `ADV-P1-1a`–`ADV-P1-1e` 的共享前置，不因 CLI 不同而分裂成两套语义；后续块不得绕过它。

### Task: `ADV-P1-1a` — CLI 角色 / 进度 / 账本写者接入

- **Goal:** 让会写角色卡、建卡状态、进度/XP/钱袋、场景、识别/揭示/原创/实体/关系/世界时间/记忆/设置/秘密/资料/冒险索引的 CLI 入口在写存档前取得 SAVE 锁，并在锁不可用时失败关闭；纯只读入口明确声明只读。
- **建议允许路径：** `tools/talkdnd/src/`（`lib.rs`、`progress.rs`、`charsheet.rs`、`chargen.rs`、`identify.rs`、`reveal.rs`、`original.rs`、`entity.rs`、`graph.rs`、`worldtime.rs`、`memory.rs`、`settings.rs`、`secret.rs`、`handout.rs`、`bookindex.rs`、`adventure.rs`、`start.rs`、`save.rs` 等实际写者）、`tools/talkdnd/tests/`、`docs/ADVANCEMENT-WRITER-INVENTORY.md`（只记录接入状态与证据位置）
- **Depends on:** `ADV-P1-1pre`（跨 CLI 过渡门）、`ADV-P1-1`（清单）、#282、#286
- **Acceptance criteria:**
  - [ ] `AC-1`: 清单 §2.1、§2.2、§2.6 中会写存档的入口全部在写目标前取得 SAVE 锁；锁序为 `SAVE → checkpoint .talkdnd.lock → integrity .talkdnd.integrity.lock`，不得反向。
  - [ ] `AC-2`: SAVE state 缺失、不可写、`active` 或 `in_doubt` 时，每个入口失败关闭且不写目标；合成测试逐条覆盖，错误信息说明未写/可能已写阶段。**state 缺失时的拒绝是决定前的临时保守处理**，不是最终 UX/兼容决定；最终语义按本节末 `NEEDS_HUMAN_DECISION` 处理。
  - [ ] `AC-3`: `char <who> spells` 普通视图保持只读（运行前后整树字节一致）；`char show` 及其余 `char` op 走 SAVE；`guard`/`secret list` 的缓存写入要么在 SAVE 内，要么按 `NEEDS_HUMAN_DECISION` 改为不写缓存。
  - [ ] `AC-4`: 旧 `progress::level_up` 的结构化与 Markdown 两条路径仍可用；回归测试证明未删除、未跳过、未弱化，且两条路径都被盘点并纳入保护。
  - [ ] `AC-5`: 本块不启用任何新的属性提升/专长选择或 D 协议提交入口；不新增 schema/sidecar。
  - [ ] `AC-6`: 未初始化存档、拒绝 UX 与 CLI 兼容按本节末的 `NEEDS_HUMAN_DECISION` 记录，不由执行者自行决定。
  - [ ] `AC-7`: 本组过渡门（见上）：同一存档上其余未接入写入口必须拒绝写入，或经 `tools/talkdnd/tests/` 的合成测试证明整个 CLI 运行真正只读；覆盖清单引用盘点 §2/§5。该证据是发布本组接入的条件，不要求等 `ADV-P1-1e` 完成。

### Task: `ADV-P1-1b` — party / combat 写者接入

- **Goal:** 让 `combat-sim` 会改遭遇/party 的命令、`party rest|set-hp|sync`，以及 `--out` 指向存档内的 `content import-srd-prose`/`content import-srd-pack`/`content import-srd` 与 `content pack build|release` 在写存档前取得 SAVE 锁；只读战斗命令明确声明只读。
- **建议允许路径：** `tools/combat-sim/src/`（`cli.rs`、`party.rs`、`state.rs`、`integrity.rs`、`player.rs`）、`tools/talkdnd/src/charsheet.rs`（party 保存调用点）、`tools/combat-sim/tests/`、`tools/talkdnd/tests/`、`docs/ADVANCEMENT-WRITER-INVENTORY.md`（只记录接入状态与证据位置）
- **Depends on:** `ADV-P1-1pre`、`ADV-P1-1a`、`ADV-P1-1`、#286
- **Acceptance criteria:**
  - [ ] `AC-1`: 清单 §2.3 中会改状态的命令全部在写 state/party/日志前取得 SAVE 锁；`status`/`scene`/`log`/`simulate`/`content list|show|check`/`pack list|show|verify`/`party show` 不写存档并声明只读。
  - [ ] `AC-2`: 单次命令的 `state.save` → `party.save` → 日志追加顺序在 SAVE 内执行；任一步失败即停止并报告已写/未写阶段，不声称多文件原子性。
  - [ ] `AC-3`: `PartyState::save` 与 `EncounterState::save` 不再把完整性锁当作内容保护；合成测试证明 party/state 内容写入受 SAVE 约束。
  - [ ] `AC-4`: `combat-sim` 不新增对 `talkdnd` 的依赖；SAVE 锁原语是共享还是由 `talkdnd` 包装，按本节末的 `NEEDS_HUMAN_DECISION` 记录并实施。
  - [ ] `AC-5`: 活动遭遇识别算法不因本块被发明；`.state.json` 只作为未来判定输入，不声称已解决活动遭遇只预览。
  - [ ] `AC-6`: `combat-sim content import-srd-prose` / `content import-srd-pack` / `content import-srd` 的 `--out` 指向存档内时，必须接入 SAVE 或失败关闭拒绝写入；`content pack build|release` 的 `--out` 指向存档内时同样处理；目标在存档外时才可按存档外写入声明。**只有 `content pack build|release` 有 `--dry-run`**，其 dry-run 保持只读；三个 `import-*` 命令没有 dry-run，必须按写者处理。用 `tools/combat-sim/tests/` 的合成测试以合法参数进入实际分支，分别覆盖“out 在存档内”和“out 在存档外”，并把它们纳入提交入口启用前的覆盖门（`ADV-P1-10`）。
  - [ ] `AC-7`: 本组过渡门（见上）：同一存档上其余未接入写入口必须拒绝写入，或经 `tools/combat-sim/tests/` 与 `tools/talkdnd/tests/` 的合成测试证明整个 CLI 运行真正只读；覆盖清单引用盘点 §2/§5，并包含 AC-6 的存档内输出目标。该证据是发布本组接入的条件，不要求等 `ADV-P1-1e` 完成。

### Task: `ADV-P1-1c` — checkpoint / migrate 写者接入

- **Goal:** 让 `checkpoint snapshot`、`checkpoint restore` 与 `migrate` 在整树读写前取得 SAVE 锁，并保持 `SAVE → checkpoint → integrity` 的总序；只读的 checkpoint 查询与 `migrate --dry-run` 保持只读。
- **建议允许路径：** `tools/talkdnd/src/checkpoint.rs`、`tools/talkdnd/src/migrate.rs`、`tools/talkdnd/src/save.rs`、`tools/talkdnd/src/lib.rs`（`tracked` 分支）、`tools/talkdnd/tests/`（含必要审计测试）、`docs/ADVANCEMENT-LOCK-DESIGN.md`（只记录接入状态与证据位置）
- **Depends on:** `ADV-P1-1pre`、`ADV-P1-1b`、`ADV-P1-1`
- **Acceptance criteria:**
  - [ ] `AC-1`: `checkpoint snapshot` / `restore` / `migrate` 在写任何目标前取得 SAVE 锁，再按 `checkpoint .talkdnd.lock` → `integrity .talkdnd.integrity.lock` 获取；反向获取在合成用例中被判为协议违规。
  - [ ] `AC-2`: 备份/恢复继续排除根目录控制文件；特殊文件、空间不足、同步失败或结果不明时停止，保留不完整备份并报告阶段。
  - [ ] `AC-3`: `checkpoint list|show` 与 `migrate --dry-run` 运行**整个 CLI** 前后整树字节一致；必须调整 `lib.rs` 的 `tracked` 分支或等价机制以消除外层审计写入，不得只证明函数体只读，也不得因接入锁而创建 lock/state 或写探针。
  - [ ] `AC-4`: 不改变 #264 结论；不声称 checkpoint 锁是 OS 锁、能保护内容或约束非协作写者。
  - [ ] `AC-5`: 未初始化存档与“迁移/恢复是否自动 init”的语义按本节末的 `NEEDS_HUMAN_DECISION` 记录。
  - [ ] `AC-6`: 本组过渡门（见上）：同一存档上其余未接入写入口必须拒绝写入，或经 `tools/talkdnd/tests/` 的合成测试证明整个 CLI 运行真正只读；覆盖清单引用盘点 §2/§5。该证据是发布本组接入的条件，不要求等 `ADV-P1-1e` 完成。
  - [ ] `AC-7`: T6a 独立取证：`checkpoint snapshot` / `restore` 与成长路径按总序 `SAVE → checkpoint → integrity` 竞争，使用两进程 + 测试级超时；挂起即判失败，不以“没死锁”代替证据。若正式提交尚未接线，使用等价合成持锁路径并限定证据范围；记录平台、head 与逐项结果。

### Task: `ADV-P1-1d` — 日志 / 完整性写者接入

- **Goal:** 让所有会追加 `Sessions/*.md`/`rolls.log` 或改写 `integrity.yaml` 的入口在写前取得 SAVE 锁，同时保持完整性锁只覆盖清单读改写、不声称保护内容。
- **建议允许路径：** `tools/talkdnd/src/save.rs`、`tools/talkdnd/src/rolls.rs`、`tools/talkdnd/src/audit.rs`、`tools/talkdnd/src/progress.rs`（日志）、`tools/talkdnd/src/charsheet.rs`（`log_line`）、`tools/talkdnd/src/checkpoint.rs`（恢复日志）、`tools/talkdnd/src/migrate.rs`（迁移日志）、`tools/combat-sim/src/player.rs`（战斗日志/rolls）、`tools/combat-sim/src/integrity.rs`、`tools/talkdnd/tests/`、`tools/combat-sim/tests/`
- **Depends on:** `ADV-P1-1pre`、`ADV-P1-1c`、`ADV-P1-1`
- **Acceptance criteria:**
  - [ ] `AC-1`: 清单 §2.5 与 §2.8 中会追加日志或改写清单的入口全部在写前取得 SAVE 锁；`log rolls`、`checkpoint list|show` 等只读入口不取写锁。
  - [ ] `AC-2`: `audit::after_command` / `integrity accept` 在 SAVE 内按 `SAVE → integrity` 顺序取锁；完整性锁仍只覆盖 `integrity.yaml` 读改写。
  - [ ] `AC-3`: 日志追加保持不截断、不改写既有行；重复请求的幂等仍由 `ADV-P1-4` 设计，不在本块声称已实现。
  - [ ] `AC-4`: `check`（无参数）与 `integrity status` 的“可能创建清单/baseline”按写入入口处理，不列为纯只读。
  - [ ] `AC-5`: 合成测试覆盖锁不可用时的拒绝、日志不重复/不丢失的既有语义，以及清单更新失败不洗掉 dirty 标记。
  - [ ] `AC-6`: 本组过渡门（见上）：同一存档上其余未接入写入口必须拒绝写入，或经 `tools/talkdnd/tests/` 与 `tools/combat-sim/tests/` 的合成测试证明整个 CLI 运行真正只读；覆盖清单引用盘点 §2/§5。该证据是发布本组接入的条件，不要求等 `ADV-P1-1e` 完成。

### Task: `ADV-P1-1e` — sidecar / schema 元数据 / 缓存写者接入

- **Goal:** 让既有 `save-meta.yaml` 写者与缓存/派生写入有明确归属：schema 元数据在 SAVE 内写；未来选择 sidecar 由 `ADV-P1-6` 注册到 `ADV-P1-4` 的发布协议；未实现前不得声称这些目标已受保护。
- **建议允许路径：** `tools/talkdnd/src/migrate.rs`（`write_meta`）、`tools/talkdnd/src/secret.rs`（缓存）、`tools/talkdnd/src/start.rs`（`.cache`/`digest`）、`tools/talkdnd/tests/`、`docs/ADVANCEMENT-WRITER-INVENTORY.md` 与 `docs/ADVANCEMENT-LOCK-DESIGN.md`（只记录接入状态与证据位置）
- **Depends on:** `ADV-P1-1pre`、`ADV-P1-1d`、`ADV-P1-1`
- **Acceptance criteria:**
  - [ ] `AC-1`: `migrate::write_meta` 的 `save-meta.yaml` 写入在 SAVE 内；合成测试证明锁不可用时 schema 元数据不被改写。
  - [ ] `AC-2`: 当前不存在选择 sidecar 写入路径；本块不创建 sidecar、不新增 schema 11、不预设字段或路径。
  - [ ] `AC-3`: 选择 sidecar 与主 schema 元数据作为未来发布目标，由 `ADV-P1-6` 按 `ADV-P1-4` 的注册方式接入；本块只记录归属，不声称已覆盖。
  - [ ] `AC-4`: `guard`/`secret list` 的缓存与 `start` 的 `.cache`/`digest` 按本节末的 `NEEDS_HUMAN_DECISION` 决定是纳入 SAVE 还是改为不写；未决前按保守方式处理。
  - [ ] `AC-5`: D 完整备份**包含既有用户备份**；lock、操作前快照、恢复工作记录等控制元数据**单独保留、不并入该备份**；缓存/派生文件按已批准分类处理，保留与清理责任未定。不得把“备份目录存在”当成完整备份，也不得把控制元数据混入用户数据备份。
  - [ ] `AC-6`: 本组过渡门（见上）：同一存档上其余未接入写入口必须拒绝写入，或经 `tools/talkdnd/tests/` 的合成测试证明整个 CLI 运行真正只读；覆盖清单引用盘点 §2/§5。该证据是发布本组接入的条件，不要求等全部组完成。

**接入语义未决（`NEEDS_HUMAN_DECISION`，不得由执行者自行决定）：**

- **未初始化存档**：没有 `.talkdnd.advancement.lock`/`.state` 的存档，普通写命令应全部拒绝、只读命令仍可用、还是由 `new`/`start`/`migrate` 自动 `init`？
- **未接入入口的拒绝 UX**：错误文案、退出码、是否在 `--help` 与玩家文档中声明只读，以及现有脚本兼容性。
- **CLI 兼容**：`new`、`start`、`migrate`、`chargen` 等创建/迁移入口是否自动初始化 SAVE；旧存档首次运行是否强制 `init`。
- **缓存写入**：`guard`/`secret list` 的 `secret-terms.cache.json`、`start` 的 `.cache`/`digest` 是纳入 SAVE 锁，还是改为不写缓存以保持只读承诺。
- **`combat-sim` 锁实现**：`combat-sim` 不依赖 `talkdnd`；SAVE 锁原语是共享还是由 `talkdnd` 包装，属架构取舍。
- **日志并发语义**：活动操作期间独立 `log`/`checkpoint <text>` 是拒绝、排队，还是允许并标记“可能交错”。
- **`new` 与 SAVE 的关系**：新存档创建时是否立即 `init`，以及失败时留下 lock 无 state 的处理。
- **控制元数据范围**：`.cache/`、`digest.md`、操作证据与备份目录的保留/清理责任未定。

以上决定记录在对应 Issue；决定前子块只能按保守方式实现或停在设计，不得把未决项写成已完成。


### Task: `ADV-P1-2` — D 阶段机与备份验证（admit / backup / candidate）

- **Goal:** 在合成 fixture 上把 D 流程实现到“候选已验证、未发布”，任何失败都停在发布之前且目标文件字节不变。
- **建议允许路径：** `tools/talkdnd/src/advancement_lock.rs`、新增 `tools/talkdnd/src/advancement_commit.rs`（或等价模块）、`tools/talkdnd/src/lib.rs`、`tools/talkdnd/src/migrate.rs`（备份实现如需复用）、`tools/talkdnd/tests/`、`docs/ADVANCEMENT-LOCK-DESIGN.md` 与 [ADVANCEMENT-RECOVERY-OPTIONS.md](ADVANCEMENT-RECOVERY-OPTIONS.md)（记录实现状态与证据位置）
- **Depends on:** `ADV-P1-1`（盘点）；D 阶段机本身不启用提交入口，提交入口的启用仍以 `ADV-P1-1a`–`ADV-P1-1e` 全部就绪为门（见 `ADV-P1-10`）。
- **Acceptance criteria:**
  - [ ] `AC-1`: `admit` 在写 `active` 前建立操作前快照；快照创建/写/文件 sync/重读校验/目录同步任一失败（含首次操作）时不写 `active`、不写目标，原 state 保持。
  - [ ] `AC-2`: `backup` 覆盖执行前全部用户数据并**包含既有用户备份**（OD-7）；锁文件、操作前快照与恢复工作记录作为控制元数据单独保留、不并入该备份。
  - [ ] `AC-3`: 特殊文件、权限不足、空间不足、同步失败或结果不明时停止，保留不完整备份并报告阶段；不得静默跳过或跟随链接逃出存档根。
  - [ ] `AC-4`: `candidate` 在已验证前像上构建并逐文件校验 schema/身份/关系/目标摘要；跨设备/跨卷拒绝；未通过校验的候选不得标记为已验证。
  - [ ] `AC-5`: 每个失败注入点（备份、清单、候选构建与校验）之后，目标角色、party、进度、日志与完整性文件字节与发布前一致；报告中断位置与恢复所需材料。
  - [ ] `AC-6`: 本块不发布任何目标文件；真实存档、网络盘、掉电与 Windows 目录持久性结论按实际执行标 `NOT_RUN` / `UNKNOWN`。
- **粒度说明：** `admit`/`backup`/`candidate` 是同一条状态机的连续阶段，单个可验收结果是“候选已验证、未发布”，因此不拆块；块内 AC 仍逐条独立取证。

### Task: `ADV-P1-3` — 新路径内层锁的有界获取（OD-9）

- **Goal:** 让新成长路径在获取内层完整性锁时按批准的默认 5 秒 deadline 有界获取、可显式覆盖、超时失败关闭，同时保留旧完整性锁的无限阻塞语义。
- **建议允许路径：** `tools/talkdnd/src/`（新增有界获取模块与 `advancement_commit.rs` 脚手架）、`tools/combat-sim/src/integrity.rs`（新增有界变体；不得改变现有阻塞 API 的语义；按源码只读约定）、`tools/talkdnd/tests/`、`tools/combat-sim/tests/`、`docs/ADVANCEMENT-LOCK-DESIGN.md`（记录实现状态与证据位置）
- **Depends on:** `ADV-P1-2`（需要提交路径脚手架作为调用点）
- **Acceptance criteria:**
  - [ ] `AC-1`: 新路径的内层获取使用有界 API，默认 5 秒 deadline，可显式覆盖；超时返回失败关闭错误，不写目标、不改 state、不留半锁。
  - [ ] `AC-2`: 旧完整性锁的阻塞语义与既有调用方不变：代码核对证明旧 API 仍无限阻塞，既有测试未删除、未跳过、未弱化。
  - [ ] `AC-3`: T6b 合成用例：另一进程长期持有完整性锁时，新路径在预算内失败并报告“内层锁”；诊断记录 `D_save` 与各内层 `D_inner`，不把 `D_save` 外推为整条提交路径有界。
  - [ ] `AC-4`: 发布与提交接线（`ADV-P1-4` 及以后）只在本块的有界获取可用时接入；本块不接线发布。
  - [ ] `AC-5`: 超时后重试重新计算预算，不沿用上次剩余时间，也不因为“曾经超时”而跳过校验。
  - [ ] `AC-6`: 本块的结论只覆盖新路径的有界获取；旧锁的跨平台阻塞行为按既有事实记录，不声称整条路径有界。

### Task: `ADV-P1-4` — 发布、日志/完整性与幂等

- **Goal:** 让一次成长提交的逐文件发布在合成故障注入下恰好应用一次（含日志与完整性更新），失败即停并留下可判定的阶段记录。
- **建议允许路径：** `tools/talkdnd/src/advancement_commit.rs`（或等价模块）、`tools/talkdnd/src/lib.rs`、`tools/talkdnd/src/progress.rs`、`tools/talkdnd/src/save.rs`、`tools/combat-sim/src/integrity.rs`（按源码只读约定）、`tools/talkdnd/tests/`、`tools/combat-sim/tests/`、`docs/ADVANCEMENT-LOCK-DESIGN.md`
- **Depends on:** `ADV-P1-2`、`ADV-P1-3`
- **Acceptance criteria:**
  - [ ] `AC-1`: 发布顺序为**基础目标顺序**（`sheets/*.yaml` → `combat/party-state.yaml` → `Characters/*.md` → `progress.yaml` → 追加日志 → 完整性清单），并按 **`ADV-P1-6` 在获授权方案确定后扩展**；目标集合以注册方式定义，扩展目标同样走 intent、前像/目标比对与逐文件发布，不由本块预设 sidecar 字段或路径。每个目标先写 intent、再复核当前=前像、再临时文件 + rename + 文件与父目录 sync、最后标 done。
  - [ ] `AC-2`: intent 写失败不发布该文件；复核不符、写失败或同步失败立即停止，并把已发布/未发布写回 state；不自动重试发布。
  - [ ] `AC-3`: 日志按操作 ID 去重、不截断不改写既有行、尾部异常停止；完整性更新保留既有 dirty 状态，不洗掉玩家手改标记。
  - [ ] `AC-4`: 幂等有合成证据：**重复提交请求**不重复增加属性、XP、资源或日志；无法证明“是否已追加”时停下而不是重放。**恢复重试**的幂等由 `ADV-P1-5` 验收，不在本 AC 范围。
  - [ ] `AC-5`: 终态语义按设计强制：部分发布或结果不明只能 `in_doubt`，零目标写入才可 `failed`，`complete` 只在候选比对、日志与完整性完成且无未决 intent 后写；伪造或矛盾终态失败关闭。
  - [ ] `AC-6`: 发布只经 `ADV-P1-3` 的有界获取接入内层锁；接入后整条提交路径的总等待仍不承诺有界（旧锁保持无限阻塞），T6b 结果只覆盖新路径。
  - [ ] `AC-7`: T2a/T7a/T7b/T8 有实测结果；T7b 只证明“如实报告无法排除覆盖”，不输出注入值、不宣称检测成功。
  - [ ] `AC-8`: 目标扩展点有合成证据：注册一个**合成扩展目标**后，它获得 intent、前像/目标摘要与发布故障行为覆盖，不被发布路径跳过；本 AC 不验收恢复覆盖（恢复由 `ADV-P1-5` 交付），也不要求 `ADV-P1-6` 的实际 sidecar 与主 schema 元数据目标已存在。

### Task: `ADV-P1-5` — 显式恢复

- **Goal:** 让失败或中断后的恢复在合成注入下能判定原操作与进度、保护当前内容并由人显式选择，且不自动续跑或重放。
- **建议允许路径：** `tools/talkdnd/src/advancement_commit.rs`（或独立的恢复模块）、`tools/talkdnd/src/lib.rs`、`tools/talkdnd/tests/`、`docs/ADVANCEMENT-LOCK-DESIGN.md` 与 [ADVANCEMENT-RECOVERY-OPTIONS.md](ADVANCEMENT-RECOVERY-OPTIONS.md)
- **Depends on:** `ADV-P1-4`
- **Acceptance criteria:**
  - [ ] `AC-1`: 任何目标写入前先建立恢复工作证据（原未完成 state 的原始字节/revision/checksum/operation_id/stage/pending、备份位置、人工选择、新恢复 ID、原操作引用）；创建/写/sync/重读/目录同步失败即停止，不做任何目标写入。
  - [ ] `AC-2`: 恢复前先保护恢复时的当前树（含之后的手改与日志）；由人显式选择“回滚到前像”或“继续到目标”，不整树覆盖、不自动重基。
  - [ ] `AC-3`: 当前内容既非前像也非目标、日志已有后续追加、身份不明或无法判定原操作时停止；保留两份证据并要求人工合并，不删除后续日志。
  - [ ] `AC-4`: 恢复再次中断后凭工作记录与受保护快照判定原操作与进度；无法判定 → `in_doubt`，只允许人工处理，不自动续跑/重放。
  - [ ] `AC-5`: **恢复重试**与重复恢复不重复应用（合成用例：恢复中断后重试、重复发起恢复），且不依赖永久历史；证据被清理后的可恢复性不作保证。
  - [ ] `AC-6`: T9a/T9b 有实测结果；#264 未解决前不声称恢复安全，也不把通过用例当作真实存档证明。
  - [ ] `AC-7`: 用**合成注册目标**证明通用恢复机制不跳过扩展目标：恢复逐文件比较覆盖全部已注册发布目标，不因目标新增而跳过；本 AC **不要求** `ADV-P1-6` 的实际 sidecar 与主 schema 元数据目标已存在（那由 `ADV-P1-6` 验收）；遇到不在已注册目标内的存档文件时明确报告范围限制，不静默忽略。

### Task: `ADV-P1-6` — sidecar 入籍与 9/10→11 自动迁移

- **Goal:** 让 schema 9/10 存档在合成 fixture 上自动迁移到 schema 11，并为每个角色建立与承载体双边绑定、可验证的选择 sidecar，同时保留手改与未知历史。
- **建议允许路径：** `tools/talkdnd/src/migrate.rs`、`tools/talkdnd/src/charsheet.rs`、`tools/talkdnd/src/progress.rs`、新增 sidecar 模块、`tools/talkdnd/src/lib.rs`、`tools/talkdnd/src/advancement_commit.rs`（`ADV-P1-4` 的发布模块，用于注册 sidecar 与主 schema 元数据目标）、`tools/talkdnd/tests/`、`LetsPlay/docs/layout.md`（存档结构权威）、`docs/ADVANCEMENT-IDENTITY-CONTRACT.md`（记录实现状态与证据位置）、三个 crate 的 `Cargo.toml` 版本号、`CHANGELOG.md`、`docs/PROJECT-CHARTER.md`（按 LIVING CHARTER 同 PR 更新）
- **Depends on:** `ADV-P1-1e`、`ADV-P1-5`；**具体身份方案已获所有者授权**（sidecar 字段、随机源、令牌生命周期、旧格式无损写入与重绑/克隆协议）；**`ADV-P1-13` 的跨平台枚举/链接/备份调查结论支持实现**——结论为 `UNKNOWN` 时本块阻塞，不得以“调查是并行任务”为由视为已满足
- **Acceptance criteria:**
  - [ ] `AC-1`: 迁移预览只读、不写任何文件；只有完整备份（含既有用户备份）校验通过后才开始写；备份失败、空间不足或特殊文件/元数据无法安全复制时停止且目标未改。
  - [ ] `AC-2`: 9 与 10 自动迁移到 11；已是 11 不重复迁移；未来/未知 schema 与损坏数据失败关闭；不迁移时的旧数据仍按已批准边界可读。
  - [ ] `AC-3`: 角色承载体与 sidecar 双侧保存同一随机非秘密标识；缺失、不一致或同档重复一律停止；不按名称、旧 ID 或摘要自动绑定，不自动分叉/合并。
  - [ ] `AC-4`: 旧角色首次关联需 DM 核对确认；无法无损处理时停止，原字节保留，不伪造历史、不把缺失记录当成“没有选择”。
  - [ ] `AC-5`: dirty sheet 不被 serde 重写；未知键、注释、玩家手改与历史未知按原字节保留；无关手改不被迁移改写。
  - [ ] `AC-6`: 迁移后停止旧版写入的逐入口审计结果记录在 Issue 证据中；不得声称旧二进制会在所有入口自动拒绝写入。
  - [ ] `AC-7`: 明确记录**不承诺**检测共同复制“角色 + sidecar”或整档重放；真实存档迁移、跨平台枚举与链接语义按实际执行标 `NOT_RUN` / `UNKNOWN`。
  - [ ] `AC-8`: 若 `ADV-P1-13` 的结论为 `UNKNOWN` 或与实现前提冲突，本块停在预览/设计并报告，不开始写迁移。
  - [ ] `AC-9`: 选择 sidecar 与主 schema 元数据作为发布目标注册到 `ADV-P1-4` 的 D 协议：intent、前像/目标摘要、故障注入与显式恢复都覆盖它们，证明这些目标没有游离在协议之外；具体字段与路径由获授权方案决定，本计划不预设。

### Task: `ADV-P1-7` — 只读预览

- **Goal:** 让玩家能在不写任何文件的前提下看到本次升级的可用机会、候选、潜在变化与未知项。
- **建议允许路径：** `tools/talkdnd/src/`（新增预览模块与只读发现接线）、`tools/talkdnd/tests/`、`LetsPlay/docs/`（玩家/DM 预览说明）
- **Depends on:** `ADV-P1-6`
- **Acceptance criteria:**
  - [ ] `AC-1`: 预览不打开、不创建、不锁定 lock 或 state，不写探针、`.cache`、日志或任何文件；合成用例比较预览前后整树字节。
  - [ ] `AC-2`: 输出 `consistency: unverified`；缺少 lock/state、只读目录或无法探测时给出 `write_capability: unverified`，不承诺可提交。
  - [ ] `AC-3`: 展示升级等级、可用机会及来源、候选、潜在属性与派生变化、未知项与手动效果；不掷持久化骰。
  - [ ] `AC-4`: 缺少可信职业/等级/资格来源时停止并解释，不猜测；不暗示所有候选均合法。
  - [ ] `AC-5`: 计算并返回 `preview_id`（完整枚举清单 + 每文件摘要 + 规则集 + 授权状态 + 遭遇状态 + 身份映射 + 计划变更的摘要），由调用方保存，预览不持久化。
  - [ ] `AC-6`: 预览在只读目录与缺失 lock/state 下仍可运行；合成测试、Issue、PR、提交与 CI 日志不得包含真实存档内容、书籍正文或秘密；本地预览输出只展示本次操作所需信息并遵守既有信息边界。

### Task: `ADV-P1-8` — 玩家显式选择与条件校验

- **Goal:** 让玩家显式选定本次机会与选项、每个资格条件都可判为“满足/不满足/未知”并带来源与规则版别，已知非法优先拒绝，未知只在有可审计确认时继续。
- **建议允许路径：** `tools/talkdnd/src/`（新增选择与校验模块）、`tools/talkdnd/tests/`、`LetsPlay/docs/`（DM 确认与手动登记说明）
- **Depends on:** `ADV-P1-7`
- **Acceptance criteria:**
  - [ ] `AC-1`: 每个条件输出“满足 / 不满足 / 未知”三态，并附依据、来源与规则版别（2014/2024）；不把提示文案当规则权威。
  - [ ] `AC-2`: 已知不满足优先于任何未知与 DM 确认，一律拒绝；**已知非法与未知并存时拒绝**。
  - [ ] `AC-3`: 未知只在 DM 明确确认且依据非空、可审计时继续，并显示“未经程序验证”，不得转成已验证。
  - [ ] `AC-4`: **玩家显式选择本次机会与选项**；未提供玩家选择时不得自动选择、不得提交；**DM 对未知条件的确认不替代玩家选择**。
  - [ ] `AC-5`: 合成负向用例：缺少玩家选择、空依据、缺确认、已知非法 + 未知、身份歧义暂停、错误规则集/未授权内容/可判定重复拒绝；拒绝路径不写任何文件。
  - [ ] `AC-6`: 未识别专长只在可独立判断的机会、来源授权与重复条件通过后登记为“未验证、效果手动”，不能绕过任何已知非法条件。
  - [ ] `AC-7`: 合成测试、Issue、PR、提交与 CI 日志不得包含真实存档内容、书籍正文或秘密；本地校验输出只展示本次操作所需信息并遵守既有信息边界；输入格式错误、越界与别名规避被拒绝。

### Task: `ADV-P1-9` — 仅可靠派生数值的自动更新

- **Goal:** 让派生重算只自动更新来源与派生关系可可靠确定的数值，其余保留原值并提示 DM。
- **建议允许路径：** `tools/talkdnd/src/`（派生数值重算模块、`progress.rs`、`charsheet.rs`）、`tools/talkdnd/tests/`、`LetsPlay/docs/`（DM 说明）
- **Depends on:** `ADV-P1-8`
- **Acceptance criteria:**
  - [ ] `AC-1`: 逐项列出属性变化对 HP、攻击、豁免、技能、施法 DC、资源与当前遭遇的影响，并标注“可可靠派生 / 需人工”；清单随 PR 发布。
  - [ ] `AC-2`: 可可靠派生的数值恰好更新一次，不重复叠加；不覆盖手改值（合成用例：手改 HP 或属性后运行派生重算）。
  - [ ] `AC-3`: 不确定或无法判定来源的派生值保留原值并提示 DM，不静默重新生成；法术列表与 combat refs 不被静默改写。
  - [ ] `AC-4`: 派生重算不改变已消耗资源、活动遭遇状态与手动效果；活动遭遇期间不触发任何写入。
  - [ ] `AC-5`: 按 2014/2024 × SRD/FULL 记录每个机制的“自动 / 手动 / 未知”覆盖结论；保留任何手动项都不得被称为全量自动化。

### Task: `ADV-P1-10` — 提交接线

- **Goal:** 让一次玩家显式选择在重新验证绑定、条件与派生结果后经 D 协议恰好提交一次，过期或变化即拒绝。
- **建议允许路径：** `tools/talkdnd/src/`（提交接线与 `lib.rs`）、`tools/talkdnd/tests/`、`LetsPlay/docs/`（玩家/DM 提交说明）、`docs/PROJECT-CHARTER.md`（按 LIVING CHARTER）
- **Depends on:** `ADV-P1-1pre`、`ADV-P1-1a`–`ADV-P1-1e`、`ADV-P1-3`、`ADV-P1-6`、`ADV-P1-7`、`ADV-P1-8`、`ADV-P1-9`；**活动遭遇识别算法由所有者决定**（未决前本块的活动遭遇 AC 不能判定通过）
- **Acceptance criteria:**
  - [ ] `AC-1`: 提交按 §2.4 顺序取锁、重新发现、重算绑定并与 `preview_id` 比较；不一致、缺少 `preview_id`、预览后内容/授权/遭遇变化一律拒绝并要求重新预览；只给单一摘要时只报“绑定变化，原因未知”。
  - [ ] `AC-2`: 提交前重新执行选择与校验；请求必须携带玩家显式选择，缺失即拒绝、不自动替选；任一条件变为不满足或未知而缺少有效确认时拒绝，不写入。
  - [ ] `AC-3`: 提交经 D 协议一次应用：属性、派生数值、party、进度、日志与完整性按 `ADV-P1-4` 的基础顺序与幂等规则处理，并由 `ADV-P1-6` 扩展的选择 sidecar 与主 schema 元数据目标一并纳入 intent、前像/目标比对与发布。
  - [ ] `AC-4`: **提交启用前必须在本块直接验收派生值边界**：不可靠或无法判定的派生值保留原值、提示 DM、不覆盖手改、不静默重生成；用本块合成用例直接取证，不得只引用 `ADV-P1-9` 的结论。
  - [ ] `AC-5`: 活动遭遇只预览不提交；损坏、歧义或无法可靠判断的遭遇同样拒绝。**遭遇识别算法未决前本 AC 不能判定通过**，按 `NEEDS_HUMAN_DECISION` 停下，不自行发明判定规则。
  - [ ] `AC-6`: 成功后展示前后差异、逐项条件状态与依据、来源、确认依据、未实现效果和需要 DM 手动完成的操作；失败说明已写/未写阶段与可恢复位置。
  - [ ] `AC-7`: 提交入口只在 `ADV-P1-1pre` 与 `ADV-P1-1a`–`ADV-P1-1e` 的全部相关写者覆盖、`ADV-P1-3` 的有界获取可用、`ADV-P1-6` 的迁移与 sidecar 就绪、`ADV-P1-9` 的派生边界验收通过后启用；任一未满足时入口不存在或失败关闭。

### Task: `ADV-P1-11` — 端到端、跨平台与故障注入收口

- **Goal:** 用合成 fixture 在三个平台与故障矩阵上取证，并如实记录未跑项与无法判定项。
- **建议允许路径：** `.github/workflows/ci.yml`、`tools/talkdnd/tests/`、`tools/combat-sim/tests/`、`tools/talkdnd/src/`（仅修复本块证据暴露的缺陷，且须在 Issue 内先说明）、`docs/`（本计划与相关设计文档的状态与限制段落）
- **Depends on:** `ADV-P1-1pre`、`ADV-P1-1a`–`ADV-P1-1e`、`ADV-P1-2` … `ADV-P1-10`
- **Acceptance criteria:**
  - [ ] `AC-1`: 合成套件在 Linux、Windows、macOS 三平台 CI 上运行，按平台分别记录结论与 head SHA；Windows 本地提交不支持按已批准边界记录，不当作缺陷修复，也不静默降级。
  - [ ] `AC-2`: 故障注入矩阵覆盖备份、候选构建/校验、每次发布（含选择 sidecar 与主 schema 元数据目标）、日志追加、完整性更新与恢复各写入前后、满盘/短写、进程中断、并发与过期预览；每项给出 `PASS` / `FAIL` / `NOT_RUN`，未跑不得折算为通过。
  - [ ] `AC-3`: 端到端合成链路“预览 → 显式选择 → 校验 → 提交 → 角色/party/进度/法术/战斗读取”一致，手动效果不被冒充为自动实现。
  - [ ] `AC-4`: 四规则集隔离（FULL 缺书/未解锁/错版拒绝，SRD 不读取 FULL）用合成包验证，不读取玩家书籍。
  - [ ] `AC-5`: 网络盘、FUSE、同步目录、跨主机互斥、掉电/目录持久性、整盘镜像与重放检测、共同复制检测：已运行项按环境/范围/head 记录结果，未运行项标 `NOT_RUN`，无法判定标 `UNKNOWN`；任何结果都不得扩展支持范围或承诺跨主机安全。
  - [ ] `AC-6`: 报告明确列出仍未实现的能力与残留风险：#264、无强制准入、无多文件原子性、无共同复制/重放检测、旧二进制不受约束；不得出现“已安全”“原子提交”一类表述。

### Task: `ADV-P1-12` — #264 现场采集（并行调查，不可替代）

- **Goal:** 在隐私安全的合成环境中为 #264 的首次 `WouldBlock` 取得可定位的失败现场，或如实记录本次未能复现。
- **建议允许路径：** [docs/CHECKPOINT-LOCK-INVESTIGATION.md](CHECKPOINT-LOCK-INVESTIGATION.md)、`docs/` 下新增调查报告；不修改产品代码或测试
- **Depends on:** none（可与其它块并行）
- **Acceptance criteria:**
  - [ ] `AC-1`: 若故障再次发生，采集失败时的锁 inode、进程树、各进程打开的描述符及其生命周期，并比较默认并行与单线程；证据中不含私有路径或存档内容。
  - [ ] `AC-2`: 明确区分“测试缺陷”与“产品锁安全影响”；无法判定时写 `UNKNOWN`，不以后续通过反推根因。
  - [ ] `AC-3`: 报告绑定环境（OS/内核/rustc/cargo）、运行次数与 head；未复现时写“本次未复现”，不写成“已解决”。
  - [ ] `AC-4`: 本块的任何结论都不得被用来把其他块的 AC 判为通过，也不得被表述为“#264 已解决”，除非有可复核的根因证据。

### Task: `ADV-P1-13` — 跨平台/网络文件系统调查（并行调查，`ADV-P1-6` 的前置门）

- **Goal:** 单独记录各平台枚举、链接、备份与文件系统语义的事实，并明确哪些结论支持或阻塞迁移实现。
- **建议允许路径：** `docs/` 下新增调查报告（可含合成实验命令与输出）；不修改产品代码、测试或 CI
- **Depends on:** none（可与其它块并行；但其结论是 `ADV-P1-6` 的前置门）
- **Acceptance criteria:**
  - [ ] `AC-1`: Linux/Windows/macOS 的目录枚举、大小写、符号链接/硬链接、特殊文件与备份元数据语义分别记录，每条绑定环境与 head。
  - [ ] `AC-2`: 网络盘/同步目录/FUSE 的实验**只在项目权限允许时**进行，并单独记录获准范围；未获准或未运行的写 `NOT_RUN`，不得以本地结果代替。
  - [ ] `AC-3`: 无法判定的条目写 `UNKNOWN`；已运行条目如实记录范围与限制，不据此扩展支持平台或承诺跨主机安全。
  - [ ] `AC-4`: 明确给出“支持 `ADV-P1-6` 实现 / 阻塞 `ADV-P1-6` / 部分支持且需限定范围”的结论；结论为 `UNKNOWN` 时按阻塞处理。
  - [ ] `AC-5`: 本块的结论不得被用来把其它块的 AC 判为通过，也不得替代 `ADV-P1-11` 的端到端与故障注入证据。

## 只能由实现后实验证明的结论（当前 `NOT_RUN` / `UNKNOWN`）

下表列出**不能**靠设计、静态阅读或既有测试通过来证明的结论。实现前它们一律是 `NOT_RUN`；实现后也只有真实执行并绑定 head 的证据才能改变状态。已运行项按环境/范围/head 记录，无法判定项写 `UNKNOWN`，都不得扩展支持范围。

| 结论 | 需要的实验 | 归属块 | 当前状态 |
|---|---|---|---|
| 两个协作写者同时提交时恰有一方成功、失败方不写目标 | 多进程竞争（T1），三平台 | `ADV-P1-1a`–`ADV-P1-1e` | `NOT_RUN` |
| 过渡拒绝门：未接入写者在 lock/state 存在时拒绝 | 过渡拒绝门合成测试（两套 CLI 直接调用） | `ADV-P1-1pre` | `NOT_RUN` |
| 阶段边界能检出锁/state 被替换并中止 | 路径替换注入（T2a/T2c/T2f1/T2f2） | `ADV-P1-1c`、`ADV-P1-2` | `NOT_RUN` |
| `admit` 快照失败时不写 `active`、不写目标（含首次操作） | 快照各窗口故障注入（T4d） | `ADV-P1-2` | `NOT_RUN` |
| 崩溃后不自动续跑，`active`/`in_doubt` 拒绝普通写入 | 分阶段 kill（T4a/T4e） | `ADV-P1-2`、`ADV-P1-4` | `NOT_RUN` |
| state 原子替换与目录持久窗口的真实行为 | 掉电/同步失败注入（T4b/T5i） | `ADV-P1-2` | `NOT_RUN` |
| 新路径内层锁的有界获取（默认 5 秒、可覆盖、超时失败关闭） | 另一进程长期持有（T6b） | `ADV-P1-3` | `NOT_RUN` |
| 发布部分失败保留备份与阶段记录、不重复应用 | 每次发布前后 kill、满盘（T7a/T14） | `ADV-P1-4` | `NOT_RUN` |
| 恢复工作证据与再次中断后的判定 | 恢复各窗口故障注入（T9a/T9b） | `ADV-P1-5` | `NOT_RUN` |
| 跨主机/网络文件系统互斥 | 网络挂载与同步目录（T11/T12），需项目权限 | `ADV-P1-13` | `NOT_RUN` / `UNKNOWN` |
| 整盘/同相对路径镜像与旧 state 重放是否可检测 | 镜像与重放实验（T5d3/T13b） | `ADV-P1-13` | `NOT_RUN`（设计只声称“不承诺检测”） |
| 真实存档上的 9/10→11 迁移与恢复 | 真实存档（当前禁止；只能合成 fixture） | `ADV-P1-6` | `NOT_RUN` |
| 活动遭遇识别 | 遭遇发现算法与合成遭遇矩阵 | `ADV-P1-10` | `NOT_RUN`（算法未决） |
| #264 根因 | 失败现场采集 | `ADV-P1-12` | `NOT_RUN` |

## 维护规则

- 本文件只经任务分支 + PR 修改；不在会话里口头改计划而不落文件。
- **任务状态、负责人、完成度与“下一个要做什么”只写在 GitHub Issues**；本文件不新增状态列、不复制 Issue 状态、不建第二套台账。若某块需要状态，去看对应 Issue。
- 新任务先作为 Issue 登记（任务包字段与证据矩阵按 [kit/orchestration.md](../kit/orchestration.md) 4.1），再回到本文件登记“顺序、依赖、建议允许路径与 AC”；不先在本文件造任务。
- 顺序调整必须同时更新「阶段」的出口条件与相关任务块的 `Depends on`，并说明依赖是否仍满足；不得只改顺序不改依赖。
- 任务块粒度过粗（一个块含多个可独立验收的结果，或 AC 无法逐条勾验）时先拆块再登记 Issue，不把粗块直接派发。
- 未运行、被阻塞或无法判定的实验一律写 `NOT_RUN` / `BLOCKED` / `UNKNOWN`；不得因为“设计上应该如此”而写成通过，也不得把未实现能力写成已完成。
- 与 `docs/PLAN.md` 的设计目标冲突时，停下对齐并以设计目标为准，不在两处各写一套；需要改变范围、目标、规则支持或一项用户决定时，按 LIVING CHARTER 在同一 PR 更新 `docs/PROJECT-CHARTER.md`。
- 本文件不写日期与排期；任何“什么时候做完”的信息不属于计划文件。
