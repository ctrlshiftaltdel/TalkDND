# PLAN — TalkDND 设计目标

> 本文定义产品要解决的问题、设计边界和验收方式。审阅基准为 2026-11-25 的 GitHub `main`，提交 [`5f112b1`](https://github.com/ctrlshiftaltdel/TalkDND/commit/5f112b16324b0761dc8ea7d43f43faaff8cb4846)，公开发布基准为 [v1.6.0](https://github.com/ctrlshiftaltdel/TalkDND/releases/tag/v1.6.0)。产品决定以[项目章程](PROJECT-CHARTER.md)的 Decision Log 为准。文末索引记录每条 Issue 和 PR 对本文的影响，实时状态和取证细节见 GitHub 原记录。

## 设计目标

玩家在 `LetsPlay/` 启动本地 Agent，在聊天中从开局、建卡、探索和战斗走到存档，并能换一个会话继续。AI DM 管叙述、NPC、选项、自定义行动和规则裁定的表达。玩家决定角色行动、成长选择和反应。程序负责掷骰、引擎已支持的机械结算、校验和存档。DM 应能说清用了哪版规则、查到了什么依据，以及哪些结果需要人工裁定。

| 能力 | 设计要求 | 截至审阅基准的交付与缺口 |
| --- | --- | --- |
| 游玩与战役连续性 | `talkdnd` 负责开局摘要、玩家主导建卡、同伴、日志、检查点、结构化差异和恢复。DM 可在既定偏离范围内即兴发挥。 | 已有游玩 CLI、角色卡 Markdown、常驻队伍状态、会话连续性和恢复。可打印 PDF/HTML 角色卡及空白模板已按决定撤销。待用干净发布包验收一次完整的开局至跨会话续档流程。[#37](https://github.com/ctrlshiftaltdel/TalkDND/pull/37) [#81](https://github.com/ctrlshiftaltdel/TalkDND/pull/81) [#126](https://github.com/ctrlshiftaltdel/TalkDND/pull/126) |
| 骰子、战斗与空间 | 统一 Rust 引擎生成所有骰子并在本地记录完整算式和每颗骰子。玩家视图按该掷骰的可见范围显示。攻击、伤害、条件、专注、移动和借机攻击走规则引擎。空间查询和两版差异由数据驱动。 | 两版核心动作、条件、Ready、感官、空间、飞行和坠落已交付。战术 Utility/Search 为库级显式选项，默认继续用 legacy，改为默认前须证明队战、目标完成和延迟均无回归。[#15](https://github.com/ctrlshiftaltdel/TalkDND/pull/15) [#211](https://github.com/ctrlshiftaltdel/TalkDND/pull/211) [#215](https://github.com/ctrlshiftaltdel/TalkDND/pull/215) [#235](https://github.com/ctrlshiftaltdel/TalkDND/pull/235) |
| 四种规则集与资料 | 一个存档选 5e SRD、5.5e SRD、5e FULL 或 5.5e FULL，仅加载对应版次。PDF 和索引保留本地，合规 FULL 内容包单独分发并在本地解锁。 | SRD 包、FULL 构建、解锁和隔离逻辑已实现。v1.6.0 的公开资产只有三平台游玩包及校验文件，FULL 的资产交付仍缺公开证据。[#193](https://github.com/ctrlshiftaltdel/TalkDND/pull/193) [#203](https://github.com/ctrlshiftaltdel/TalkDND/pull/203) [#207](https://github.com/ctrlshiftaltdel/TalkDND/pull/207) [#209](https://github.com/ctrlshiftaltdel/TalkDND/pull/209) [v1.6.0 资产](https://github.com/ctrlshiftaltdel/TalkDND/releases/tag/v1.6.0) |
| DM 知识、保密与自动化 | 知识查询读本地来源，默认只显示玩家已知内容。每档固定类别的自动化设置只改变何时提示、询问或代结算。玩家选择和反应始终由玩家决定。 | `knowledge` 五个入口和存档默认、场景/遭遇覆盖已交付。部分其他表面只读命令的完整性审计或缓存行为仍需逐入口核对。[#219](https://github.com/ctrlshiftaltdel/TalkDND/pull/219) [#223](https://github.com/ctrlshiftaltdel/TalkDND/pull/223) [#280](https://github.com/ctrlshiftaltdel/TalkDND/pull/280) [#284](https://github.com/ctrlshiftaltdel/TalkDND/pull/284) [#285](https://github.com/ctrlshiftaltdel/TalkDND/issues/285) |
| 施法 | 用结构化法术、角色法术列表和一条战斗执行路径。保留旧自由文本，明确程序与 DM 的处理边界。 | v1.6.0 已发布 schema 10、列表 CLI、战斗施法和两版手写法术数据。随包的 300 多个 SRD 法术仍缺结构化元数据，高级效果与若干施法限制仍需人工裁定。[#241](https://github.com/ctrlshiftaltdel/TalkDND/pull/241) [#243](https://github.com/ctrlshiftaltdel/TalkDND/pull/243) [#245](https://github.com/ctrlshiftaltdel/TalkDND/pull/245) [#249](https://github.com/ctrlshiftaltdel/TalkDND/pull/249) [#251](https://github.com/ctrlshiftaltdel/TalkDND/pull/251) |
| 单职业成长 | 让玩家看见可选机会、了解资格并显式选择。可靠更新派生数值，保留手改和未知历史，提供可恢复的迁移与提交。 | 主分支已有三态校验库、身份诊断、写者保护、提交与显式恢复的库级实现。当前主 schema 仍为 10。schema 11、玩家预览、选择、派生重算和成长提交 CLI 尚需交付。[#261](https://github.com/ctrlshiftaltdel/TalkDND/pull/261) [#276](https://github.com/ctrlshiftaltdel/TalkDND/pull/276) [#310](https://github.com/ctrlshiftaltdel/TalkDND/pull/310) [#312](https://github.com/ctrlshiftaltdel/TalkDND/pull/312) [阶段计划](ADVANCEMENT-PHASE-1-PLAN.md) |

## 设计逻辑

**沿数据的责任链设计。** 选择规则集 → 载入同版规则和存档 → 本地查询与可见性过滤 → 玩家决策 → Rust 结算 → 记录结果并校验 → 后续会话恢复。每一环都有明确的读写责任。规则、内容和存档证据能追溯到来源。

### 游玩、知识和自动化

- `LetsPlay/` 是玩家入口，自包含，二进制或显式根目录能从任意工作目录定位它。根目录用于开发。游戏资料按一本书一个 `<slug>/` 目录存放，pdf2md 生成本地 `index/`。冒险书定义既有事实，DM 在每档创建时确定的偏离范围内组织表现与原创内容，用仅 DM 可见的台账控制类型、稀有度和章节预算。
- `talkdnd` 管理建卡、角色卡、同伴控制者、战役日志、世界时间、实体关系、线索和检查点。`combat-sim` 管战斗状态和同档队伍状态。保存、恢复和迁移按[数据权威矩阵](../LetsPlay/docs/campaign-continuity.md)写入，防止同一事实出现在多个互不一致的写入路径。
- `talkdnd knowledge search/get/cite/context/related` 用稳定 ID、确定性排序和固定来源 `ruling/save/book/srd/adventure/original`。默认隐藏未来剧情、未识别名与原创隐藏面。显式 DM 视图标记 `visibility: dm`。`dm-secrets.md` 内容始终留在秘密边界内。知识查询与 `guard` 共用过滤边界，查询本身保持只读、离线且无持久化索引或秘密缓存。其他命令已有的控制缓存按各命令契约验收。
- 自动化级别为 `manual / assisted / auto`，有效顺序为遭遇 > 场景 > 存档默认。`resolution` 和 `combat` 默认 `auto`，`npc`、`player_choice`、`reaction` 和 `near_limit` 默认 `assisted`。程序校验类别、级别和覆盖 ID，审计改动，接近上限先警告。玩家选择与反应不得设为 `auto`，开关不改变规则、骰子或默认战术 AI。[玩家自动化说明](../LetsPlay/docs/automation.md) · [知识说明](../LetsPlay/docs/knowledge.md)

- 自动游玩反馈在受控 DM 主机上显式开启，记录开局、续档、检查点和战斗反应的结构化结果。记录落在游玩根目录的忽略目录。后台仅向固定 GitHub 仓库发送白名单字段，按阶段和故障码去重，遇到创建结果不明先查询。回滚确认失败立即入队，其余已分类技术故障在最近两场累计三次入队。试点验证与成长迁移分别取证。[任务 #322](https://github.com/ctrlshiftaltdel/TalkDND/issues/322)

### 规则数据、战斗和施法

- 同一 Rust 引擎生成所有骰子、结算战斗及已支持的机械检定。触及按体积，视线和掩护按同一组障碍，区域与目标体积求交，移动逐步校验。飞行与坠落由显式命令处理。启用空间字段的遭遇走统一查询，旧存档保留旧二维结果。
- 擒抱、推撞、徒手、双武器与额外攻击共用动作资源和空间内核。条件的来源、等级、增减和清理可追溯。Ready 的反应只花一次，感官查询中的躲藏与隐形不重复计入。详细回归标准见[战斗规则最低回归](#战斗规则最低回归)。
- 存档的 `ruleset` 值为 `5e-srd`、`5.5e-srd`、`5e-full` 或 `5.5e-full`。SRD 5.1 / 5.2.1 内容包使用各自 edition，按 CC-BY-4.0 随包发布并保留 `NOTICE`。FULL 至少要求一本同版且已解锁的 pack。缺少有效同版 pack 时，`start` 与 `content check` 失败关闭。缺失、仅有错版、未解锁或损坏均须有可理解的提示。解锁根据玩家本地 PDF 的内容识别，PDF hash 不作门槛。旧 `2014` / `2024` 值可读，下一次写入迁移到对应 SRD。
- 施法模型包括法术等级、学派、成分、仪式、范围/持续时间、升环、职业表和子职来源。`talkdnd` 管已知、准备、法术书与戏法，校验建卡、长休、升级和抄书相关规则。保留旧自由文本与脏角色卡。`combat-sim cast/use` 共用执行器校验槽位、声明、专注和已建模升环。契约槽用覆盖后的等级，Mystic Arcanum 用独立长休资源。仪式由 `talkdnd` 校验并记录，不占槽也不升环：2014 按职业特性及 known/prepared/book，2024 按已准备加仪式标签，法师的 Ritual Adept 可用书内未准备法术。
- 已发布版本的人工边界包括未结构化法术、非主骰升环、持续区域和高级效果、V/S 的实际约束、材料库存、同回合限制、跨回合长施法、特性或魔祈授予的仪式。抄书扣金币与写角色卡跨文件，失败时可能需要人工补偿。多职业施法超出现有批准范围。[施法设计与决定](SPELLCASTING.md) · [发布限制](../CHANGELOG.md)

### 成长、迁移和恢复

仅有 Markdown 的旧角色卡经 DM 核对后可在末尾追加一个非秘密身份标记。原有字节逐字保留。角色卡标记与成长记录互相验证，历史选择继续记为未知。只读身份预览可以先查缺失、错配和同档重复，再决定是否进入迁移。

存在结构化角色卡时，由 YAML 承载身份。候选在原字节之后追加顶层字段，再读取并逐项比较既有 YAML 值。无法满足两项要求或发现两处身份来源时，停下核对。

- **读取先于写入。** 预览不锁定、不创建文件，输出一致性未验证和可写性，显示每条前提的“满足 / 不满足 / 未知”、规则版和来源。不满足失败关闭，未知只有 DM 明确确认且有可审计依据才可继续。玩家选择为独立必填项。活动遭遇或无法可靠识别遭遇状态时只允许预览。
- **按明确登记的遭遇生命周期决定能否提交。** 新遭遇记录其自身状态。撤退或中止由 DM 明确结束。旧遭遇没有可验证的状态时，由 DM 核对后登记。活动遭遇、待确认反应、未结清的长施法交接、损坏或归属不明的遭遇都阻止提交。提交在现有存档锁内复核这些状态。
- **身份决定写入归属。** 已选 Q 身份方向与 B/D 存储和恢复方向。下一版主 schema 11 自动迁移 9/10，具体身份字段、令牌随机源与生命周期、无损旧格式、重绑/克隆/回滚语义仍需确定。身份缺失、不一致或同档重复时停止，禁止按名字自动绑定。迁移前校验完整用户数据备份，含既有用户备份。锁、操作前快照和恢复工作记录单独保留。用户手改、未知键、注释及历史未知必须保留，只更新可可靠推导的数值。
- **完整备份验证可恢复的元数据。** 验收核对文件内容、权限、所有者、可恢复的时间戳、ACL、扩展属性和既有用户备份。备份清单记录原始 POSIX `ctime`，供审计核对。任何应恢复的元数据无法保留或验证时，迁移在写入目标前停止。见[锁与恢复契约](ADVANCEMENT-LOCK-DESIGN.md)。
- **写者采用同一协作协议。** `talkdnd` 与 `combat-sim` 共用内部 `advancement-lock`，保持显式初始化与写者过渡拒绝。未初始化的旧档保持原行为。相关新版本写者完成接入后才启用成长提交。新内层锁默认等待 5 秒、可覆盖、超时拒写。旧完整性锁仍可无限阻塞。协议只能约束协作写者，无法保证多文件原子性，无法承诺拦住旧二进制和外部编辑器或检测整档重放。
- **提交和恢复均可核查。** 验证候选和备份后才逐文件发布，记录 intent、done、日志与完整性。重复请求不得重复应用。恢复先保护当前树并记录证据，再由人明确选择回到前像或继续到目标。手改冲突、后续日志或无法确认原操作时停止并交人工。恢复中断后重新审查证据，禁止自动重放。主分支实现已能用库 API 演练这些动作，成长提交和恢复 CLI 尚未向玩家开放。
- **平台范围必须有证据。** 当前批准边界是本地常见文件系统，Windows 本地成长提交失败关闭，日常 Windows 游玩仍受支持。网络盘、FUSE、同步目录不支持成长写入，强制准入检测尚未实现。现有文件系统调查只验证了 Linux 本地 ext4 的部分前提，macOS、Windows、目录崩溃耐久和跨主机互斥不能由该结果外推。[文件系统报告](ADVANCEMENT-CROSS-PLATFORM-FS-INVESTIGATION.md) · [锁与恢复契约](ADVANCEMENT-LOCK-DESIGN.md) · [#264 现场报告](ADVANCEMENT-CHECKPOINT-LOCK-FIELD-CAPTURE.md)

## 范围与约束

- 本地 CLI 支持 Linux x86_64、Windows x86_64、macOS arm64。发布游玩包只含可公开的已跟踪游玩文件、许可和二进制。无账号、托管游戏或网络服务。在引擎和 Git 跟踪数据中只收公开 SRD 或其可分发衍生内容。
- 玩家规则书、冒险书、存档、角色卡、会话日志、DM 秘密和凭据不得进入仓库、公开任务记录或 CI 日志。已授权的 PHB/DMG/MM pack 从本地索引构建，独立于游玩包和 Git 发布。托管 CI 用合成 fixture，既不构建也不获取真实 FULL pack。授权范围变更时按所有者决定撤换资产。
- 语言为 Rust。既有例外是经 uv 运行的 AgentOrch Python 套件与 pdf2md，以及 CI YAML。原有套件校验、上游基线、漂移核查和内容检查继续覆盖仓库。`tools/combat-sim/src` 维持默认只读的约定。四个 Rust crate 版本同步，`advancement-lock` 为不单独发布的内部库。主存档 schema 变更附自动迁移并升次版本或主版本，发布标签有对应 CHANGELOG 小节。
- 配置、规则集、schema、完整性、身份或资格发生无法判定的错误时，给出可定位的原因与恢复路径，防止猜测或静默降级。产品边界变更回写项目章程，任务细节与实际 CI 证据回链 GitHub。

## 推进顺序

| 顺序 | 交付 | 通过条件 |
| --- | --- | --- |
| 1. 修恢复并建立游玩基线 | 修正库级恢复中完整性锁的作用区间。把发布包开局、掷骰、战斗、存档和续档串成一条回归。 | 对照竞争写者验证清单读改写的持锁范围，覆盖继续与回滚。`#264` 目前为 Linux 默认并行 2/41 复现，持锁者未知，单独跟踪。以合成存档记录可重跑的游玩证据。 |
| 2. 交付只读预览 | 复用三态校验和身份诊断，让玩家知道可选机会、条件依据和是否具备写入能力。 | 整树字节一致，边界信息明确。活动或不确定遭遇只允许预览。同步明确身份方案，补平台备份和失败关闭的取证。 |
| 3. 接通单职业成长闭环 | 在身份与平台前提满足后接入 schema 迁移、玩家选择、可靠派生、显式提交和恢复。 | 合成旧档能完整备份、迁移、预览、选择、恰好一次提交和中断恢复。未证实的迁移写入边界保持阻塞。按既有决定，首阶段全面验收后暂停，再审议全量专长或多职业。 |
| 4. 补齐可见的规则和发布缺口 | 补常用施法数据。验收四规则集、秘密过滤、自动化边界与 FULL 的获取、解锁和失败提示。 | 两版采用相同数据契约但读各自内容。真实 FULL 资产按授权渠道单独验收。高级规则另按明确玩家场景和现有总路线排期。 |

步骤 1 的回归与平台调查可以独立推进。Linux、macOS 和 Windows 都已从改名后的 Release 游玩包完成两版 SRD 的合成回归，覆盖包外建档、掷骰、战斗、跨进程处理护盾术、玩家视图名称遮蔽和重复确认拒绝。Linux 和 macOS 的继续恢复与回滚锁竞争用例均已通过。[三平台游玩包及恢复测试](https://github.com/ctrlshiftaltdel/TalkDND/actions/runs/45580891809)

先取得可用的只读体验，写入只覆盖有证据支持的边界。高级法术、怪物与遭遇、探索与危险、社交和经济、可选规则沿 [#183](https://github.com/ctrlshiftaltdel/TalkDND/issues/183) 的主题推进，每组以具体场景和实测覆盖定发布范围。该 Epic 的旧版本编号和多职业范围需与后续决定对齐。当前成长以[首阶段计划](ADVANCEMENT-PHASE-1-PLAN.md)和章程为准。

### 两版规则核对后的实施顺序

1. **修正共享时序、感官与长休语义。** 反应在各角色自己的下个回合开始时恢复。恐惧持续期间限制主动靠近，视线只影响劣势。2024 的震颤感知按共享地面定位，定位不会授予视觉或改写掩护。2014 的遭遇与角色长休要经 DM 确认进食饮水才降低力竭。2014 长休恢复一半生命骰，2024 恢复全部。后续需让角色资料和遭遇共用可续存的力竭等级。
2. **建立玩家反应流程。** 普通战斗 CLI 已为玩家角色接入 Ready、借机攻击和护盾术的两种触发。魔法飞弹可把三枚飞弹分给多个目标，各目标在伤害结算前各选一次。反应资源不足时不提示。用 `reaction <ID> take|pass` 确认或放弃。系统在所有选择完成后提交，续档沿用原骰值，旧 ID 不能重复使用。待确认状态使用兼容性标记让旧版加载遭遇时拒绝继续，选择完成后回到普通战斗状态格式。筹备与续档已跑合成游玩回归。
3. **让 DM 手动裁定接入同一结算状态。** 战斗已按版别记录同回合施法，2014 双向检查附赠动作施法限制，2024 检查实际用槽。准备动作与自动反应共用记录。普通施法和准备法术在消耗动作或资源前要求确认手动效果，确认写入日志。战斗日志和状态可单独隐藏 DC，原骰值、成败和存档保留。长施法由战役与遭遇显式交接，DM 在记录完成后处理法术效果。
4. **统一升级写入。** 只读预览按版别、职业和获授机会列出选择。2014 专长作为可选替代，2024 属性提升作为专长。等级、玩家选择与可靠派生数据进入同一次提交。用体质 17→18 的 8 级场景验证最大 HP 变化，重试只应用一次。

长施法首版支持跨探索与战斗延续同一次施法。战役存档记录施法者、法术和已完成时长。探索推进使用世界时间，战斗内使用回合，并要求施法者按版别持续用动作与维持专注。交接前检查同一施法的标识和已登记进度，避免把同一段时长计两次。专注中断时保留法术位，完成后恰好扣一次，重启后继续同一次施法。未知角色绑定、改动的遭遇或无法确认的扣槽结果一律保留现有数据并报告。

精确时间从旧天数自动换算。检查点将精确时钟和队伍扣费凭据一起恢复。两版的 Mending 都可以走同一流程。仪式资格、尚未生成元数据的法术与高级效果继续由 DM 裁定。

## 可度量的全局验收

1. **干净发布包。** 在规定的三平台解压到可改名的目录，从任意工作目录进入，完成建档、一次真实掷骰、一场合成战斗和跨会话恢复。核对算式、战斗状态、存档路径与恢复前后的结构化差异。应用包的可运行性和 AI DM 的聊天体验分别记录证据。
2. **四规则集与资料。** SRD 无自备 PDF 仍可用且不读 FULL。FULL 的正确同版 pack 可解锁，错版、缺失、损坏及未解锁均拒绝。旧规则集值与旧存档迁移保留原数据。验证普通包和公开日志不夹带版权内容或个人数据。
3. **决策权和保密。** 以同一 fixture 验证知识五入口稳定 ID、排序和可见性，显式 DM 视图标注可见级别。验证自动化覆盖优先级和玩家选择/反应的限制。用失败前后整树比较验证只读和拒写。
4. **存档与规则回归。** 检查点恢复、手改、备份、中断、并发与旧格式各有合成回归。故障注入按平台和 SHA 记录。未知项保留未知，真实掉电、目录耐久、网络互斥与 `#264` 结论只认对应实测证据。
5. **仓库发布质量。** 四个 Rust crate 的测试、格式和 Clippy 检查通过。现有完整套件、预期漂移、无泄露、自包含、pdf2md、双版内容与离线链接检查通过。产出三平台二进制和游玩包，并核对版本、CHANGELOG 与授权 FULL 资产。

### 战斗规则最低回归

- 战斗核心动作在 2014/2024 下都按 `rules.combat_actions` 结算。擒抱/推撞失败与非法防御不消耗错误资源，推撞走强制移动并记录阻挡，额外攻击只由数据决定。擒抱/推撞消耗攻击动作的一次攻击，额外攻击生物可继续剩余攻击。`attack --one`/`--unarmed --one` 只结算一次并保留剩余攻击。旧 combat state 仍以 schema 1 加载。
- 血怒在 HP ≤ 有效最大 HP 一半时即时成立（含 0 HP、治疗与临时 HP）。力竭 1–6 级按存档 edition 生效、长休降级、6 级走正常死亡清理（来源条件、专注与施加的专注条件），等级可由 CLI 设置/查看/移除。不带 `--level` 的旧力竭命令只保留 0 级手动标记。魅惑记录施加者并拒绝攻击（含擒抱/推撞）/伤害性目标，来源死亡清除。恐惧在来源处于视线内时给攻击和属性检定劣势。恐惧持续期间禁止主动靠近，即使来源不可见。来源死亡按条件配置清理。来源敏感条件拒绝未知/歧义来源，旧 `linked` 数据仍参与清理。旧 state 与旧 `condition add/remove` 保持兼容，`STATE_SCHEMA_VERSION` 仍为 1。
- 准备动作只存一个响应和一个触发条件。`ready` 消耗动作，重复/无动作/失能/无反应时失败且不消耗。能力/法术响应在设置时立即扣法术位/资源并开始专注，设置失败不扣动作/资源/专注，触发不再二次扣费或重新开始专注。显式 `trigger` 与自动（敌方进入触及/范围、攻击、使用能力、开始回合）触发都校验有效期、反应与目标合法性，成功只消费一次反应，响应走正常攻击/能力/移动路径。反应在角色自己的下个回合开始时恢复。玩家控制角色在 CLI 中确认或放弃通过合法性检查的响应，续档继续处理同一个待确认事件。未触发/过期/`ready-clear`/死亡/0HP/失能/休眠不退法术位，只结束这次准备开始的专注。下一次自己回合开始过期。旧 combat state 无准备字段仍以 schema 1 加载，旧 `attack --force`/`use` 不变，两版共用 `rules.ready`。
- 感官用 `can_see` 返回清晰/轻遮蔽/重遮蔽/被阻挡并附光照、遮蔽、距离、感官与原因。`dim` 或轻遮蔽为轻遮蔽，`dark` 或重遮蔽为重遮蔽。黑暗视觉在范围内把黑暗当微光、微光当明亮，不穿透物理遮蔽。盲视忽略光照/遮蔽但仍受全掩护。2024 的震颤感知定位共享地面内的目标，视线、攻击劣势与掩护继续独立判定。飞行、悬浮和虚体不满足地面条件。真实视觉看穿普通黑暗与 `invisible`，不看穿普通 `hidden` 或全掩护。攻击、能力攻击与 AI 评估在攻击者看不见目标时给劣势、目标看不见攻击者时给优势并记录原因，`hidden`/`invisible` 条件不重复计算。`status --json` 显示原始 `senses` 与解析后的 vision，文本 status 保持一行。`Scene`/`Zone.obscured` 可省略、网格用场景级，旧内容/旧 state 以 schema 1 加载，不改 save-meta、不加 talkdnd 迁移。

## Issue 与 PR 对照

以下为基线快照，共 **159 条 Issue、157 条 PR**。对应的需求、决定与实现尽量并排。调查、重复条目、测试和发布维护保留其实际影响。全部 157 个 PR 的合并提交均可从本基线追溯。已合并的库、调查或文档不自动记为玩家功能。撤销或被后续决定替换的行为保留历史出处。

基线后的草稿 [PR #317](https://github.com/ctrlshiftaltdel/TalkDND/pull/317) 汇总独立分支的规则核对和实现。它增加玩家反应、护盾术对魔法飞弹的触发、两版同回合施法、手动确认、跨探索与战斗的长施法，以及为成长提交提供依据的显式遭遇登记与结束。发布入口使用三平台游玩包复核。

<details>
<summary>1–85｜游玩入口、规则基础与本地书籍</summary>

| 记录 | 增加、变更或保留的设计内容 |
| --- | --- |
| [Issue #1](https://github.com/ctrlshiftaltdel/TalkDND/issues/1) / [PR #2](https://github.com/ctrlshiftaltdel/TalkDND/pull/2) | 建立自包含的 LetsPlay 游玩目录、Rust 技术边界和内容泄漏检查。保留 GitHub 版 AgentOrch 的接入历史。 |
| [Issue #3](https://github.com/ctrlshiftaltdel/TalkDND/issues/3) / [PR #4](https://github.com/ctrlshiftaltdel/TalkDND/pull/4) | 发布工作流为 Linux x64、Windows x64、macOS arm64 预编译 combat-sim。 |
| [Issue #5](https://github.com/ctrlshiftaltdel/TalkDND/issues/5) / [PR #7](https://github.com/ctrlshiftaltdel/TalkDND/pull/7) | 建立活章程与产品决定记录，后续范围、规则和技术决定可追溯。 |
| [Issue #6](https://github.com/ctrlshiftaltdel/TalkDND/issues/6) / [PR #8](https://github.com/ctrlshiftaltdel/TalkDND/pull/8) | CI 扩至格式、Clippy、三平台回归、依赖审计、内容及文档检查。主分支产出可下载二进制，fetch-binary 安装到游玩目录。 |
| [Issue #9](https://github.com/ctrlshiftaltdel/TalkDND/issues/9) / [PR #10](https://github.com/ctrlshiftaltdel/TalkDND/pull/10) | 发布包由单个 combat-sim 扩为跟踪的 LetsPlay 内容和二进制。 |
| [Issue #11](https://github.com/ctrlshiftaltdel/TalkDND/issues/11) / [PR #15](https://github.com/ctrlshiftaltdel/TalkDND/pull/15) | 统一为 combat-sim roll 掷骰，DM 显示算式与骰值。 |
| [Issue #12](https://github.com/ctrlshiftaltdel/TalkDND/issues/12) / [PR #16](https://github.com/ctrlshiftaltdel/TalkDND/pull/16) | 明确冒险事实的权威来源与 DM 即兴表述、NPC 反应的空间。 |
| [Issue #13](https://github.com/ctrlshiftaltdel/TalkDND/issues/13) / [PR #14](https://github.com/ctrlshiftaltdel/TalkDND/pull/14) | 稳定缺少 gh 的安装前置条件回归，避免测试工件 Text file busy 干扰结论。 |
| [Issue #17](https://github.com/ctrlshiftaltdel/TalkDND/issues/17) / [PR #18](https://github.com/ctrlshiftaltdel/TalkDND/pull/18) | 建立本地后台 PDF 转 Markdown 与 OCR 流程，pdf2md 成为经 uv 管理的游玩工具。 |
| [Issue #19](https://github.com/ctrlshiftaltdel/TalkDND/issues/19) / [PR #20](https://github.com/ctrlshiftaltdel/TalkDND/pull/20) | 把套件 Python 示例和脚本入口统一到 uv run。 |
| [Issue #21](https://github.com/ctrlshiftaltdel/TalkDND/issues/21) / [PR #22](https://github.com/ctrlshiftaltdel/TalkDND/pull/22) | pdf2md 0.2.0 修复地图崩溃、OCR 噪声、多栏/空白回退、标题字形和按页选择转换。 |
| [Issue #23](https://github.com/ctrlshiftaltdel/TalkDND/issues/23) / [PR #24](https://github.com/ctrlshiftaltdel/TalkDND/pull/24) | pdf2md 0.2.1 保留回退页标题，升级只重转相关页。 |
| [PR #25](https://github.com/ctrlshiftaltdel/TalkDND/pull/25) | pdf2md 0.2.2 用页内正文占比和相对字号识别标题，针对回退 chunk 选择性升级。 |
| [Issue #26](https://github.com/ctrlshiftaltdel/TalkDND/issues/26) / [PR #27](https://github.com/ctrlshiftaltdel/TalkDND/pull/27) | 修复回退正文被大量误提为标题，pdf2md 0.2.3 收紧标题判定。 |
| [Issue #28](https://github.com/ctrlshiftaltdel/TalkDND/issues/28) / [PR #29](https://github.com/ctrlshiftaltdel/TalkDND/pull/29) | 增加页级审计、修正层和可选 Agent 复核，保留对原始转换的追溯。 |
| [PR #30](https://github.com/ctrlshiftaltdel/TalkDND/pull/30) | 修复 Windows 下 review --engine cmd 路径反斜杠和引号的解析。 |
| [PR #31](https://github.com/ctrlshiftaltdel/TalkDND/pull/31) | 审计召回评分在对照前归一化行末断词，避免误拒正确合并的词。 |
| [Issue #32](https://github.com/ctrlshiftaltdel/TalkDND/issues/32) / [PR #33](https://github.com/ctrlshiftaltdel/TalkDND/pull/33) | 增加 2014 SRD 怪物、法术与支援型 AI 数据，补齐旧版战斗入门。 |
| [Issue #34](https://github.com/ctrlshiftaltdel/TalkDND/issues/34) / [PR #35](https://github.com/ctrlshiftaltdel/TalkDND/pull/35) | pdf2md 复核采用软门、最大轮次重试，允许本地 SRD 片段辅助校对。 |
| [Issue #36](https://github.com/ctrlshiftaltdel/TalkDND/issues/36) / [PR #37](https://github.com/ctrlshiftaltdel/TalkDND/pull/37) | 交付 talkdnd 的 new、start、chargen、npc、checkpoint 和 log 游玩入口。 |
| [Issue #38](https://github.com/ctrlshiftaltdel/TalkDND/issues/38) / [PR #39](https://github.com/ctrlshiftaltdel/TalkDND/pull/39) | 把玩家指令与强制玩家主导建卡放在短 DM 手册，详细操作下沉到游玩文档。 |
| [Issue #40](https://github.com/ctrlshiftaltdel/TalkDND/issues/40) / [PR #41](https://github.com/ctrlshiftaltdel/TalkDND/pull/41) | 建卡按 SRD 和本地书数据生成攻击、职业特性和资源，接入本地导入。 |
| [Issue #42](https://github.com/ctrlshiftaltdel/TalkDND/issues/42) / [PR #49](https://github.com/ctrlshiftaltdel/TalkDND/pull/49) | 补建卡校验：起始等级、2014 一级子职、护甲熟练、法术数量与种族特性。 |
| [Issue #43](https://github.com/ctrlshiftaltdel/TalkDND/issues/43) / [PR #44](https://github.com/ctrlshiftaltdel/TalkDND/pull/44) | 修正规则：回合外施法、短休生命骰、长休结束专注、全队休息、同伴死亡豁免和 NPC 等级。 |
| [Issue #45](https://github.com/ctrlshiftaltdel/TalkDND/issues/45) / [PR #46](https://github.com/ctrlshiftaltdel/TalkDND/pull/46) | 同档持久化 HP、槽位、资源与条件，由战斗引擎读写队伍状态。 |
| [Issue #47](https://github.com/ctrlshiftaltdel/TalkDND/issues/47) / [PR #48](https://github.com/ctrlshiftaltdel/TalkDND/pull/48) | talkdnd check 从角色卡结算战斗外检定和豁免。 |
| [Issue #50](https://github.com/ctrlshiftaltdel/TalkDND/issues/50) / [PR #51](https://github.com/ctrlshiftaltdel/TalkDND/pull/51) | v0.1.2 补开局书路径、重复技能提示、死亡条件、示例卡、秘密清单与怪物回合说明。 |
| [Issue #52](https://github.com/ctrlshiftaltdel/TalkDND/issues/52) / [PR #53](https://github.com/ctrlshiftaltdel/TalkDND/pull/53) | pdf2md 0.3.0 增加校对报告、原位插图和地图、房间索引。 |
| [Issue #54](https://github.com/ctrlshiftaltdel/TalkDND/issues/54) / [PR #55](https://github.com/ctrlshiftaltdel/TalkDND/pull/55) | 支持从 2014 PHB 本地索引读取子职排版。 |
| [Issue #56](https://github.com/ctrlshiftaltdel/TalkDND/issues/56) / [PR #57](https://github.com/ctrlshiftaltdel/TalkDND/pull/57) | 修复房间标题、朗读引文、粘连与断空格，页修正后刷新冒险索引。 |
| [Issue #58](https://github.com/ctrlshiftaltdel/TalkDND/issues/58) / [PR #59](https://github.com/ctrlshiftaltdel/TalkDND/pull/59) | 补齐 2014 PHB 子职导入时的剩余识别覆盖。 |
| [Issue #60](https://github.com/ctrlshiftaltdel/TalkDND/issues/60) / [PR #61](https://github.com/ctrlshiftaltdel/TalkDND/pull/61) | 批量导入 SRD 5.1 怪物与法术，增加快速遭遇入口。 |
| [Issue #62](https://github.com/ctrlshiftaltdel/TalkDND/issues/62) / [PR #63](https://github.com/ctrlshiftaltdel/TalkDND/pull/63) | 可记录经验、升级和战利品/金币，进度保存在同一档。 |
| [Issue #64](https://github.com/ctrlshiftaltdel/TalkDND/issues/64) / [PR #65](https://github.com/ctrlshiftaltdel/TalkDND/pull/65) | 统一 v0.1.4 发布的 crate 版本。 |
| [Issue #66](https://github.com/ctrlshiftaltdel/TalkDND/issues/66) / [PR #67](https://github.com/ctrlshiftaltdel/TalkDND/pull/67) | 加载器略过非内容文件，摘要仅列书籍 Markdown 与索引，发布 v0.1.5。 |
| [PR #68](https://github.com/ctrlshiftaltdel/TalkDND/pull/68) / [Issue #69](https://github.com/ctrlshiftaltdel/TalkDND/issues/69) | 历史交付可编辑和打印的 A4 PDF/HTML 角色卡，v0.1.6 提供导出。后续由 #81 撤销。 |
| [Issue #70](https://github.com/ctrlshiftaltdel/TalkDND/issues/70) / [PR #71](https://github.com/ctrlshiftaltdel/TalkDND/pull/71) | 战斗规则按 campaign ruleset 选 2014 或 2024 子目录。 |
| [Issue #72](https://github.com/ctrlshiftaltdel/TalkDND/issues/72) / [PR #73](https://github.com/ctrlshiftaltdel/TalkDND/pull/73) | v0.1.7 支持用 XP 记录事件、以里程碑提升等级的混合推进。 |
| [Issue #74](https://github.com/ctrlshiftaltdel/TalkDND/issues/74) / [PR #75](https://github.com/ctrlshiftaltdel/TalkDND/pull/75) | pdf2md 0.4.0 提供连贯阅读版与按页定位版。 |
| [PR #76](https://github.com/ctrlshiftaltdel/TalkDND/pull/76) | 历史交付角色卡 v2 和空白版，强化资源、法术和笔记布局。后续由 #81 撤销。 |
| [Issue #77](https://github.com/ctrlshiftaltdel/TalkDND/issues/77) / [PR #78](https://github.com/ctrlshiftaltdel/TalkDND/pull/78) | pdf2md 0.4.1 解析并校验数据块，用警告标记提示不一致。 |
| [Issue #79](https://github.com/ctrlshiftaltdel/TalkDND/issues/79) / [PR #80](https://github.com/ctrlshiftaltdel/TalkDND/pull/80) | pdf2md 0.5.0 从扫描页提取 WebP 插图，生成接触表。 |
| [PR #81](https://github.com/ctrlshiftaltdel/TalkDND/pull/81) | 撤销 PDF/HTML 导出与空白卡，去掉渲染依赖，保留 chargen 的 Markdown 角色卡。 |
| [Issue #82](https://github.com/ctrlshiftaltdel/TalkDND/issues/82) / [PR #83](https://github.com/ctrlshiftaltdel/TalkDND/pull/83) | 补充阅读版、逐页版、数据块检查和图片处理的使用说明。 |
| [Issue #84](https://github.com/ctrlshiftaltdel/TalkDND/issues/84) / [PR #85](https://github.com/ctrlshiftaltdel/TalkDND/pull/85) | 发布 v0.2.0，归档这一轮游玩工具和资料转换变更。 |

</details>

<details>
<summary>86–166｜战役连续性、隐私、恢复与稳定版</summary>

| 记录 | 增加、变更或保留的设计内容 |
| --- | --- |
| [Issue #86](https://github.com/ctrlshiftaltdel/TalkDND/issues/86) / [PR #87](https://github.com/ctrlshiftaltdel/TalkDND/pull/87) | v0.3.0 引入桌规、存档完整性清单、日志对账和防剧透边界。 |
| [Issue #88](https://github.com/ctrlshiftaltdel/TalkDND/issues/88) / [PR #89](https://github.com/ctrlshiftaltdel/TalkDND/pull/89) | v0.4.0 增加结构化角色卡字段及 char 命令，当时的规则书优先策略后来由四规则集细化。 |
| [Issue #90](https://github.com/ctrlshiftaltdel/TalkDND/issues/90) / [PR #91](https://github.com/ctrlshiftaltdel/TalkDND/pull/91) | v0.5.0 修复迁移后 AC 与 Defense 重算，并按玩家可见性提供本地书籍 handout。 |
| [Issue #92](https://github.com/ctrlshiftaltdel/TalkDND/issues/92) / [PR #93](https://github.com/ctrlshiftaltdel/TalkDND/pull/93) | v0.6.0 增加自检、覆盖升级时自动迁移、发布版本规范和 CHANGELOG。 |
| [Issue #94](https://github.com/ctrlshiftaltdel/TalkDND/issues/94) / [PR #95](https://github.com/ctrlshiftaltdel/TalkDND/pull/95) | v0.6.1 发布包顶层改为 TalkDND，任意目录名和启动路径可定位游玩根。 |
| [Issue #96](https://github.com/ctrlshiftaltdel/TalkDND/issues/96) / [PR #97](https://github.com/ctrlshiftaltdel/TalkDND/pull/97) | v0.7.0 在每档定义偏离范围和比例，原创内容按仅 DM 可见台账记录。 |
| [Issue #98](https://github.com/ctrlshiftaltdel/TalkDND/issues/98) / [PR #99](https://github.com/ctrlshiftaltdel/TalkDND/pull/99) | v0.8.0 建立按场景阅读、玩家已知、秘密 guard、名字遮蔽、选项与骰子展示规则。 |
| [Issue #100](https://github.com/ctrlshiftaltdel/TalkDND/issues/100) / [PR #101](https://github.com/ctrlshiftaltdel/TalkDND/pull/101) | v0.9.0 将同伴纳入 Characters，使用 controller 表达谁决定或操作。 |
| [Issue #102](https://github.com/ctrlshiftaltdel/TalkDND/issues/102) / [PR #103](https://github.com/ctrlshiftaltdel/TalkDND/pull/103) | v0.10.0 收口为一本书一个目录和单一分页 Markdown，整理书库结构。 |
| [Issue #104](https://github.com/ctrlshiftaltdel/TalkDND/issues/104) / [PR #105](https://github.com/ctrlshiftaltdel/TalkDND/pull/105) | 校正 DM/开发文档引用的命令与二进制，补仓库地图。 |
| [Issue #106](https://github.com/ctrlshiftaltdel/TalkDND/issues/106) / [PR #108](https://github.com/ctrlshiftaltdel/TalkDND/pull/108) | 游玩中可修正单页并即时重组索引。 |
| [Issue #107](https://github.com/ctrlshiftaltdel/TalkDND/issues/107) / [PR #109](https://github.com/ctrlshiftaltdel/TalkDND/pull/109) | library merge 在整书与分卷并存时归档旧分卷。 |
| [Issue #110](https://github.com/ctrlshiftaltdel/TalkDND/issues/110) / [PR #111](https://github.com/ctrlshiftaltdel/TalkDND/pull/111) | v0.11.0 汇合页修正、书库整理和命令契约。 |
| [Issue #112](https://github.com/ctrlshiftaltdel/TalkDND/issues/112) / [Issue #113](https://github.com/ctrlshiftaltdel/TalkDND/issues/113) / [PR #122](https://github.com/ctrlshiftaltdel/TalkDND/pull/122) | 确立 P00–P08 数据权威、可见性、版本、写入入口与恢复矩阵。两条同名 Issue 纳入同一契约。 |
| [Issue #114](https://github.com/ctrlshiftaltdel/TalkDND/issues/114) / [PR #124](https://github.com/ctrlshiftaltdel/TalkDND/pull/124) | 为连续性能力建立九组 CLI 及模块骨架，实际功能由后续交付。 |
| [Issue #115](https://github.com/ctrlshiftaltdel/TalkDND/issues/115) / [PR #123](https://github.com/ctrlshiftaltdel/TalkDND/pull/123) | 将连续性 sidecar 与检查点清单纳入完整性校验。 |
| [Issue #116](https://github.com/ctrlshiftaltdel/TalkDND/issues/116) / [PR #126](https://github.com/ctrlshiftaltdel/TalkDND/pull/126) | 建立状态历史、检查点、结构化差异和恢复，支持跨会话续接。 |
| [Issue #117](https://github.com/ctrlshiftaltdel/TalkDND/issues/117) / [PR #125](https://github.com/ctrlshiftaltdel/TalkDND/pull/125) | 交付结构化冒险索引与本地查找，支持更快导航。 |
| [Issue #118](https://github.com/ctrlshiftaltdel/TalkDND/issues/118) / [PR #132](https://github.com/ctrlshiftaltdel/TalkDND/pull/132) | 将固定 DM 风格和记忆纳入存档，跨会话可重用。 |
| [Issue #119](https://github.com/ctrlshiftaltdel/TalkDND/issues/119) / [PR #133](https://github.com/ctrlshiftaltdel/TalkDND/pull/133) | 建立稳定实体、时间感知关系及其记录和查询。 |
| [Issue #120](https://github.com/ctrlshiftaltdel/TalkDND/issues/120) / [PR #139](https://github.com/ctrlshiftaltdel/TalkDND/pull/139) | 交付简化世界时间和可见的预定事件。 |
| [Issue #121](https://github.com/ctrlshiftaltdel/TalkDND/issues/121) / [PR #140](https://github.com/ctrlshiftaltdel/TalkDND/pull/140) | 增加即兴 Oracle 和扩展的 DM 规划模板。 |
| [Issue #127](https://github.com/ctrlshiftaltdel/TalkDND/issues/127) / [PR #135](https://github.com/ctrlshiftaltdel/TalkDND/pull/135) | 默认拒绝回合外攻击，DM 可用显式 --force 并形成可审计记录。 |
| [Issue #128](https://github.com/ctrlshiftaltdel/TalkDND/issues/128) / [PR #134](https://github.com/ctrlshiftaltdel/TalkDND/pull/134) | 战斗 state JSON 纳入完整性，成功写入同步更新基线。 |
| [Issue #129](https://github.com/ctrlshiftaltdel/TalkDND/issues/129) / [PR #137](https://github.com/ctrlshiftaltdel/TalkDND/pull/137) | combat-sim add 在战斗中追加参与者，保留轮次、既有状态和日志。 |
| [Issue #130](https://github.com/ctrlshiftaltdel/TalkDND/issues/130) / [PR #136](https://github.com/ctrlshiftaltdel/TalkDND/pull/136) | check 支持工具熟练检定，显式选择属性，熟练只按数据加成。 |
| [Issue #131](https://github.com/ctrlshiftaltdel/TalkDND/issues/131) / [PR #138](https://github.com/ctrlshiftaltdel/TalkDND/pull/138) | 增加秘密标记只读扫描、显式导入和开局零标记提示。 |
| [Issue #141](https://github.com/ctrlshiftaltdel/TalkDND/issues/141) / [PR #142](https://github.com/ctrlshiftaltdel/TalkDND/pull/142) | 发布 v1.0.0，串起 P00–P08 和稳定版游玩要求。 |
| [Issue #143](https://github.com/ctrlshiftaltdel/TalkDND/issues/143) / [PR #146](https://github.com/ctrlshiftaltdel/TalkDND/pull/146) | 核实 §3.4 跨引用审查意见和恢复回滚，用真实证据确定修正项。 |
| [Issue #144](https://github.com/ctrlshiftaltdel/TalkDND/issues/144) | 要求在实际 Windows Terminal/PowerShell 上验收最终 v1.0.1 候选包，区别于一般三平台 CI。 |
| [Issue #145](https://github.com/ctrlshiftaltdel/TalkDND/issues/145) / [PR #147](https://github.com/ctrlshiftaltdel/TalkDND/pull/147) | 在质量审查和实机验证后发布 v1.0.1，更新版本、日志与章程。 |
| [Issue #148](https://github.com/ctrlshiftaltdel/TalkDND/issues/148) / [PR #150](https://github.com/ctrlshiftaltdel/TalkDND/pull/150) | 恢复失败后按原备份镜像回滚，清理备份外残留文件。 |
| [Issue #149](https://github.com/ctrlshiftaltdel/TalkDND/issues/149) / [PR #151](https://github.com/ctrlshiftaltdel/TalkDND/pull/151) | 串行化完整性清单读改写，防止并发更新丢失指纹。 |
| [Issue #152](https://github.com/ctrlshiftaltdel/TalkDND/issues/152) / [PR #153](https://github.com/ctrlshiftaltdel/TalkDND/pull/153) | 发布 v1.0.2，包含恢复回滚和完整性并发修复。 |
| [Issue #154](https://github.com/ctrlshiftaltdel/TalkDND/issues/154) / [PR #155](https://github.com/ctrlshiftaltdel/TalkDND/pull/155) | checkpoint restore 和 migrate 内联写清单共用完整性锁。 |
| [Issue #156](https://github.com/ctrlshiftaltdel/TalkDND/issues/156) | 误建的回滚审查 Issue。保留溯源，相关实质修正见 PR #158。 |
| [Issue #157](https://github.com/ctrlshiftaltdel/TalkDND/issues/157) | 误建的恢复后处理 Issue。保留溯源，相关实质修正见 PR #160。 |
| [PR #158](https://github.com/ctrlshiftaltdel/TalkDND/pull/158) | 回滚保留活动完整性锁。备份不完整时拒绝恢复并报告不能读取的内容。 |
| [Issue #159](https://github.com/ctrlshiftaltdel/TalkDND/issues/159) / [PR #161](https://github.com/ctrlshiftaltdel/TalkDND/pull/161) | 稳定完整性并发测试的时序，保留并发断言。 |
| [PR #160](https://github.com/ctrlshiftaltdel/TalkDND/pull/160) | 文件提交后迁移、追加日志或重设基线失败时，报告阶段、检查点、备份及可能已改动状态。 |
| [Issue #162](https://github.com/ctrlshiftaltdel/TalkDND/issues/162) / [PR #164](https://github.com/ctrlshiftaltdel/TalkDND/pull/164) | 每次恢复使用独立备份目录，避免同秒复用并污染原备份。 |
| [Issue #163](https://github.com/ctrlshiftaltdel/TalkDND/issues/163) | 调查备份复制完整性锁与回滚排除规则，区分契约偏差与已经证实的故障。 |
| [Issue #165](https://github.com/ctrlshiftaltdel/TalkDND/issues/165) / [PR #166](https://github.com/ctrlshiftaltdel/TalkDND/pull/166) | 发布 v1.0.3，汇入恢复、锁与错误报告修正。 |

</details>

<details>
<summary>167–235｜空间、内容平台、知识与战斗</summary>

| 记录 | 增加、变更或保留的设计内容 |
| --- | --- |
| [PR #167](https://github.com/ctrlshiftaltdel/TalkDND/pull/167) / [Issue #168](https://github.com/ctrlshiftaltdel/TalkDND/issues/168) | 建立空间和战术 AI 路线图，定义统一查询、移动、飞行、AI 渐进启用和旧档兼容。 |
| [Issue #169](https://github.com/ctrlshiftaltdel/TalkDND/issues/169) / [PR #197](https://github.com/ctrlshiftaltdel/TalkDND/pull/197) | 冻结两版空间规则矩阵、兼容约定与统一空间查询契约。 |
| [Issue #170](https://github.com/ctrlshiftaltdel/TalkDND/issues/170) / [PR #204](https://github.com/ctrlshiftaltdel/TalkDND/pull/204) | 增加可选空间基础类型和序列化字段，旧存档保留原语义。 |
| [Issue #171](https://github.com/ctrlshiftaltdel/TalkDND/issues/171) / [PR #206](https://github.com/ctrlshiftaltdel/TalkDND/pull/206) | 统一按体积查询距离、触及、视线、掩护和区域交集。 |
| [Issue #172](https://github.com/ctrlshiftaltdel/TalkDND/issues/172) / [PR #208](https://github.com/ctrlshiftaltdel/TalkDND/pull/208) | 实现占位感知移动边、Dijkstra 可达性与 A* 寻路。 |
| [Issue #173](https://github.com/ctrlshiftaltdel/TalkDND/issues/173) / [PR #210](https://github.com/ctrlshiftaltdel/TalkDND/pull/210) | 实现垂直移动、飞行支撑丧失与坠落结算。 |
| [Issue #174](https://github.com/ctrlshiftaltdel/TalkDND/issues/174) / [PR #211](https://github.com/ctrlshiftaltdel/TalkDND/pull/211) | 将空间内核用于战斗、CLI、借机、触发器与状态查询，旧档保留二维路径。 |
| [Issue #175](https://github.com/ctrlshiftaltdel/TalkDND/issues/175) / [PR #212](https://github.com/ctrlshiftaltdel/TalkDND/pull/212) | 构建可解释的 Utility 战术候选、目标和评分，提供显式选择。 |
| [Issue #176](https://github.com/ctrlshiftaltdel/TalkDND/issues/176) / [PR #214](https://github.com/ctrlshiftaltdel/TalkDND/pull/214) | 实现预算约束的概率搜索、对抗回应和超预算回退。 |
| [Issue #177](https://github.com/ctrlshiftaltdel/TalkDND/issues/177) / [PR #215](https://github.com/ctrlshiftaltdel/TalkDND/pull/215) | 用回归和基准考察合法性、队战、目标及延迟，Utility/Search 保持显式选用，legacy 继续为默认。 |
| [Issue #178](https://github.com/ctrlshiftaltdel/TalkDND/issues/178) / [PR #205](https://github.com/ctrlshiftaltdel/TalkDND/pull/205) | 补公开结构化空间内容：悬浮、掘地、区域形状和怪物覆盖。 |
| [Issue #179](https://github.com/ctrlshiftaltdel/TalkDND/issues/179) / [PR #213](https://github.com/ctrlshiftaltdel/TalkDND/pull/213) | 探测快速推演和搜索性能，用可测预算约束后续规划器。 |
| [Issue #180](https://github.com/ctrlshiftaltdel/TalkDND/issues/180) / [PR #202](https://github.com/ctrlshiftaltdel/TalkDND/pull/202) | 同步路线图与冻结的契约，分开合成几何校验、内容补齐和两版移动差异。 |
| [Issue #181](https://github.com/ctrlshiftaltdel/TalkDND/issues/181) / [PR #188](https://github.com/ctrlshiftaltdel/TalkDND/pull/188) | 引入 ContentPack 格式、来源及加载器，支持规则内容与引擎解耦。 |
| [Issue #182](https://github.com/ctrlshiftaltdel/TalkDND/issues/182) / [PR #193](https://github.com/ctrlshiftaltdel/TalkDND/pull/193) | 导入两版 SRD 5.1 / 5.2.1 内容包，保留署名和来源。 |
| [Issue #183](https://github.com/ctrlshiftaltdel/TalkDND/issues/183) | 总 Epic 将内容、自动化、知识和规则分组推进。其中旧版本顺序及多职业预期由后续已定范围校正。 |
| [Issue #184](https://github.com/ctrlshiftaltdel/TalkDND/issues/184) / [PR #185](https://github.com/ctrlshiftaltdel/TalkDND/pull/185) | 升级 AgentOrch Format 1.6，保留套件基线和迁移历史。 |
| [Issue #186](https://github.com/ctrlshiftaltdel/TalkDND/issues/186) / [PR #187](https://github.com/ctrlshiftaltdel/TalkDND/pull/187) | 套件链接扫描排除本地资料、存档和缓存覆盖场景，避免把私有树带入验收。 |
| [Issue #189](https://github.com/ctrlshiftaltdel/TalkDND/issues/189) / [PR #203](https://github.com/ctrlshiftaltdel/TalkDND/pull/203) | 从本地索引构建获准 FULL pack，校验来源及清单，支持独立归档。托管 CI 使用合成数据。 |
| [Issue #190](https://github.com/ctrlshiftaltdel/TalkDND/issues/190) / [PR #207](https://github.com/ctrlshiftaltdel/TalkDND/pull/207) | 按本地 PDF 内容识别同版书型并解锁，文件名和 PDF hash 不作为唯一门槛。 |
| [Issue #191](https://github.com/ctrlshiftaltdel/TalkDND/issues/191) / [PR #192](https://github.com/ctrlshiftaltdel/TalkDND/pull/192) | ContentPack 增加物品、职业、种族、背景、专长、规则和表格实体。 |
| [Issue #194](https://github.com/ctrlshiftaltdel/TalkDND/issues/194) / [PR #195](https://github.com/ctrlshiftaltdel/TalkDND/pull/195) | 补 2024 SRD 规则叙述内容与导入入口。核实两版 SRD 无独立职业引言，收回原计划中的该项补写。 |
| [Issue #196](https://github.com/ctrlshiftaltdel/TalkDND/issues/196) / [PR #209](https://github.com/ctrlshiftaltdel/TalkDND/pull/209) | 发布四个玩家可见规则集，按 edition 隔离 SRD/FULL，并兼容迁移旧 2014/2024 值。 |
| [Issue #198](https://github.com/ctrlshiftaltdel/TalkDND/issues/198) / [PR #199](https://github.com/ctrlshiftaltdel/TalkDND/pull/199) | 发布 v1.1.0 内容平台和 SRD 数据基础。 |
| [Issue #200](https://github.com/ctrlshiftaltdel/TalkDND/issues/200) / [PR #201](https://github.com/ctrlshiftaltdel/TalkDND/pull/201) | 更新 AgentOrch 1.6.3，记录本地冲突处理、官方基线和差异。 |
| [Issue #216](https://github.com/ctrlshiftaltdel/TalkDND/issues/216) / [PR #217](https://github.com/ctrlshiftaltdel/TalkDND/pull/217) | 发布 v1.2.0，随游玩包交付四规则集与 FULL 处理能力。FULL 实体资产单列。 |
| [Issue #218](https://github.com/ctrlshiftaltdel/TalkDND/issues/218) / [PR #219](https://github.com/ctrlshiftaltdel/TalkDND/pull/219) | 把自动化级别存入 schema 9，支持遭遇、场景和存档覆盖，审计设置并保留玩家决策。 |
| [Issue #220](https://github.com/ctrlshiftaltdel/TalkDND/issues/220) / [PR #221](https://github.com/ctrlshiftaltdel/TalkDND/pull/221) | 发布 v1.3.0 自动化及对应存档迁移。 |
| [Issue #222](https://github.com/ctrlshiftaltdel/TalkDND/issues/222) / [PR #223](https://github.com/ctrlshiftaltdel/TalkDND/pull/223) | 交付只读 knowledge 五入口、稳定 ID、来源、默认玩家过滤及显式 DM 可见级别。 |
| [Issue #224](https://github.com/ctrlshiftaltdel/TalkDND/issues/224) / [PR #225](https://github.com/ctrlshiftaltdel/TalkDND/pull/225) | 发布 v1.4.0，统一 DM 知识查询能力。 |
| [Issue #226](https://github.com/ctrlshiftaltdel/TalkDND/issues/226) / [PR #227](https://github.com/ctrlshiftaltdel/TalkDND/pull/227) | 交付两版核心动作：擒抱、推撞、徒手、双武器和数据驱动的额外攻击。 |
| [Issue #228](https://github.com/ctrlshiftaltdel/TalkDND/issues/228) / [PR #229](https://github.com/ctrlshiftaltdel/TalkDND/pull/229) | 交付血怒、力竭、魅惑、恐惧，统一条件、来源、阈值和清理。 |
| [Issue #230](https://github.com/ctrlshiftaltdel/TalkDND/issues/230) / [PR #231](https://github.com/ctrlshiftaltdel/TalkDND/pull/231) | 交付结构化 Ready，复用动作、反应、资源、专注和触发路径。 |
| [Issue #232](https://github.com/ctrlshiftaltdel/TalkDND/issues/232) / [PR #233](https://github.com/ctrlshiftaltdel/TalkDND/pull/233) | 交付统一感官/遮蔽查询、距离与原因，按规则处理隐形、躲藏和攻击优势。 |
| [Issue #234](https://github.com/ctrlshiftaltdel/TalkDND/issues/234) / [PR #235](https://github.com/ctrlshiftaltdel/TalkDND/pull/235) | 发布 v1.5.0 四组战斗规则，主存档和战斗状态 schema 保持兼容。 |

</details>

<details>
<summary>236–255｜v1.6 施法设计、实现与公开发布</summary>

| 记录 | 增加、变更或保留的设计内容 |
| --- | --- |
| [Issue #236](https://github.com/ctrlshiftaltdel/TalkDND/issues/236) / [PR #237](https://github.com/ctrlshiftaltdel/TalkDND/pull/237) | 制定单一施法执行器、法术元数据、列表、成分与仪式的分层设计，列出兼容和人工裁定边界。 |
| [Issue #238](https://github.com/ctrlshiftaltdel/TalkDND/issues/238) / [PR #240](https://github.com/ctrlshiftaltdel/TalkDND/pull/240) | 把 D-1 到 D-12 的所有者选择写入章程，固定 schema 10、来源、仪式、成分与非目标。 |
| [Issue #239](https://github.com/ctrlshiftaltdel/TalkDND/issues/239) / [PR #241](https://github.com/ctrlshiftaltdel/TalkDND/pull/241) | Ability 增加可选 spell 元数据，SRD 导入器和 content check 支持该块。老内容没有该块时按旧行为加载。 |
| [Issue #242](https://github.com/ctrlshiftaltdel/TalkDND/issues/242) / [PR #243](https://github.com/ctrlshiftaltdel/TalkDND/pull/243) | 交付角色卡结构化 spells、两版职业/授予规则与 9→10 备份迁移，保留旧数据。 |
| [Issue #244](https://github.com/ctrlshiftaltdel/TalkDND/issues/244) / [PR #245](https://github.com/ctrlshiftaltdel/TalkDND/pull/245) | 交付法术列表命令、战斗引用、遗留采纳与 dry-run 同步，接建卡和旧升级入口。 |
| [Issue #246](https://github.com/ctrlshiftaltdel/TalkDND/issues/246) / [PR #247](https://github.com/ctrlshiftaltdel/TalkDND/pull/247) | 将 CI macOS runner 从 macos-14 换为 macos-15，保留三平台矩阵。 |
| [Issue #248](https://github.com/ctrlshiftaltdel/TalkDND/issues/248) / [PR #249](https://github.com/ctrlshiftaltdel/TalkDND/pull/249) | 同一 cast/use 执行器结算法术位、主骰升环、戏法成长、材料声明、契约槽、奥术和专注。 |
| [Issue #250](https://github.com/ctrlshiftaltdel/TalkDND/issues/250) / [PR #251](https://github.com/ctrlshiftaltdel/TalkDND/pull/251) | 两版手写层补 17/18 个法术，2024 四个瞬时 AoE 结构化。补施法文档、章程和人工限制。 |
| [Issue #252](https://github.com/ctrlshiftaltdel/TalkDND/issues/252) / [PR #253](https://github.com/ctrlshiftaltdel/TalkDND/pull/253) | 记录 v1.6.0 交付、版本、迁移和发布说明。正式资产以 GitHub Release 为准。 |
| [Issue #254](https://github.com/ctrlshiftaltdel/TalkDND/issues/254) / [PR #255](https://github.com/ctrlshiftaltdel/TalkDND/pull/255) | 修正 v1.6.0 测试计数 554→556，纳入两个单测试二进制。功能和 schema 不变。 |

</details>

<details>
<summary>256–292｜成长设计、风险调查与协议准备</summary>

| 记录 | 增加、变更或保留的设计内容 |
| --- | --- |
| [Issue #256](https://github.com/ctrlshiftaltdel/TalkDND/issues/256) / [PR #257](https://github.com/ctrlshiftaltdel/TalkDND/pull/257) | 把 v1.7 首段定为单职业成长与专长分期，首阶段完成后暂停。保留手改与未知历史。 |
| [Issue #258](https://github.com/ctrlshiftaltdel/TalkDND/issues/258) / [PR #259](https://github.com/ctrlshiftaltdel/TalkDND/pull/259) | 盘点角色、战斗、进度和存档的真实入口与读写风险，为成长实现划范围。 |
| [Issue #260](https://github.com/ctrlshiftaltdel/TalkDND/issues/260) / [PR #261](https://github.com/ctrlshiftaltdel/TalkDND/pull/261) | 实现资格“满足 / 不满足 / 未知”校验内核。本次不接玩家提交入口。 |
| [Issue #262](https://github.com/ctrlshiftaltdel/TalkDND/issues/262) / [PR #263](https://github.com/ctrlshiftaltdel/TalkDND/pull/263) | 调查 checkpoint 锁的偶发 WouldBlock。初次现场不足，未定位原故障根因。 |
| [Issue #264](https://github.com/ctrlshiftaltdel/TalkDND/issues/264) | 持续跟踪失败时持锁者、描述符与测试/产品影响。#316 后根因仍未知。 |
| [Issue #265](https://github.com/ctrlshiftaltdel/TalkDND/issues/265) / [PR #266](https://github.com/ctrlshiftaltdel/TalkDND/pull/266) | 提出自动迁移、完整备份与可恢复发布选项，记录成本和失败边界。 |
| [Issue #267](https://github.com/ctrlshiftaltdel/TalkDND/issues/267) / [PR #268](https://github.com/ctrlshiftaltdel/TalkDND/pull/268) | 确认 B/D 存储与恢复方向、schema 11、自动迁移、活动遭遇及显式恢复边界。 |
| [Issue #269](https://github.com/ctrlshiftaltdel/TalkDND/issues/269) / [PR #270](https://github.com/ctrlshiftaltdel/TalkDND/pull/270) | 写明 sidecar 身份绑定契约，拒绝按显示名称或旧 ID 猜测绑定。 |
| [Issue #271](https://github.com/ctrlshiftaltdel/TalkDND/issues/271) / [PR #272](https://github.com/ctrlshiftaltdel/TalkDND/pull/272) | 记录所有者选择 Q。具体格式、随机源、令牌生命周期与重绑/克隆仍待确定。 |
| [Issue #273](https://github.com/ctrlshiftaltdel/TalkDND/issues/273) / [PR #274](https://github.com/ctrlshiftaltdel/TalkDND/pull/274) | 适配 Rust 1.100 的测试可执行文件布局，恢复 talkdnd-dev 集成验证。 |
| [Issue #275](https://github.com/ctrlshiftaltdel/TalkDND/issues/275) / [PR #276](https://github.com/ctrlshiftaltdel/TalkDND/pull/276) | 实现只读身份发现和诊断，识别异常并提供定位，不改变归属或存档。 |
| [Issue #277](https://github.com/ctrlshiftaltdel/TalkDND/issues/277) / [PR #278](https://github.com/ctrlshiftaltdel/TalkDND/pull/278) | 设计协作 SAVE 锁域、锁顺序、文件发布和故障恢复协议，列明不能保证的边界。 |
| [Issue #279](https://github.com/ctrlshiftaltdel/TalkDND/issues/279) / [PR #280](https://github.com/ctrlshiftaltdel/TalkDND/pull/280) | 将 Oracle 从会更新完整性基线的审计路径移开，恢复其只读行为。 |
| [Issue #281](https://github.com/ctrlshiftaltdel/TalkDND/issues/281) / [PR #284](https://github.com/ctrlshiftaltdel/TalkDND/pull/284) | 将确认只读的 known、snapshot diff、adventure search/index 移出隐式完整性写入。 |
| [Issue #282](https://github.com/ctrlshiftaltdel/TalkDND/issues/282) / [PR #283](https://github.com/ctrlshiftaltdel/TalkDND/pull/283) | 落地 v1 协作锁原语与所有者平台、接入、备份和有界获取决定，当时尚无写者接入。 |
| [Issue #285](https://github.com/ctrlshiftaltdel/TalkDND/issues/285) | 跟踪只读入口改写完整性基线的副作用。草稿分支按命令的访问类型处理普通存档，并用旧基线逐条验证；秘密词检查和列表保留派生缓存。 |
| [Issue #286](https://github.com/ctrlshiftaltdel/TalkDND/issues/286) / [PR #287](https://github.com/ctrlshiftaltdel/TalkDND/pull/287) | 修正 checkpoint 旧锁归属、抢占和控制文件备份边界，保留已有完整性锁语义。 |
| [Issue #288](https://github.com/ctrlshiftaltdel/TalkDND/issues/288) / [PR #289](https://github.com/ctrlshiftaltdel/TalkDND/pull/289) | 建立首阶段实施顺序、依赖、迁移、验证和当前未定项，承接首阶段后暂停。 |
| [Issue #290](https://github.com/ctrlshiftaltdel/TalkDND/issues/290) / [PR #291](https://github.com/ctrlshiftaltdel/TalkDND/pull/291) | 盘点跨 CLI 写者、实际写入目标和过渡拦截，将写者接入拆成按类验收。 |
| [Issue #292](https://github.com/ctrlshiftaltdel/TalkDND/issues/292) | 确认显式 init、拒写 UX、缓存控制元数据、共享 crate、日志并发与保留规则。 |

</details>

<details>
<summary>293–316｜成长事务库、显式恢复和取证</summary>

| 记录 | 增加、变更或保留的设计内容 |
| --- | --- |
| [Issue #293](https://github.com/ctrlshiftaltdel/TalkDND/issues/293) / [PR #294](https://github.com/ctrlshiftaltdel/TalkDND/pull/294) | 建立两套 CLI 的过渡拒绝门、共享内部 crate 与写者注册。核查真实日志、内容输出和多级链接目标。 |
| [Issue #295](https://github.com/ctrlshiftaltdel/TalkDND/issues/295) / [PR #296](https://github.com/ctrlshiftaltdel/TalkDND/pull/296) | 将角色/进度/账本写者接到 SAVE，保留角色法术视图等只读路径。 |
| [Issue #297](https://github.com/ctrlshiftaltdel/TalkDND/issues/297) / [PR #298](https://github.com/ctrlshiftaltdel/TalkDND/pull/298) | 将 party/combat 写者接到共享 SAVE，保持依赖单向和未初始化旧档行为。 |
| [Issue #299](https://github.com/ctrlshiftaltdel/TalkDND/issues/299) / [PR #300](https://github.com/ctrlshiftaltdel/TalkDND/pull/300) | 将 checkpoint/migrate 写者接到 SAVE，维护现有回滚和备份边界。 |
| [Issue #301](https://github.com/ctrlshiftaltdel/TalkDND/issues/301) / [PR #302](https://github.com/ctrlshiftaltdel/TalkDND/pull/302) | 将日志与完整性写者接到 SAVE，日志只读查询继续可用。 |
| [Issue #303](https://github.com/ctrlshiftaltdel/TalkDND/issues/303) / [PR #304](https://github.com/ctrlshiftaltdel/TalkDND/pull/304) | 守住 save-meta 写入边界，单列原子控制缓存及根目录控制文件。本次未加入成长 sidecar。 |
| [Issue #305](https://github.com/ctrlshiftaltdel/TalkDND/issues/305) / [PR #306](https://github.com/ctrlshiftaltdel/TalkDND/pull/306) | 库级 D 协议执行准入、完整备份、构建并验证候选，结束于未发布状态。 |
| [Issue #307](https://github.com/ctrlshiftaltdel/TalkDND/issues/307) / [PR #308](https://github.com/ctrlshiftaltdel/TalkDND/pull/308) | 新成长路径有界获取内层完整性锁，默认 5 秒。原阻塞锁语义维持。 |
| [Issue #309](https://github.com/ctrlshiftaltdel/TalkDND/issues/309) / [PR #310](https://github.com/ctrlshiftaltdel/TalkDND/pull/310) | 库级逐文件发布、日志/完整性和幂等接线，使用 intent、比对和 done。仍无玩家成长提交 CLI。 |
| [Issue #311](https://github.com/ctrlshiftaltdel/TalkDND/issues/311) / [PR #312](https://github.com/ctrlshiftaltdel/TalkDND/pull/312) | 合入显式恢复库：先保护当前树和工作证据，人工选择前像或目标，重试识别进度，冲突停止。真实掉电等证据仍缺。 |
| [Issue #313](https://github.com/ctrlshiftaltdel/TalkDND/issues/313) / [PR #314](https://github.com/ctrlshiftaltdel/TalkDND/pull/314) | 发布文件系统调查，Linux ext4 实测部分支持迁移前提。Windows/macOS、网络盘和部分耐久性仍未知。 |
| [Issue #315](https://github.com/ctrlshiftaltdel/TalkDND/issues/315) / [PR #316](https://github.com/ctrlshiftaltdel/TalkDND/pull/316) | 发布 #264 现场，Linux 默认并行 checkpoint 套件 41 次中复现 2 次。持锁者和产品影响仍未知。 |

</details>
