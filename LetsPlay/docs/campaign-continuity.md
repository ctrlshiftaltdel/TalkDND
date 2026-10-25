# 战役连续性数据契约（P00）

> 本文件是「战役连续性」程序 P00–P08 的唯一数据契约：权威矩阵、文件格式、会话与时间模型、schema 与 integrity 策略、命令面、可见性规则、非目标与成功指标。
> 状态：2026-10-24 经项目所有者批准，由 TASK-0056（Issue #113）冻结；2026-10-24 按复审意见修订（检查点覆盖范围纳入 identified.yaml、restore 的 schema 不匹配与恢复后日志/基线规则、检查点 DM 本地定位），对应 docs/PROJECT-CHARTER.md Decision Log 同日条目；2026-10-24 按 TASK-0066（Issue #128）修订：`combat/*.state.json` 界定为权威数据并在存在时纳入 integrity，见第 1.1 节与第 3.3 节，对应 docs/PROJECT-CHARTER.md Decision Log 同日条目。后续任务（P00a–P08）按本契约实现；偏离本契约必须先改本文件并登记为项目所有者决策。
> 读者：P00–P08 各任务的实现者与审查者；DM 按需查阅（LetsPlay/docs/ 下的其他章节照旧）。
> 路径约定：除特别说明外，所有存档路径相对于游戏文件夹；Saves/<slot>/ 表示一份存档。英文标识符（字段名、命令名、文件名、枚举值）按本文逐字使用，不翻译、不改写、不改大小写。

## 1. 权威矩阵

每类数据有且只有一个权威写入者。其他程序只读；DM 手改会被 talkdnd check 报告（纳入 integrity 的文件），或被可见性规则拦截。

### 1.1 现有存储（全覆盖）

| 数据 | 文件 | 唯一写入者（命令） | 读取者 | 可见级别 | integrity | 迁移 / 回退 |
|---|---|---|---|---|---|---|
| 角色数据块 | Saves/<slot>/sheets/<id>.yaml | talkdnd char（另 chargen sheet、level-up、companion sheet、迁移时由 migrate 改写） | check/char/start/known、combat-sim | 玩家可见 | 纳入（sheets/*.yaml） | 旧卡首次使用时自动迁移；回退 = P01 检查点或 Saves/_backups/ 备份 |
| 渲染角色卡 / 剧情 NPC 卡 | Saves/<slot>/Characters/*.md、Saves/<slot>/NPCs/*.md | talkdnd char（渲染）、companion sheet、npc；不要手改 | start/known、DM | 玩家可见（NPCs/ 只放剧情 NPC） | 纳入（跳过 _ 开头文件，如 _party.md） | 随主 schema 迁移（5→6 移动过队友卡）；回退同上 |
| 队伍战斗状态 | Saves/<slot>/combat/party-state.yaml | combat-sim 战斗命令；talkdnd char 回写 HP/资源/状态 | start（当前状态节）、check | 玩家可见 | 纳入（combat/*.yaml） | 自带 version，高版本拒绝并提示更新；回退 = 检查点 |
| 战役清单与战斗状态 | Saves/<slot>/combat/campaign.yaml、combat/*.state.json | talkdnd new（清单）；combat-sim（*.state.json） | combat-sim、start | 玩家可见 | 均纳入（*.state.json 存在时纳入；combat-sim 每次成功写入后自动刷新基线，2026-10-24 TASK-0066 起） | 随主 schema；*.state.json 可用 combat-sim 从清单重建，但战斗进行中它是权威记录 |
| 经验 / 金币 / 物品 / 场景 | Saves/<slot>/progress.yaml | xp add、level-up、loot、scene | known/start/check | 玩家可见 | 纳入 | 自带 version（0/1 兼容），高版本拒绝；回退 = 检查点 |
| 已揭示知识 | Saves/<slot>/reveals.yaml | talkdnd reveal add（identify/handout 等自动登记也写入这里） | known、guard | 玩家可见（玩家已知事实的唯一台账） | 纳入 | 自带 version，高版本拒绝；回退 = 检查点 |
| 原创内容台账 | Saves/<slot>/original.yaml | talkdnd original add | check、start（DM 节）、known（已遇见的原创） | DM-only（hidden 面在揭示前不得外泄） | 纳入 | 自带 version，高版本拒绝；回退 = 检查点 |
| 存档元数据 | Saves/<slot>/save-meta.yaml | talkdnd new、migrate、settings | 所有命令（schema 检查） | 程序维护，DM 不手改 | 纳入 | schema 迁移见第 3.1 节；回退 = Saves/_backups/ 整档备份 |
| 即兴公开定论 | Saves/<slot>/canon.md | DM / 主持 agent 手写（无命令） | DM / 主持 agent | 玩家可见（公开设定） | 不纳入（当前行为，本契约不改变） | 纯文本无 schema；回退 = 整档备份 |
| DM 秘密 | Saves/<slot>/dm-secrets.md | DM 手写（无命令） | talkdnd secret（收集秘密词）、guard | DM-only，永不进入玩家视图、Issue、PR、CI | 不纳入 | 纯文本；永不入库、永不提交 |
| 已识别怪物 / 物品 | Saves/<slot>/identified.yaml | talkdnd identify | known、combat-sim --player、start | 双级别：真名玩家可见（识别后）；appearance 覆盖仅 DM | 不纳入（当前行为，P00b 不改变） | 自带 version，高版本拒绝；回退 = 检查点 |
| 会话日志 | Saves/<slot>/Sessions/*.md | 所有记录命令：checkpoint、log、char、xp、reveal、handout、identify、settings、level-up、loot、scene 等（每步一行或一节） | start（最后存档点）、check 对账、DM | 混合：日志供 DM 与接档 agent 读；玩家可看内容由 known/guard 把关 | 不纳入（append-only；对账基线记录日志位置而非内容哈希） | 纯文本无 schema；回退 = 整档备份 |
| 完整性清单 | Saves/<slot>/integrity.yaml | 工具每次写入后自动更新（after_write） | check、integrity status | 程序维护，不要手改 | 自身不纳入（不自哈希） | 旧存档首次工具写入时建立基线；删除后下次写入重建 |

### 1.2 其他现有存储（附录，同样受矩阵约束）

| 数据 | 文件 | 唯一写入者 | 可见级别 | integrity |
|---|---|---|---|---|
| 掷骰记录 | Saves/<slot>/rolls.log | talkdnd 掷骰命令（每一次程序掷骰） | 玩家可见 | 不纳入 |
| 接档摘要 | Saves/<slot>/digest.md | talkdnd start（每次覆盖） | DM / 接档 agent | 不纳入 |
| 建卡状态 | Saves/<slot>/chargen.json | talkdnd chargen | DM / 玩家流程 | 不纳入 |
| 房规 | Saves/<slot>/house-rules.md | DM 手写（可选文件） | 玩家可见 | 纳入（存在时） |
| 手动秘密词 | Saves/<slot>/secret-terms.yaml | talkdnd secret add/remove | DM-only | 不纳入 |
| DM 长期计划 | Saves/<slot>/dm-plan.md | talkdnd original plan 建模板，DM 手写 | DM-only | 不纳入 |
| 同伴草稿 | Saves/<slot>/npc-drafts/ | talkdnd companion random | DM / 玩家确认流程 | 不纳入 |
| 资料图登记与导出 | Adventures/<slug>/index/handouts.yaml、Saves/<slot>/handouts/ | talkdnd handout 命令族 | 登记表情景可见性逐条标；导出只限 player-safe | 不纳入 |

### 1.3 新增存储（P01–P08 引入，格式见第 4 节）

| 数据 | 文件 | 唯一写入者 | 读取者 | 可见级别 | integrity |
|---|---|---|---|---|---|
| 检查点清单与数据副本 | Saves/<slot>/checkpoints/<id>/ | talkdnd checkpoint snapshot（恢复由 checkpoint restore 替换存档） | checkpoint list/show、snapshot diff | DM-only（DM 本地恢复数据，非玩家视图）；数据副本同原文件 | 仅 checkpoints/*/manifest.yaml 存在时纳入 |
| 固定记忆 | Saves/<slot>/memory.yaml | talkdnd memory add/remove | memory list、start（关键记忆节） | 玩家视图只显示 player-confirmed | 存在时纳入（P00b） |
| 实体登记 | Saves/<slot>/entities.yaml | talkdnd entity 命令族 | entity/relation context/known（按过滤规则） | dm_only 条目仅 DM | 存在时纳入（P00b） |
| 关系登记 | Saves/<slot>/relations.yaml | talkdnd relation 命令族 | relation/relation context | dm_only 关系仅 DM | 存在时纳入（P00b） |
| 世界时间 | Saves/<slot>/world-time.yaml | talkdnd time 命令族 | time show、start（启用时） | dm_only 事件仅 DM；其余玩家可见 | 存在时纳入（P00b） |
| 冒险结构索引 | Adventures/<slug>/index/structured.json、Rules/<版本>/<slug>/index/structured.json | talkdnd adventure index build（派生数据） | adventure index show/search | 标题/页码/锚点，无正文 | 不纳入（书籍目录下的派生产物，gitignored） |

## 2. 会话与时间模型

### 2.1 会话（session）

- 一个会话 = Sessions/ 下的一个日志文件，命名 YYYY-MM-DD-session-NN.md（talkdnd new 建 Sessions/<date>-session-00-setup.md）。
- 当前会话 = 按文件名排序最新的日志（README.md 除外）。新开一场会话 = 新建一个日志文件；文件名即会话标识。
- 会话标识的规范形式是日志文件名（如 2026-10-24-session-01.md）。文件名以日期开头，字典序即时间序；P04/P06 的时点比较直接用字典序，不另建会话编号。

### 2.2 存档点（checkpoint section）

- 日志内以 ## 存档点（<现实日期 时间>）小节记录交接状态（talkdnd checkpoint <文本> 或 stdin 追加）。它是权威的交接快照文字：游戏内时间、地点、每个角色 HP/资源/金币/重要物品、未决事项、下一步选项。
- start 的「最后存档点」节 = 当前日志里最后一个 ## 存档点 小节（save::last_checkpoint 的现有行为，不改变）。
- 文字存档点与 P01 的结构化检查点是两层：前者写给人和接档 agent，后者给程序恢复用；二者互不依赖。

### 2.3 场景（scene）

- 当前场景 = progress.yaml 的 scene 标签，由 talkdnd scene <名称> 记录并打印 known。它是权威的场景标签；entities/relations 的 first_seen、since_session 引用会话标识，不引用场景。

### 2.4 游戏内时间（world time）

- 权威游戏内时钟只有一处：world-time.yaml，且仅在 enabled: true 时存在（默认 false，见第 4.5 节）。
- 未启用时，存档没有任何游戏内日历；日志和存档点里的游戏内时间文字只是自由文本，不是权威值，程序不解析、不比较。
- 启用后：date/phase 是自由文本标签（DM 用 time set 维护），elapsed_days 是唯一天数计数（time advance 累加）。没有任何命令从叙述或场景切换推断时间。

### 2.5 权威与派生

| 权威 | 派生 |
|---|---|
| 会话日志（发生了什么）、## 存档点 小节（最后记录的交接状态）、progress.yaml 的 scene、启用时的 world-time.yaml | digest.md（start 生成）、checkpoints/ 数据副本（快照时拷贝）、index/structured.json（pdf2md 输出生成）、start 摘要中的「较上次的变化」（snapshot diff 计算） |

派生数据可以在任何时刻删除并重建；权威数据只能由其唯一写入者变更。

## 3. Schema 策略

### 3.1 主存档 schema（save-meta.yaml 的 schema）

- 当前主 schema 为 7。整个 P00–P08 程序中，只有 P03 把它升到 8（新增 settings.style），并且必须：整档备份到 Saves/_backups/、写会话日志行、幂等、失败回退——与既有迁移（4→5、5→6、6→7）同一套规则。
- P01/P04/P06/P07/P08 不升主 schema。新增状态一律放进第 3.2 节的 sidecar。
- 高版本存档（schema 比程序支持的新）一律拒绝打开并提示更新 talkdnd（现有行为，不改变）。

### 3.2 版本化 sidecar

- 每个新文件自带 version 字段，从 1 开始：memory.yaml、entities.yaml、relations.yaml、world-time.yaml 为 version: 1；checkpoints/<id>/manifest.yaml 为 version: 1；index/structured.json 为 "version": 1。
- 缺失 = 功能未启用 / 内容为空：读取时按默认值处理（见第 4 节），首次写入时创建。
- 高版本 = 拒绝：报错点名文件和版本，提示更新 talkdnd（与 reveals.yaml/identified.yaml/original.yaml/party-state.yaml 的现有拒绝行为一致）。
- sidecar 的字段演进遵循与主 schema 相同的纪律：加字段必须给默认值、可自动迁移、有测试；删字段或改语义按新任务处理，不在实现任务里顺手改。
- 写入一律走原子写（临时文件 + 重命名，复用 fsutil::atomic_write），不留下半写文件。

### 3.3 integrity 纳入规则（P00b 执行）

- 存在才纳入：memory.yaml、entities.yaml、relations.yaml、world-time.yaml、checkpoints/*/manifest.yaml。文件不存在时清单里没有该项，check 不报缺失。
- 永不纳入：Sessions/（任何日志）、checkpoints/ 下的数据副本（只哈希 manifest.yaml，避免整份重复哈希）、integrity.yaml 自身。第 1.1/1.2 节里标注「不纳入」的存储维持现状，P00b 不扩大、也不收窄。2026-10-24 更新（TASK-0066）：唯一例外是 `combat/*.state.json`，从「不纳入」改为存在时纳入（见第 1.1 节）；本节其余「不纳入」项不变。
- 纳入后行为不变：工具写入自动更新指纹；手改被 changes_against 报告为 Modified，DM 用 talkdnd integrity accept --reason 接受。

### 3.4 check 扩展点

talkdnd check（不带参数）保持「只读审计」定位，不成为第二事实源。扩展只有两类，且都落在 P00b 之后的实现任务里：

1. 手改发现（P00b 自动获得）：第 3.3 节纳入的 sidecar 一旦被手改，走现有 Modified/Added/Removed 报告路径。
2. 跨引用一致性警告（P01/P04/P06 各加各的，全部是警告不是错误）：
   - relations.yaml 的 from/to 指向不存在的实体 id；
   - entities.yaml 出现重名或别名冲突（add 时已拒绝，check 兜底全表扫描）；
   - world-time.yaml 有 pending 事件的 due 标签等于当前 date 标签（到期提示）；
   - checkpoints/ 里有清单缺失或损坏的检查点（列出不可用 id）。

## 4. 新文件格式契约

以下六个格式在 P00 冻结。实现任务不得增删字段、改默认值或改拒绝语义；确需调整时先改本文件并登记项目所有者决策。

### 4.1 checkpoints/<id>/（P01）

- 位置：Saves/<slot>/checkpoints/<id>/。<id> 格式：YYYYMMDD-HHMMSS（本机时间），可跟 -<reason-slug>（reason 的 ASCII 小写字母/数字/连字符摘要，最长 24 字符）；重名追加 -2、-3。字典序即时间序。
- manifest.yaml（version: 1）最小字段：

  | 字段 | 类型 | 说明 |
  |---|---|---|
  | version | int | 固定 1 |
  | id | string | 同目录名 |
  | created | string | YYYY-MM-DD HH:MM（本机时间） |
  | reason | string | 可空；snapshot --reason 传入 |
  | schema | int | 快照时的主存档 schema |
  | log_file | string | 快照时的当前会话日志文件名 |
  | log_lines | int | 该日志在快照时的行数 |
  | files | map | 相对路径 → sha256:<hex>；覆盖下列数据副本的每个文件 |

- 数据副本覆盖范围（冻结，来自任务包）：progress.yaml、reveals.yaml、save-meta.yaml、original.yaml、identified.yaml、canon.md、house-rules.md（存在时）、Characters/、NPCs/、sheets/、combat/。不含 Sessions/、checkpoints/、dm-secrets.md、dm-plan.md、secret-terms.yaml、rolls.log、digest.md、chargen.json、handouts/、npc-drafts/、integrity.yaml。
- 回退语义（冻结）：检查点回退机械状态与已揭示/已识别知识——覆盖范围内的文件恢复到快照时状态；identified.yaml 文件小且必须与 reveals.yaml 保持一致，故纳入覆盖范围（其 integrity 按第 1.1 节维持不纳入，P00b 不改变）。检查点不回退 append-only 日志（Sessions/、rolls.log）、DM 工作文件（dm-secrets.md、dm-plan.md、secret-terms.yaml、digest.md、chargen.json、npc-drafts/）、派生文件（digest.md、checkpoints/ 自身）或二进制资料图导出（handouts/）。
- 写入命令：talkdnd checkpoint snapshot [--reason R] [--json]；读取命令：checkpoint list、checkpoint show <id>、snapshot diff。
- 可见性：检查点是 DM 本地恢复数据，不是玩家视图——清单与数据副本只供 DM 与恢复流程使用；对玩家只暴露第 6 节过滤后的 snapshot diff 输出。
- integrity：仅 checkpoints/*/manifest.yaml 存在时纳入。
- 失败/损坏行为：manifest 缺失、YAML 解析失败、version > 1、files 指纹与实际副本不符 → 该检查点标记为不可用（list/show 明确标注，restore 拒绝）；不影响其他检查点。
- 保留策略：每次成功 snapshot 后只保留最新 10 份，更早的在输出里报告后删除。
- 并发：同一 slot 同时只允许一个 snapshot/restore，用 Saves/<slot>/.talkdnd.lock 文件锁；第二个并发调用以退出码 1 失败并说明锁被占用。
- 迁移/回退：旧存档没有 checkpoints/ 时 snapshot 直接可用（无迁移）；restore 前先把整份当前存档备份到 Saves/_backups/<slot>-<stamp>-restore-<id>，再原子替换检查点覆盖范围内的文件，并删除快照之后新增的、属于覆盖范围的文件；范围外文件原样保留。提交中断（部分替换后失败）进入回滚时，把恢复前整份备份镜像回存档：复制备份内容，并删除备份中不存在的残留文件与目录（`.talkdnd.lock`、`.talkdnd.integrity.lock` 与 `.cache` 等被排除的运行时数据除外）；只有确认整个存档树回到操作前状态，才报告「已回滚到恢复前状态」，否则如实报告回滚失败、当前存档可能不完整并给出备份位置。schema 不匹配（冻结）：manifest.schema 小于当前主 schema 时，先完成恢复，再按第 3.1 节正常迁移（整档备份 + 会话日志行，与既有迁移同一套规则）；manifest.schema 大于当前主 schema 时拒绝恢复，报错点名检查点与 schema 并提示更新 talkdnd。恢复是受控工具操作：完成后按工具写入语义更新 integrity 清单与对账基线，使恢复后 talkdnd check 无未解释变化，并在当前会话日志追加一行恢复记录（检查点 id、备份路径、时间）；日志只追加、不截断，既有历史一律不改写。没有 --yes 拒绝执行（退出码 1）。整档备份必须完整：存档里若存在符号链接、套接字等无法作为普通文件或目录复制的条目，restore 在改动任何文件之前拒绝执行并说明原因（退出码 1），不会带着不完整的备份继续。

### 4.2 memory.yaml（P03）

- 位置：Saves/<slot>/memory.yaml；version: 1。
- 结构：entries 列表，每条 {id, text, source, added_at, tags}：

  | 字段 | 类型 | 默认 | 说明 |
  |---|---|---|---|
  | id | string | 自动 | M-001 起顺序编号，不重用 |
  | text | string | 必填 | 非空；空文本拒绝 |
  | source | string | manual | player-confirmed \| import \| manual |
  | added_at | string | 自动 | YYYY-MM-DD HH:MM |
  | tags | list | [] | 字符串标签 |

- 写入命令：talkdnd memory add <文本> [--source S] [--tag T]...、memory remove <id>；读取命令：memory list、start 的「关键记忆」节。
- integrity：存在时纳入。
- 失败/损坏行为：文件缺失 = 空台账；YAML 非法 = 报错点名文件；version > 1 = 拒绝并提示更新；重复 id 不可能（顺序分配）；删除不存在的 id = 退出码 1。
- 迁移/回退：无独立迁移（P03 升主 schema 到 8 时本文件随新功能创建）；回退 = 删除文件回到无记忆状态（先备份，DM 录入内容会丢）。
- 与 canon.md/reveals.yaml 的边界：记忆是「长期有效的固定事实与偏好」，不记录事件进程（日志）、不记录玩家已知揭示（reveals）、不记录公开定论（canon）。边界以本契约为准；若实现中发现重叠，优先扩展既有文件，不新增第四套事实源。

### 4.3 entities.yaml（P04）

- 位置：Saves/<slot>/entities.yaml；version: 1。
- 结构：entities 列表，每条 {id, type, name, aliases, first_seen, status, dm_only}：

  | 字段 | 类型 | 默认 | 说明 |
  |---|---|---|---|
  | id | string | 自动 | E-001 起顺序编号，不重用、不随改名变化 |
  | type | string | 必填 | npc \| location \| faction \| quest \| item |
  | name | string | 必填 | 主名称；与既有 name/aliases 大小写不敏感比较，冲突拒绝 |
  | aliases | list | [] | 别名；同样参与冲突比较 |
  | first_seen | string | 当前会话日志文件名 | 会话标识（可带 #锚点） |
  | status | string | active | active \| retired |
  | dm_only | bool | false | true 时仅 DM 视图可见 |

- 写入命令：entity add/list/show/rename/retire；读取命令：同左 + relation context + known（按可见性过滤）。
- integrity：存在时纳入。
- 失败/损坏行为：缺失 = 空登记表；YAML 非法或 version > 1 = 拒绝并提示；重名/别名冲突 = 退出码 1 并列出冲突实体，不自动合并；rename 把旧名追加进 aliases（保留旧名→新名历史）并写会话日志；retire 只改 status，记录保留。
- 迁移/回退：无独立迁移；回退 = 删除文件（先备份）。旧日志里的名字不受 rename 影响（日志不改写）。
- 候选规则：从日志或资料里发现疑似新实体只提示，不自动写入。

### 4.4 relations.yaml（P04）

- 位置：Saves/<slot>/relations.yaml；version: 1。
- 结构：relations 列表，每条 {id, type, from, to, since_session, until_session, source, dm_only, note}：

  | 字段 | 类型 | 默认 | 说明 |
  |---|---|---|---|
  | id | string | 自动 | R-001 起顺序编号 |
  | type | string | 必填 | member \| ally \| enemy \| located_at \| holds \| seeks \| knows |
  | from / to | string | 必填 | 实体 id（命令行接受 id 或唯一名称，落库一律 id） |
  | since_session | string | 当前会话日志文件名 | 生效会话标识 |
  | until_session | string | "" | 结束会话标识；"" = 仍生效 |
  | source | string | "" | 来源（日志锚点 / reveal 编号 / manual） |
  | dm_only | bool | false | true 时仅 DM 视图可见 |
  | note | string | "" | 可选说明 |

- 时点语义：关系在会话 S 生效，当且仅当 since_session ≤ S（文件名字典序）且（until_session 为空或 S < until_session）。已结束关系不出现在后续会话的查询与 relation context。
- 写入命令：relation add/list/end/show/context；end 写 until_session（缺省当前会话）并保留记录——更正错误关系用 end + note 说明，不删除行，保证可追溯。
- integrity：存在时纳入。
- 失败/损坏行为：缺失 = 空登记表；YAML 非法或 version > 1 = 拒绝；from/to 指向不存在实体 = 退出码 1（check 也会警告）；重复 add 同一 (type, from, to) 且未结束 = 幂等返回既有 id，不新建行。
- 迁移/回退：无独立迁移；回退 = 删除文件（先备份）。
- session 标识只用日志文件名（第 2.1 节），不依赖世界时间（P06）。

### 4.5 world-time.yaml（P06）

- 位置：Saves/<slot>/world-time.yaml；version: 1。
- 顶层字段：

  | 字段 | 类型 | 默认 | 说明 |
  |---|---|---|---|
  | enabled | bool | false | 功能开关；false 时所有写命令拒绝 |
  | calendar | string | "" | 自由文本历法标签 |
  | date | string | "" | 自由文本当前日期标签 |
  | phase | string | "" | 自由文本时段标签 |
  | elapsed_days | number | 0 | 唯一的天数计数，advance 累加 |
  | events | list | [] | 预定事件，每条 {id, due, title, status, dm_only, notes} |

  事件字段：id（EV-001 起）、due（自由文本标签）、title、status（pending \| done \| skipped）、dm_only（bool，默认 false）、notes（string，默认 ""）。

- 到期判定（P00 冻结的简化规则）：只按标签相等——due 与当前 date 标签相同且 status = pending 即「今天到期」，由 time event list 标注、start 在启用时提示。程序不做历法算术（月长、闰年、跨标签先后一律不推断）；elapsed_days 只回答「过了多少天」。
- 可见性（2026-10-25 项目所有者决定，澄清 dm_only 与「列出全部事件」的冲突）：time event list 默认只列非 dm_only 事件（玩家视图），--all 为 DM 视图（全部事件，dm_only 行首标 [DM]，不得进入玩家可看材料）；start 只提示玩家可见的到期事件。
- 写入命令：time enable/disable/set/advance/event add/done/skip；读取命令：time show、time event list、start（启用时的时间与到期事件节）。
- integrity：存在时纳入。
- 失败/损坏行为：文件缺失 = 未启用（默认）；YAML 非法或 version > 1 = 拒绝并提示；enabled: false 时 set/advance/event add/done/skip 一律退出码 1（show/event list/enable 可用）；advance 必须带 --days 或 --hours 至少一个；重复 advance 不累计错误（纯加法）。
- 迁移/回退：无独立迁移；disable 保留文件内容，删除文件等于回到未启用。事件可引用 P04 实体 id 写在 notes 里（纯文本引用，不做外键校验——外键校验只覆盖 relations，见第 3.4 节）。
- 推进纪律：只有显式命令改时间；叙述、场景切换、休息结算都不自动推进。

### 4.6 index/structured.json（P07）

- 位置：书文件夹下的 index/structured.json，即 Adventures/<slug>/index/structured.json 与 Rules/<版本>/<slug>/index/structured.json。派生数据，gitignored，可随时删除重建。
- version 固定 1；结构：

  | 字段 | 类型 | 说明 |
  |---|---|---|
  | version | int | 固定 1 |
  | book | string | 书文件夹 slug |
  | source | object | {path, sha256}：path 为相对书文件夹的 PDF 路径；sha256 为 PDF 的 sha256（取不到时退化为 mtime 秒，并在 source.mtime 记录） |
  | headings | list | {level, title, page, anchor}，按文档顺序 |
  | chapters | list | {title, page, anchor} |
  | maps | list | {title, page, anchor} |
  | figures | list | {title, page, anchor} |
  | statblocks | list | {title, page, anchor} |
  | low_quality_pages | list | 页码整数列表 |

- 不存正文：只存标题、页码、锚点与类型；check-leaks 覆盖 index/ 目录。
- 写入命令：talkdnd adventure index build [--book B]（缺省本存档的 adventure）；读取命令：adventure index show <book>、adventure index search <book> <查询>。
- integrity：不纳入（书籍目录下的派生产物）。
- 失败/损坏行为：缺 index.json 或 <书名>.pages.md = 退出码 1 并点名缺什么；JSON 损坏 = 明确报错并提示重建；源 PDF 的 sha256/mtime 与记录不符 = 重建；版本更高 = 拒绝并提示更新。
- 重建规则：幂等；顺序稳定（文档序）；源未变、版本未降时跳过（输出 skipped）。
- 迁移/回退：无迁移；删除即重建。不自动抽取实体/线索；DM 标签只接受显式输入。

## 5. 命令面（P01–P08）

### 5.0 总规则

- 命令名逐字使用下表的英文名；全局参数不变：--root、--save、隐藏的 --seed（仅供测试）。
- 退出码：0 成功；1 工具错误（拒绝、缺失、损坏、锁被占、参数组合非法）；2 命令行用法错误（clap）。
- 人读输出为中文行（与现有一致）；标注 [--json] 的命令额外支持单行 JSON 到 stdout，字段名英文。
- 存量命令行为、输出、退出码不变（P00a 验收第 4 条）。唯一的结构变化：checkpoint 从「纯文本参数」变为「命令组」，talkdnd checkpoint <文本>（或 stdin）原样可用；文本以子命令名（snapshot/list/show/restore）开头时用 -- 分隔或走 stdin。
- P00a 脚手架（TASK-0057）落地九个命令组：checkpoint、snapshot、memory、settings style、entity、relation、time、adventure index、oracle；模块 checkpoint.rs、diff.rs、memory.rs、entity.rs、graph.rs、worldtime.rs、bookindex.rs、oracle.rs 各带 Cmd 枚举与 pub fn run(root, save, cmd) -> Result<()>，未实现子命令统一 bail!("not implemented in TASK-0057")；lib.rs 只转交。帮助文本按本节写死。
- 2026-10-25 澄清（TASK-0064）：oracle 是唯一不读写任何存档文件的命令组；它的三个子命令同样从 lib 分发接收共享 RNG，因此 --seed 下输出可复现。

### 5.1 P01 — checkpoint / snapshot（TASK-0059，合并原 P01+P02）

| 命令 | 参数 | 输出 | 退出码 |
|---|---|---|---|
| checkpoint <文本> | 文本或 stdin | 追加 ## 存档点（时间）到当前日志（现有行为） | 0 / 1 |
| checkpoint snapshot | [--reason R] [--json] | 新检查点 id、路径、保留清理报告；--json 输出 manifest 内容 | 0 / 1（锁被占、旧存档无日志） |
| checkpoint list | [--json] | 每行：id、created、reason、log 锚点、是否可用 | 0 |
| checkpoint show <id> | [--json] | manifest 字段 + 文件数 + 总字节；损坏检查点明确标注 | 0 / 1（不存在/损坏） |
| checkpoint restore <id> | --yes（必需）[--json] | 备份路径、替换/删除文件清单；无 --yes 拒绝 | 0 / 1 |
| snapshot diff <idA> <idB\|current> | current 关键字 = 当前存档 | 结构化差异：HP、临时 HP、等级、资源、状态、金币、物品、XP、队伍成员；字段缺失/改名标注「未知」，不推测；同一输入输出确定 | 0 / 1 |

- start 摘要增加「最后检查点」和「较上次的变化」两节（P01 拥有 start.rs）。
- 并发：文件锁 Saves/<slot>/.talkdnd.lock；恢复前整档备份（第 4.1 节）。

### 5.2 P03 — memory / settings style（TASK-0061）

| 命令 | 参数 | 输出 | 退出码 |
|---|---|---|---|
| memory add <文本> | [--source player-confirmed\|import\|manual] [--tag T]... | 新 id；空文本拒绝 | 0 / 1 |
| memory list | [--tag T] [--all] | 默认只列 player-confirmed；--all 列全部并标来源（DM） | 0 |
| memory remove <id> | | 删除报告；id 不存在退出码 1 | 0 / 1 |
| settings style show | | 当前 narration/detail/pace/npc_voice/options | 0 |
| settings style set <字段> <值> | --reason R（必需，记日志与原因） | 旧值 → 新值 + 会话日志行 | 0 / 1（值不合法） |

- settings（无参数）与 settings dice on|off 行为不变；style 是 settings 下的新子命令组。
- 字段取值：narration（short/standard/long）、detail（low/standard/high）、pace（slow/standard/fast）、npc_voice（distinct/neutral）、options（2-4/none）。
- 只有玩家明确反馈才改，不自动推断；每次改动写会话日志（含 reason）。
- start 摘要在「玩家已知」后增加「关键记忆」节；玩家视图只显示 player-confirmed 条目。
- schema 7 → 8 只此一处（第 3.1 节），旧档备份、默认 style、失败回退。

### 5.3 P04 — entity / relation（TASK-0062，合并原 P04+P05）

| 命令 | 参数 | 输出 | 退出码 |
|---|---|---|---|
| entity add | --type npc\|location\|faction\|quest\|item --name N [--alias A]... [--first-seen S] [--dm-only] | 新 id；重名/别名冲突拒绝并列出冲突方 | 0 / 1 |
| entity list | [--type T] [--all] | 默认 active；--all 含 retired 并标 status | 0 |
| entity show <id\|名称> | | 该实体全部字段与关系数 | 0 / 1 |
| entity rename <id\|名称> <新名称> | | 旧名进 aliases，日志行 | 0 / 1 |
| entity retire <id\|名称> | | status=retired，记录保留 | 0 / 1 |
| relation add | --type member\|ally\|enemy\|located_at\|holds\|seeks\|knows --from F --to T [--since S] [--source X] [--note N] [--dm-only] | 新 id 或既有 id（幂等） | 0 / 1 |
| relation list | [--entity E] [--at S] [--all] | 默认当前会话生效中的关系；--all 含已结束 | 0 |
| relation end <id> | [--at S] [--note N] | 写 until_session，记录保留 | 0 / 1 |
| relation show <id> | | 该关系全部字段 | 0 / 1 |
| relation context <实体> | | 实体摘要 + 当前生效关系（玩家过滤） | 0 / 1 |

- 所有查询按第 6 节过滤 dm_only 与已结束关系。
- 不改 identify.rs（只读参考）；不动 lib.rs/migrate.rs/start.rs。

### 5.4 P06 — time（TASK-0063）

| 命令 | 参数 | 输出 | 退出码 |
|---|---|---|---|
| time show | | enabled/calendar/date/phase/elapsed_days/事件计数（事件计数为玩家可见数，不含 dm_only） | 0 |
| time enable / time disable | | 开关状态 + 日志行 | 0 |
| time set | --date D [--phase P] | 新标签；未启用拒绝 | 0 / 1 |
| time advance | --days N 或 --hours N（至少一个） | 新 elapsed_days；纯加法 | 0 / 1 |
| time event add | --due 标签 --title T [--dm-only] [--note N] | 新事件 id | 0 / 1 |
| time event list [--all] | | 默认玩家视图（只列非 dm_only）；--all 为 DM 视图（全部事件，dm_only 行首标 [DM]）；due 等于当前 date 且 pending 的标「今天到期」 | 0 |
| time event done <id> / time event skip <id> | | 状态变更 + 日志行 | 0 / 1 |

- 未启用时写命令一律退出码 1（show/event list/enable 可用）。
- start 仅在启用时显示时间与玩家可见的到期事件（dm_only 事件不出现）；事件不自动改数值或剧情。
- 事件 notes 可文本引用 P04 实体 id；不做外键校验。

### 5.5 P07 — adventure index（TASK-0060）

| 命令 | 参数 | 输出 | 退出码 |
|---|---|---|---|
| adventure index build | [--book B]（缺省本存档 adventure） | rebuilt/skipped + structured.json 路径；源未变跳过 | 0 / 1（缺 index） |
| adventure index show <book> | | 章/节/图示/数据卡的标题、页码、锚点；不打印正文 | 0 / 1 |
| adventure index search <book> <查询> | | 只匹配标题/章节/图示/数据卡标签；输出标题、页码、锚点 | 0 / 1 |

- 输出绝不包含正文；重建幂等、顺序稳定；缺 index.json/pages.md 明确报错。
- 不改 adventure section/search 的现有读取语义。

### 5.6 P08 — oracle（TASK-0064）

| 命令 | 参数 | 输出 | 退出码 |
|---|---|---|---|
| oracle yes-no | [<问题>] | yes 或 no（掷 1d6：1–3 yes，4–6 no），经 combat-sim 骰子引擎 | 0 |
| oracle event | [--focus F] | 一个事件焦点短语（模块内置原创清单，等概率；不引入书籍随机表） | 0 |
| oracle meaning | [--chaos 1-9]（默认 5） | 一个含义短语（内置原创清单）+ chaos 值 | 0 |

- 全部命令只读存档：不写 progress/canon/reveals 或任何权威状态；不伪造骰子；--seed（全局隐藏参数）保证测试可复现。
- 出版冒险模式下不得绕过 deviation/original 限制；dm-plan.md 模板增加可选小节（当前局面、参与者目标、冲突、线索、可能行动、不介入后果），旧文件可读、向后兼容。

## 6. 可见性规则

### 6.1 玩家视图

玩家视图 = 下列输出的并集：known；start 摘要中面向玩家的部分（关键记忆节只含 player-confirmed）；handout 导出内容；combat-sim --player；guard 检查对象（叙述草稿）；entity list/entity show/relation list/relation context/memory list/time show/time event list 的默认输出（time event list 默认即玩家视图；--all 的 DM 视图见第 6.2 节）；adventure index show/search；snapshot diff 默认输出（检查点清单与数据副本本身是 DM 本地恢复数据，不进入玩家视图，见第 4.1 节）。

过滤规则（实现时逐条生效）：

1. dm_only: true 的实体、关系、事件、记忆条目不出现。
2. 记忆还要求 source = player-confirmed 才进玩家视图（import/manual 仅 DM）。
3. 已结束关系（until_session 非空且不晚于当前会话）不出现；retired 实体不出现在默认列表与 relation context。
4. original.yaml 的 hidden 面、dm-secrets.md、dm-plan.md、secret-terms.yaml 内容永不进入玩家视图。
5. identified.yaml 未识别条目只显示 appearance，不显示真名。
6. handout 只导出标记 player-safe 的条目或其登记 crop；DM-only 资料图绝不导出。
7. 冒险章节的进度限制不因索引绕过（adventure section --reason 规则沿用）。

### 6.2 DM 视图

DM 视图 = 全部数据，并在输出里标注 DM-only。--all/--dm 开关只存在于第 5 节列出的命令上；拿到 DM 全量输出的 agent 有义务不让它进入玩家可看材料（guard 是闸门）。

### 6.3 dm_only 与 hidden

- dm_only（新字段）：读取路径过滤，见上。
- hidden（original.yaml 既有字段）：揭示前是秘密词来源；talkdnd reveal add --original O-xxx 后才进入玩家可知范围。本程序不新增第二类 hidden 语义。
- guard 的秘密词源维持现有五类（dm-secrets.md、dm-plan.md、未来章节标题、original --hidden、secret-terms.yaml），不因新 sidecar 扩大；dm_only 内容靠读取过滤保护。

## 7. 非目标

1. 不新建事实源：entities.yaml/relations.yaml/memory.yaml 是索引与偏好，不替代 canon.md/reveals.yaml/progress.yaml/会话日志的权威地位。
2. 不自动推进游戏内时间；叙述、场景切换、休息结算都不改 world-time.yaml。
3. 不自动合并同名/同别名实体；候选只提示不写入。
4. P07 不做正文存储、全文检索或实体/线索自动抽取；搜索只覆盖标题级标签。
5. Oracle 不写任何存档文件、不参与规则结算、不伪造骰子、不引入书籍随机表。
6. 不做历法算术（月长、闰年、跨标签先后）。
7. 不新增网络服务、账号体系、外部服务或新依赖；P09（第二屏展示与玩家设备输入）不在本程序范围，如要做须单独章程变更与安全评估。
8. P00 本身不改代码、不建数据文件、不写真实存档；存量命令行为不变。

## 8. 成功指标

每个任务对应一个可观测的游玩痛点；没有证据的功能不进入实施。

| 任务 | 痛点 | 可观测指标 |
|---|---|---|
| P00（本契约） | 后续任务互相踩 lib.rs/integrity.rs，数据归属靠口头约定 | 契约被 P00a–P08 逐条引用；全文没有未决项 |
| P00a 脚手架 | 每个功能都改 lib.rs 造成路径冲突 | 九个命令组 --help 可运行并列出的参数与本契约一致；未实现子命令非 0 退出且打印 not implemented |
| P00b integrity | 手改新文件没人发现 | 手改 memory/entities/relations/world-time/manifest 被 talkdnd check 报告为 Modified |
| P01 检查点 | 会话中断 / agent 退出后只能手工重建状态 | 恢复最后一个完整检查点后 check 无未解释变化；snapshot diff 说明较上次的变化；重复/并发触发只有一个成功 |
| P03 风格与记忆 | 接档摘要丢长线事实，每局偏好反复重说 | 摘要含关键记忆与风格；玩家视图不出现未确认记忆；旧档迁移有备份与回退 |
| P04 实体与关系 | 「那个 NPC 是谁」要重读日志 | relation context 给出当前关系；rename 不破坏旧引用；玩家视图无 dm_only |
| P06 世界时间 | 「游戏里过了几天」没有答案 | 默认关闭不影响现有战役；启用后到期事件在 start 提示；时间只随显式命令前进 |
| P07 结构索引 | 为找一个章节整本读进上下文 | 不整本读即可定位章节/图示/数据卡；搜索结果只有标题级信息 |
| P08 dm-plan 与 Oracle | 局势没有结构，即兴没有灵感来源 | dm-plan 可选小节可读可写；oracle 输出 seed 可复现且不碰任何存档文件 |

## 9. 波次与依赖图

项目所有者 2026-10-24 批准的范围：P00–P08；P02 并入 P01，P05 并入 P04，P09 移出。

| 波次 | 任务（Issue） | 并行 | 说明 |
|---|---|---|---|
| 0 | P00 数据契约（#113） | 串行 | 冻结权威矩阵、文件格式、命令面、schema/integrity 策略 |
| 1 | P00a CLI 脚手架（#114）+ P00b integrity（#115） | 2 路并行 | 文件不相交：lib.rs vs integrity.rs |
| 2 | P01 状态历史（#116）+ P07 结构索引（#117） | 2 路并行 | P01 独占 start.rs/layout.md/README；P07 独占 adventure.rs/pdf2md.md |
| 3 | P03 风格与记忆（#118）+ P04 实体与关系（#119） | 2 路并行 | P03 独占 start.rs/migrate.rs/layout.md/README；P04 独占新模块与新文档 |
| 4 | P06 世界时间（#120） | 串行 | start.rs 在 P03 合并后释放；依赖 P04 实体 id |
| 5 | P08 dm-plan + Oracle（#121） | 串行 | 依赖 P03 文档、P06 时间、P07 索引 |

- 并行度：每波最多 2 个执行实例（路径锁、三平台 CI 与审查带宽所限），不按机器余量盲目加。
- 发布切片：项目所有者 2026-10-25 决定，P00–P08 与实战质量加固合并为 **1.0.0** 一次性发布（第一个稳定版），替代原 v0.12/v0.13/v0.14 切片；数据契约本身不变。
- 门禁：每个任务按 Issue 五节验收 + G1–G6 + 当前 head 的 CI；P01/P03/P04/P06 的迁移与恢复必须有失败注入测试。P03 是唯一主 schema 变更（升到 8），必须独占其迁移窗口。
- P09 不进入路线图；如要做，单独章程变更与安全评估后再立任务。

## 10. 文档维护

- 本契约是 P00–P08 的裁决依据：实现任务发现契约与现实冲突时，停下来登记任务，不自行发明权限或悄悄偏离。
- 修改本契约 = 一次项目所有者决策：同一 PR 更新 docs/PROJECT-CHARTER.md 的 Decision Log 与受影响章节（活章程规则）。
- 各实现任务按自己 Issue 的可改路径更新配套文档（LetsPlay/docs/layout.md、pdf2md.md、narration.md，以及新增的 entities.md、world-time.md、oracle.md）；那些文档与本文冲突时以本文为准，并回改本文。
