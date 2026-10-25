# 更新日志

本文件记录 TalkDND 每个发布版本的变化，按版本从新到旧排列。版本号规则见 [docs/versioning.md](docs/versioning.md)。GitHub Release 的说明文字由本文件对应的版本小节生成（`cargo run --manifest-path tools/talkdnd-dev/Cargo.toml -- release-notes <版本>`）。

每个版本分为：**新功能**、**变更**、**修复**、**更新说明 / 迁移**、**已知限制**。括号里的 `#N` 是合并的 PR。

## 未发布

### 新功能

无。

### 变更

无。

### 修复
- **收口 restore / migrate 的内联完整性清单写并发缺口**（#154）。TASK-0075 给完整性清单的读-改-写加了专用锁之后，还剩两处内联写不经锁：`checkpoint restore` 末尾的清单重定基（读清单 → 拍快照 → 写对账基准 → 存回）和存档迁移收尾的 `carry_dirty_flags`（把手改同伴的旧指纹从 `NPCs/` 搬到 `Characters/`）。并发进程若在两处的读与存之间提交，其指纹会被旧清单覆盖——restore 会把检查点里的陈旧指纹写回去，被并发命令刚改过的文件从此一直被误报成「工具以外改动」。现在这两段都在同一把 `.talkdnd.integrity.lock` 内完成并在锁内重新读清单；锁顺序固定为先检查点锁 `.talkdnd.lock`、后完整性锁，全仓库无反向顺序，迁移内部的多次清单写逐次串行不嵌套，不会死锁。指纹算法、清单 schema、`check` 的文案与退出码、「脏文件永不洗白」语义均不变；新增确定性回归测试在修复前代码上可复现丢指纹。`LetsPlay/docs/table-integrity.md` 的已知限制相应更新为已收口。

### 更新说明 / 迁移

无存档 schema 变更、无迁移。

### 已知限制

- 两个进程真的同时写同一个受保护文件时，后提交的一方仍会被保守地报成「工具以外改动」；这是「脏文件永不洗白」的设计取舍，按日志确认后用 `integrity accept --reason` 处理。

## v1.0.2 — 2026-10-25

### 新功能

无（质量修复补丁）。

### 变更

无（本版只包含两处缺陷修复；命令行为与存档结构不变）。

### 修复
- **修复并发写入丢失完整性指纹导致的存档误报**（#151）。两个 `talkdnd` / `combat-sim` 进程交错更新同一存档的 `integrity.yaml` 时，后写者会用自己加锁前读到的旧清单覆盖先写者刚记录的指纹，而「脏文件永不洗白」让这条误报一直挂着。现在每次清单读-改-写（工具写入后的指纹记录、`check`/`start` 的对账基准写回、`integrity accept`、旧存档首次建基准）都在存档目录的专用锁文件 `.talkdnd.integrity.lock` 内完成，并在锁内重新读取清单；锁随进程退出由操作系统释放，没有陈旧锁，锁文件本身不纳入指纹。指纹算法、清单 schema、`check` 的退出码与文案都不变。已知限制：`checkpoint restore` 末尾与迁移流程里个别内联清单写仍不经锁（见 `LetsPlay/docs/table-integrity.md`）。
- **`checkpoint restore` 的回滚改为整档镜像**（#150）。提交阶段部分替换后失败进入回滚时，此前只把恢复前整份备份复制回存档：若快照时存在、恢复前已被删除的覆盖文件在提交阶段被重新创建，回滚不会清掉它（暂存重建的目录同理），却仍报告「已回滚到恢复前状态」。现在回滚会删除备份中不存在的残留文件与目录（锁文件与 `.cache` 等被特意排除的运行时数据除外），只有整个存档树确认一致才报告回滚成功；暂存中止路径同样清掉暂存创建的目录，残留删不掉时逐条如实报告。

### 更新说明 / 迁移
- **覆盖式更新**：下载 1.0.2 发布包，把包内 `TalkDND/LetsPlay/` 的内容覆盖到现有游戏文件夹，覆盖同名文件；存档目录原样保留。不要用旧包整个替换。
- 无存档 schema 变更、无迁移：schema 仍为 8，更新后直接可玩。
- pdf2md 仍为 0.6.0，不重转已转换的书；两个程序（combat-sim、talkdnd）随发布包更新。
- 1.0.2 包含 PR #150（TASK-0074）与 PR #151（TASK-0075）两处修复；两者在各自 head 的 CI 全绿，合并后在 main 重跑全部测试与检查（combat-sim 208、talkdnd 271、talkdnd-dev 45 个测试，`validate_kit.py`、`check-leaks`、`check-letsplay` 退出码 0）。

### 已知限制
- `checkpoint restore` 末尾与 `talkdnd migrate` 的内联清单写不经过 `.talkdnd.integrity.lock`；restore 自身由 `.talkdnd.lock` 串行化，但与其他进程的清单写仍可能交错（TASK-0075 范围外，留待后续任务）。
- 两个进程真的同时写同一个受保护文件时，后提交的一方会被保守地报成「工具以外改动」；这是「脏文件永不洗白」的设计取舍，按日志确认后用 `integrity accept --reason` 处理。
- 沿用 1.0.1 的已知限制：检查点整份备份目录名带秒级时间戳，同一秒内对同一检查点 restore 两次会撞名；检查点不覆盖 memory/entities/relations/world-time、会话日志与 DM 工作文件。

## v1.0.1 — 2026-10-25

### 新功能

无（质量修复补丁）。

### 变更
- **`talkdnd check` 新增契约 §3.4 跨引用审计**（#143）。无参数的 `check` 按冻结契约检查四类问题，各以一行 `cross-ref:` 前缀报出：`relations.yaml` 里指向不存在实体的悬空端点（按 id 精确比较，每个端点一行并带关系 id）、全台账内冲突的实体名/别名（大小写不敏感，含「名字等于自己的别名」）、世界时间中 `status: pending` 且 `due` 等于今天的事件（世界时间未启用时附带明确说明）、不可用的检查点（manifest 缺失或损坏、副本校验失败，与 `checkpoint list` 的 `problem` 口径一致）。全部是警告不是错误：`check` 仍退出 0，完整性语义不变；`start` 的 DM 警告节可能显示这些警告。只有跨引用警告、没有其他问题时，`check` 不再打印 `integrity: OK`（与既有 deviation 警告的 clean 语义一致）。
- **读不了的 sidecar 不再中断检查**（#143）。某个数据文件（YAML 损坏、schema 更新）读不了时，转为一条点名文件与错误原因的警告，其余检查继续。

### 修复
- **`checkpoint restore` 不再虚报回滚成功**（#143）。提交失败后的回滚会验证结果：只有整份存档回滚确实成功才打印「已从 … 回滚到恢复前状态」；否则明确报告回滚失败、当前存档可能不完整，并给出恢复前整份备份的位置与提交、回滚两处错误。暂存或提交失败后的临时文件清理失败改为逐条列出，不再静默忽略。
- **`secret import` dry-run 预览说明如实化**（#143）。文档与预览输出写明它本来就是的行为：仅限 DM 的本地审阅清单，最多列出 20 条待确认词，不写 `secret-terms.yaml` 也不写会话日志，不得用于玩家可见材料。预览行为本身没有变化。

### 更新说明 / 迁移
- **覆盖式更新**：下载 1.0.1 发布包，把包内 `TalkDND/LetsPlay/` 的内容覆盖到现有游戏文件夹，覆盖同名文件；存档目录原样保留。不要用旧包整个替换。
- 无存档 schema 变更、无迁移：schema 仍为 8，更新后直接可玩。
- pdf2md 仍为 0.6.0，不重转已转换的书；两个程序（combat-sim、talkdnd）随发布包更新。
- v1.0.1 的内容是 #143 的质量修复（经 PR #146 合并），并经 #144 在真实 Windows 终端完成 9 个场景 / 90 个步骤的实机验证（0 失败、0 意外写入、0 数据泄漏）。发布提交只改版本号、CHANGELOG 与章程，不含行为源码变更；行为基线与已验证候选一致（候选 commit `140e081`，Windows ZIP SHA-256 `f39b11843c8cffd432d8502eb40e2bca34563a44c7384d0c598e47648ed71dcd`）。项目所有者 2026-10-25 决定接受版本号差异（行为基线已在 140e081 验证），不再为 1.0.1 重建并重跑 Windows 矩阵；若未来要求最终制品与候选逐字节一致，再在打标签前重建复测。

### 已知限制
- 检查点整份备份的目录名带秒级时间戳，同一秒内对同一检查点 restore 两次会撞名。这是 v1.0.0 就有的行为（#144 Windows 实机记录），不在 1.0.1 修复。
- 检查点不覆盖 memory/entities/relations/world-time、会话日志与 DM 工作文件，restore 不会回退这些内容（契约 §4.1 的冻结设计）。
- 跨引用警告可能包含 dm_only 实体的名字/别名与关系 id/类型，属 DM 视图输出，不得粘贴给玩家。

## v1.0.0 — 2026-10-25

### 新功能
- **检查点与快照恢复**（P01，#126）。`checkpoint snapshot|list|show|restore` 把恢复所需的机械状态与已揭示/已识别知识（progress、reveals、save-meta、original、identified、canon、house-rules，以及 Characters/NPCs/sheets/combat）打进 `checkpoints/<id>/`，manifest 里每个文件记 sha256；`snapshot diff <idA> <idB|current>` 输出两个时点之间的结构化差异。`restore` 必须加 `--yes`：恢复前先把当前存档整体备份；快照的 schema 比当前旧时按正常流程迁移（整档备份 + 会话日志一行），比当前新则拒绝；恢复后补一条会话日志并刷新完整性基线，`check` 保持干净。检查点保留最新 10 份，超出自动淘汰最旧的。检查点是 DM 本地恢复数据，不回退会话日志、`rolls.log`、dm-secrets/dm-plan 等 DM 工作文件。`start` 摘要显示最后检查点与较上次的变化。
- **冒险结构索引**（P07，#125）。`adventure index build|show|search` 为每本书生成 `index/structured.json`：只存标题、页码、锚点，从不输出正文；重建幂等，源文件未变时跳过并报告「索引已跳过（来源未变）」。
- **长期记忆与叙事风格**（P03，#132）。`memory add|list|remove` 记录长期事实与偏好（带 `source` 与标签）；玩家视图只含 player-confirmed，`--all` 是 DM 视图。`settings style show|set` 调整 narration/detail/pace/npc_voice/options，`set` 必须给 `--reason`，每次改动写会话日志。
- **实体与关系台账**（P04，#133）。`entity add|list|show|rename|retire` 登记 npc/location/faction/quest/item，支持别名；`relation add|list|end|show|context` 记录实体间关系，可按会话时点过滤（关系在 since_session 到 until_session 之间有效）、按 dm_only 过滤；`relation add` 对同一关系幂等；`entity add` 遇同名或同别名冲突一律拒绝并列出冲突项，不静默复用旧 id。
- **世界时间与预定事件**（P06，#139）。`time show|enable|disable|set|advance` 与 `time event add|list|done|skip`。默认关闭，关闭时 set/advance/event 写入被拒绝（show/list/enable 可用）；只有 `time set` / `time advance` 推进时钟；到期判定按标签精确相等；`dm_only` 事件默认隐藏。`start` 在启用时显示世界时间与今天到期的事件（只列玩家可见的）。
- **即兴 Oracle 与 dm-plan 模板**（P08，#140）。`oracle yes-no|event|meaning` 给 DM 即兴灵感：yes-no 只打印 `yes`/`no`，event 输出事件焦点短语，meaning 输出「短语（混乱 N）」。短语全部是本任务手写的原创内容，不来自任何书籍或随机表；三个子命令共用骰子引擎，隐藏的 `--seed` 可完整复现输出。Oracle 严格只读：不写任何存档文件。`original plan` 模板在「计划」「伏笔与回收」之外新增六个可选小节（当前局面、参与者目标、冲突、线索、可能行动、不介入后果）；已有 `dm-plan.md` 永不覆盖。
- **战斗质量加固**（#134、#135、#136、#137、#138）。非当前行动者的攻击默认拒绝（`--force` 显式 DM 覆盖）；战斗状态 JSON 纳入完整性清单；`combat-sim add` 在已存在遭遇中加入新战斗者；`talkdnd check` 支持工具熟练项检定（`check <who> tool:<name> --ability <属性>`）；`secret scan` / `secret import` 提供 DM 秘密词清单的只读覆盖率报告与显式批量导入，`start` 在 dm-secrets.md 有内容但零标记时预警。

### 变更
- 存档 schema 7 → **8**（只有 P03 的 `settings.style`，存进 save-meta）。首次 `start` 或相关写入遇到 schema 7 的存档，先整份备份到 `Saves/_backups/` 再迁移，重复运行不重复迁移。其余新数据都是带版本的 sidecar：version 1，文件缺失 = 功能未启用，高版本拒绝。
- integrity 清单覆盖 memory/entities/relations/world-time/checkpoints 与战斗状态文件（存在时才计入）。工具自己的写入成功后自动刷新基线；手改由 `check` 报告，可用 `integrity accept --reason` 记录。
- `start` 摘要新增：最后检查点、较上次的变化、关键记忆（玩家确认的固定事实）、启用时的世界时间；`check` 支持工具熟练项；命令面新增九个命令组（P00a，#124）；数据契约（P00，#122）与 sidecar 完整性（P00b，#123）同期落地。
- 三个程序版本统一为 **1.0.0**（combat-sim、talkdnd、talkdnd-dev）；pdf2md 仍为 0.6.0，不重转已转换的书。

### 修复
- 非当前行动者的攻击不再默认放行（#135）。
- 战斗状态写入失败不再推进完整性基线；之后的手改会被正常报告（#134）。
- 同名/同别名实体重复登记不再静默复用旧 id，改为拒绝并列出冲突（#133）。
- `time show` 与 `time event list` 的 dm_only 事件不再进入玩家视图，包括计数——`time show` 的总数与待办数都不把隐藏事件算进去（#139）。
- `combat-sim add` 不再重建遭遇，已有战斗者的 HP、资源、状态、先攻与遭遇日志全部保留（#137）。

### 更新说明 / 迁移
- **覆盖式更新**：下载 1.0.0 发布包，把包内 `TalkDND/LetsPlay/` 的内容覆盖到现有游戏文件夹，覆盖同名文件；存档目录原样保留。不要用旧包整个替换（会丢存档、规则书和冒险书）。
- 更新后第一次 `bin/talkdnd start` 会把 schema 7 的存档备份到 `Saves/_backups/` 再迁移到 8；重复运行不重复迁移。确认无误后可删除旧备份。
- 旧存档不需要任何新 sidecar：memory/entities/relations/world-time/checkpoints 都按需启用，文件不存在 = 功能未启用。检查点、实体、世界时间默认不改变现有战役的行为。
- pdf2md 不重转：已转换的书保持原样；两个程序（combat-sim、talkdnd）随发布包更新。
- **1.0.0 是第一个稳定版**，把原计划的 v0.12/v0.13/v0.14 三个切片收为一个里程碑（项目所有者 2026-10-25 决定，见 docs/PROJECT-CHARTER.md Decision Log）。

### 已知限制
- 检查点不覆盖 memory/entities/relations/world-time、会话日志与 DM 工作文件，restore 不会回退这些内容。
- 检查点 id 精确到秒，同一秒内打多份快照靠 `-2`/`-3` 后缀区分；按 id 排序不代表真实创建先后。restore 不回退会话日志、`rolls.log`、dm-secrets.md/dm-plan.md 等 DM 工作文件。
- 实体/关系不会自动出现在 `known` 或 `start` 摘要；`check` 不做跨引用全表扫描，引用校验发生在写入时。
- 世界时间不做历法算术（月长、闰年、标签先后都不处理）；极端大的已过天数下 f64 精度可能吞掉纯小时增量，此时 `time advance` 报告「已过天数未变化」且不改动存档；`start` 遇到损坏的 world-time.yaml 会静默省略该节；手改由 check 的完整性报告兜底（YAML 解析错误本身不另报）。
- 冒险索引只存标题/页码/锚点，不自动抽取实体或线索。
- Oracle 短语表是手写原创，程序无法自动证明「不含书籍文本」；`--seed` 仅供测试复现。
- `start` 的零标记预警只看 dm-secrets.md；dm-plan.md 不在 #131 的扫描范围内。

## v0.11.0 — 2026-10-24

### 新功能
- **游玩中修正书页**（#108）。`pdf2md fix --book "<书名>" --page N [--from 文件] [--reason "原因"]` 把核对过的一页 Markdown 写进该书的 `index/fixes/pNNNN.md`，首行记录 PDF 的 sha256；没有转换进程时立即重组，下一次 `adventure section` 读到的就是修正版。`--no-apply` 只存修正；覆盖旧修正前自动备份到 `fixes/backups/`。页号越界、该页尚未转换、输入为空都会拒绝。
- **书库收口：整本与分卷并存**（#109）。`talkdnd library merge <slug> [--dry-run]` 把 `<slug>-a`、`-b`、`-2`、`-part-2` 等分卷文件夹整体归档到 `.talkdnd/book-backups/merge-<slug>/`，从不删除；分卷的 `combat/` 只在整本没有时移入；存档里的旧路径重指向整本。`library tidy` / `start` 发现整本与分卷并存时会提示这条命令，不会自己搬。

### 变更
- **开发契约与检查清单对齐代码**（#105）。`AGENTS.md` 与 `README.md` 现在写明玩家运行 `combat-sim` 与 `talkdnd` 两个程序；`fetch-binary` / `install-local` 装两个制品、做两次构建；补上 `check-leaks`、`edition: github` 禁止 `orchestrator.py init`、仓库地图、存档 schema/版本和 CI 检查入口；README 开发检查补齐 `tools/talkdnd`、kit unittest、pdf2md pytest、cargo deny、yamllint 与单测过滤。
- **pdf2md 游玩中修正流程**（#108）。`LetsPlay/docs/pdf2md.md` 新增「游玩中修正」；`LetsPlay/AGENTS.md` 在原有行内补一句，仍为 150 行。
- **开发命令说明**（#105）。新增 `tools/talkdnd-dev/README.md`，列出六个子命令、用法与退出码。

### 修复
- 修正 `AGENTS.md` / `README.md` 把玩家程序只写成一个 `combat-sim`、以及 `install-local` 只构建一个 crate 的错误描述（#105）。

### 更新说明 / 迁移
- 覆盖式更新：下载新发布包，解压到临时位置，把包内 `TalkDND/` 的内容复制到现有游戏文件夹，覆盖同名文件；存档目录原样保留。
- 无存档 schema 变化，`start` 不需要迁移。
- pdf2md 的 VERSION 仍是 0.6.0：`fix` 只是新增命令，不改变提取格式，升级不会重转已转换的书。
- 书库里有整本与分卷并存时，`bin/talkdnd start` 会提示 `bin/talkdnd library merge <slug>`；运行前先确认整本 PDF 完整。旧分卷不会被删除。
- 游玩中修正的页保存在该书 `index/fixes/`，只在本机，不入库；PDF 换 hash 后旧修正自动失效。

### 已知限制
- 不会自动判断两份不同文件名的 PDF 是同一本书；`library merge` 只处理整本 + 分卷的命名结构，且需要显式运行。
- `pdf2md fix` 需要该页已经转换；未转换的页先 `pdf2md run --book … --pages …`。

## v0.10.0 — 2026-10-23

### 新功能
- **书库统一标准：一本书一个文件夹**（#103）。冒险书放在 `Adventures/<slug>/`，里面是原始 PDF、pdf2md 转换结果 `index/` 和战斗数据 `combat/`；规则书放在 `Rules/<版本>/<slug>/`，里面是 PDF 和 `index/`。程序数据 `Rules/chargen/`、`Rules/combat/`、`Rules/defaults/`、`Rules/rulings*.md` 不是书，留在原处。
- **slug 规则固定、可复现**（`talkdnd` 与 pdf2md 共用）：取 PDF 文件名（不含 `.pdf`），ASCII 字母和数字转小写保留；撇号 `'`、`’` 去掉；其余每一段连续字符变成一个 `-`；去掉首尾的 `-`。例：`Storm King's Thunder` → `storm-kings-thunder`，`SRD_CC_v5.2.1` → `srd-cc-v5-2-1`。书名里没有任何 ASCII 字母或数字时，用 `book-` 加 8 位十六进制哈希（FNV-1a 32 位）。详见 `LetsPlay/docs/layout.md`「书库」。
- **自动整理书库**：每次 `bin/talkdnd start` 先整理一次；也可以手动运行新命令 `bin/talkdnd library tidy`（加 `--dry-run` 只报告、不改动）。
  - 直接丢在 `Adventures/`、`Rules/2014/`、`Rules/2024/` 下的 PDF，以及放在名字不对的文件夹里的 PDF，连同旧版转换结果 `.md/<书名>/`，移进 `<slug>/` 和 `<slug>/index/`。名字不对、但里面已有 `combat/` 的整本书文件夹整体改名。
  - 每一处移动都在输出里报告（「书库整理：已移动 A → B」），并记进 `.talkdnd/library-moves.yaml`，存档里引用旧路径的地方随之改写。
  - 不知道版本的规则书（放在 `Rules/` 顶层或其他子目录）只提示放进 `Rules/2014/` 或 `Rules/2024/`，不移动。
  - 从不删除书文件；目标位置已有同名文件时两份都保留并提示手动处理。可以重复运行，第二次不会再有改动。pdf2md 正在后台转换时不整理，等它结束后再运行。
  - 手工放的 `.md` 文本不是标准格式，程序不读取，整理时会提示。
- 新建存档（`new`）和 `chargen set adventure` 会沿用已有的书文件夹（例如书名带不带 `The` 都认同一本），不会再为同一本书建第二个文件夹。多卷冒险（如 A/B 两册）按卷各自一个文件夹，查书时自动合并。

### 变更
- **pdf2md 0.6.0：只生成一份 Markdown**。转换结果只保留页码版 `<书名>.pages.md`（`<a id="pN"></a>` = PDF 第 N 页），不再生成去掉锚点的阅读版 `<书名>.md`（两份逐行相同，只差锚点行）。`index.md` 的「Read:」和 `index.json` 的 `book` 都指向页码版。PDF 在书文件夹里时输出到 `<slug>/index/`，还没整理的 PDF 仍输出到旧位置 `.md/<书名>/`。
- **升级不重转**：pdf2md 从 0.5.0 升级到 0.6.0 不重做任何分块，也不重新组装；只删除多余的阅读版、改写 `index.md`/`index.json` 的指向，并把 `manifest.json` 的版本记为 0.6.0。`talkdnd start` 整理书库时也会删除页码版旁多余的阅读版（先确认页码版存在且带页锚点），并报告删除了哪些文件、节省了多少字节。`pdf2md verify` 发现残留的阅读版会提示。
- 所有读取书的地方改用新结构，同时继续兼容旧结构 `.md/<书名>/`：`adventure section`/`search`、`known`、`handout`（登记表移到 `Adventures/<slug>/index/handouts.yaml`）、`data status`/`data import`（从页码版导入，自动去掉锚点行，导入结果与之前逐字节相同）、`start` 摘要的「规则来源」（列页码版）。
- combat-sim 加载冒险目录时跳过 pdf2md 输出目录 `index/`；书文件夹里有 `combat/` 时只读 `combat/`，没有 `combat/` 的书文件夹整个跳过。
- 存档 schema 升到 **7**。`talkdnd start` / `talkdnd migrate` 遇到 schema 6 的存档，先整份备份到 `Saves/_backups/`，再把存档文本文件里指向书的旧路径（散放的 PDF、`.md/<书名>/…`、书库整理移动过的路径）改成新路径，同步更新存档完整性指纹，并在会话日志记一行。以后每次 `start` 也会把新的书库移动同步到存档。
- **裁定文件不再“重复”**：`Rules/defaults/rulings*.md` 是随版本发布的出厂默认，每次更新覆盖，DM 不读也不改；`Rules/rulings*.md` 是本机副本，牌桌上只以它为准。`start` 同步时：副本缺失就复制；副本没改过（与上次复制的默认或任何历史出厂版本相同）就自动换成新默认；副本改过则保留，若默认也变了，提示你对照 `Rules/defaults/` 手动合并。以前没改过的旧副本也会被认出来并自动更新。
- 文档改为只要求放 PDF，不再建议手工转 Markdown：`LetsPlay/README.md`、`Adventures/README.md`、`Rules/2014/README.md`、`Rules/2024/README.md`、`AGENTS.md`、`docs/layout.md`（新增「书库」「裁定文件」两节）、`docs/pdf2md.md`、`docs/chargen.md`、`docs/handouts.md`、`docs/narration.md`、`docs/deviation.md`，以及 pdf2md、talkdnd、combat-sim 的 README。
- `talkdnd-dev check-leaks` 把书文件夹里的 pdf2md 输出（`index/` 下的 `*.pages.md`、`manifest.json`、`img/` 等）也当作不得入库的生成文件。
- 所有程序版本统一升到 0.10.0（talkdnd、combat-sim、talkdnd-dev）；pdf2md 升到 0.6.0。

### 修复
- 同一本书可能出现两个文件夹（存档建的冒险文件夹和 PDF 所在文件夹名字不同）的问题：新建存档和整理都沿用已有的书文件夹。
- 原有测试全部通过。新增测试覆盖：slug 规则、整理（dry-run 与实跑计划一致、散放 PDF 与旧 `.md/` 移入、带 `combat/` 的文件夹改名、未知版本只提示、手工 `.md` 只提示、重名两份都保留、幂等、阅读版删除并改写 `index.md`/`index.json`）、schema 6→7 迁移与之后的移动同步、旧结构仍可读、combat-sim 跳过书文件夹、裁定副本自动升级、pdf2md 0.6.0 输出位置和升级不重转。

### 更新说明 / 迁移
- **覆盖式更新**：下载新发布包，解压到临时位置，把包内 `TalkDND/` 文件夹里的全部内容复制到现在的游戏文件夹，覆盖同名文件。存档目录原样保留。不要把旧包的 `TalkDND/` 整个替换掉（会丢存档、规则书和冒险书）。
- 更新后第一次 `bin/talkdnd start` 会整理书库并报告每一处移动和删除的阅读版（及节省的字节），然后把存档迁移到 schema 7（先备份）。想先看会动什么，可以先运行 `bin/talkdnd library tidy --dry-run`。
- 已转好的书不会重新转换；后台 pdf2md 只做 0.6.0 的升级记录。
- 以前手工放的 Markdown 书（例如 `Adventures/<书名>.md`、`Rules/2024/<书名>-2024.md`）程序不再读取，整理时会提示；确认有对应 PDF 后可以删除。

### 已知限制
- pdf2md 正在后台转换时，`start` 跳过本次整理（输出里会说明），下次再整理。
- 名字里没有 ASCII 字母或数字的书，文件夹名是 `book-` 加哈希，不直观，但固定不变。
- 书库整理只看 PDF；只有手工 Markdown、没有 PDF 的书不会被移动。

## v0.9.0 — 2026-10-23

### 新功能
- **队友统一放在 `Characters/`**：存档里的全部队伍成员——玩家角色和 DM 操控的同伴——都放在 `Saves/<存档>/Characters/<名字>.md`，仍由 `sheets/<id>.yaml` 生成。以前同伴放在 `NPCs/`（不可游玩角色），和“队伍成员”的身份不符。
- **用数据区分玩家角色和同伴**：`sheets/<id>.yaml` 新增 `controller:` 字段，`player` 表示玩家角色，`dm` 表示 DM 操控的同伴。生成的角色卡在程序生成部分第一行显示「**操控**：玩家角色（player）」或「**操控**：DM 操控的同伴（companion）」。没有该字段的旧数据按位置和战斗块推断：还在 `NPCs/` 或 `kind: npc` 的算同伴，其余算玩家角色。
- **新命令名 `bin/talkdnd companion random|sheet`**：`companion sheet <草稿号> --name <名字>` 把同伴写到 `Characters/<id>.md`（`kind: npc`，`controller: dm`），并加进 party。旧命令 `bin/talkdnd npc random|sheet` 继续可用，会提示已改名，写入位置同样是 `Characters/`。同伴不能占用玩家角色的 id（会报错，请换名字）。
- `start` 摘要的「队伍」一节给每人标上［玩家角色］或［同伴（DM 操控）］；若有队友仍在 `NPCs/`，会给出警告。`bin/talkdnd known` 的队伍行写成「名字（id，玩家角色/同伴）：职业 等级…」。

### 变更
- 存档 schema 升到 **6**。`talkdnd start` / `talkdnd migrate` 遇到 schema 5 及以前的存档，会先整份备份到 `Saves/_backups/`，再做以下迁移：
  - 只移动队友：`NPCs/` 里被 `sheets/*.yaml` 的 `file:` 指向的角色卡，以及战斗块 `side: party`、在 `combat/campaign.yaml` party 名单或 `party-state.yaml` 里的角色卡，移到 `Characters/`。
  - `NPCs/_party.md`（同伴总览）移到 `Characters/_party.md`。总览里原来指向 `../Characters/` 的相对链接会改成同目录链接。
  - 剧情 NPC（没有战斗块，或战斗块不在队伍一方）留在 `NPCs/`。`NPCs/` 文件夹保留，可以为空。
  - `sheets/<id>.yaml` 只做文本级修改：改 `file:` 路径，并在下一行加 `controller:`，注释、未知字段和原有格式都保留。
  - 已经生成过的角色卡（含程序生成部分）加上「操控」一行，其余内容不变。
  - 完整性清单同步到新路径。迁移前就被手改过的文件，换到新路径后仍然标记，不会被迁移“洗白”。
  - 会话日志记一行「存档结构迁移 schema 5→6 …；队友移到 Characters/：…」。
  - 重复运行不会重复迁移。
- 重名处理：如果 `Characters/` 里已有同名文件，两份都保留，移过来的那份改名为 `<原名>-companion.md`（`_party.md` 则为 `_party-companion.md`），对应 `sheets` 的 `file:` 同步修改，迁移报告给出警告，请人工核对。
- `known` 的队伍列表改为列出 `Characters/` 里的所有角色卡，加上 party 名单里的成员。以前只列 `kind: pc`，所以用 `npc sheet` 生成的同伴（`kind: npc`）会被漏掉；`NPCs/` 里的剧情 NPC 不再算进队伍。
- 同伴角色卡抬头的「玩家」一栏从「DM（同伴 NPC）」改为「DM（同伴，DM 操控）」。
- `LetsPlay/AGENTS.md` 改用 `companion` 命令，开场前改读 `Characters/_party.md`。`docs/layout.md`、`chargen.md`、`narration.md`、`dice.md`、`combat.md`、`table-integrity.md`、`Saves/README.md`、示例存档（`example/save/`：同伴卡移进 `Characters/`，`NPCs/README.md` 说明只放剧情 NPC）和工具 README 同步更新。
- 所有程序版本统一升到 0.9.0（talkdnd、combat-sim、talkdnd-dev）。

### 修复
- `known` 漏列 `kind: npc` 同伴的问题（见上）。
- 原有测试全部通过。新增测试覆盖：schema 5→6 迁移（dry-run 不写文件；队友和 `_party.md` 移走、剧情 NPC 留下；`controller` 写入；内容不丢；完整性干净；备份；日志；幂等）、重名两份都保留、手改标记跟随到新路径、`companion sheet` 写到 `Characters/`、旧 `npc sheet` 仍可用并提示、同伴不能占用玩家角色 id、`known` 标明玩家角色/同伴。

### 更新说明 / 迁移
- **覆盖式更新**：下载新发布包，解压到临时位置，把包内 `TalkDND/` 文件夹里的全部内容复制到现在的游戏文件夹，覆盖同名文件。存档目录原样保留。不要把旧包的 `TalkDND/` 整个替换掉（会丢存档、规则书和冒险书）。
- 更新后第一次 `talkdnd start` 会生成 schema 5→6 的迁移备份，把队友移到 `Characters/`，并在输出和会话日志里列出移动了哪些文件。确认无误后可删除 `Saves/_backups/` 里的旧备份。
- 迁移后，同伴总览在 `Characters/_party.md`。DM 以后把新的剧情 NPC 写进 `NPCs/`，同伴一律放在 `Characters/`。
- 已写好的旧命令不用改：`npc random`、`npc sheet` 仍然可用。

### 已知限制
- 没有专门修改 `controller` 的命令。同伴改由玩家操控（或反过来）时，目前只能由 DM 编辑 `sheets/<id>.yaml` 的 `controller:`，再用 `talkdnd integrity accept --reason "…"` 记录。
- 判断 `NPCs/` 里的文件是不是队友，依据是 `sheets` 数据、party 名单、`party-state.yaml` 和战斗块的 `side: party`。如果一个剧情 NPC 也写成了 `side: party` 的战斗块（例如临时随队的向导），也会被当作队友移走。迁移报告会列出每个移动的文件，请核对。
- 从未由程序生成过的旧角色卡（没有程序生成部分）要等下一次 `talkdnd char` 写入时，才会显示「操控」一行；`sheets/<id>.yaml` 里的 `controller:` 在迁移时就已写好。
- 历史会话日志和旧的 `digest.md` 里提到的 `NPCs/...` 路径不会被改写（日志是历史记录）。`digest.md` 下次 `start` 时会重新生成。

## v0.8.0 — 2026-10-23

### 新功能
- **按场景读冒险书**：新增 `bin/talkdnd adventure section <章节|地点|关键词> [--list] [--reason "<原因>"]`。程序在本地转换好的冒险 Markdown（`Adventures/<slug>/.md/<书名>/<书名>.md`）里按 Markdown 标题切分，只打印标题匹配的那一节（含它的子标题），不再把整本书倒出来。`--list` 只列标题，不显示正文。章节是否“超前”按存档的冒险进度轨和当前章节判断：超出当前进度默认拒绝；确有必要时加 `--reason`，程序会打印该节并在会话日志记「超前阅读」。标为 DM-only 的小节同样要 `--reason`。没有本地 Markdown 时给出转换/设置冒险的提示。`adventure section` 的标题匹配也做了查询时归一化（大小写、引号、标点、连字、断行连字符），不修改源 Markdown。
- **冒险按页搜索**：新增 `bin/talkdnd adventure search "<词或短语>" [--limit 20]`（别名 `find`），在页码版 `<书名>.pages.md` 中逐行查找，每个命中带 `p.<PDF 页码>`；精确命中在前，行首容错命中标「（模糊）」。查询时归一化大小写、弯/直引号、撇号、标点、空白、连字（`ﬁ`→`fi`）和跨行连字符（`exam-` + `ple`→`example`），每行开头允许丢 1–4 个字符（扫描页边遮挡），因此 `known` 能命中 `..mown`。只读 Markdown，不重写书；`--limit 0` 不限条数。
- **pdf2md 开书体检与章节起点**：转换一本书时统计文字层字符数、0 字页、乱码率、扫描/数字版判断与书签数，给出 `text layer usable` / `OCR needed` 结论，写入 `manifest.json`、`index.json`，并在 `index.md` 的「Intake audit」「Chapter starts」两节展示；重复运行结果不变。CropBox 超出 MediaBox 的页会先复制单页并夹取副本的 CropBox 再读文字层，原 PDF 不动，不会仅因此误报 `OCR needed`。`pdf2md status` 与 `adventure search`/`section` 会显示简短结论。没有书签的书可用 `--chapter-open "<每章开头短语>"` 或 `PDF2MD_CHAPTER_OPEN` 识别章节起点，结果存进 intake 元数据；`pdf2md audit --chapter-open` 同样生效。命令示例：`uv run --project tools/pdf2md python -m pdf2md run --chapter-open "The Adventure Begins"`、`uv run --project tools/pdf2md python -m pdf2md audit --book "书名" --chapter-open "The Adventure Begins"`。
- **玩家已知清单 `bin/talkdnd known`**：汇总 `reveals.yaml` 的已揭示事实、`handouts.yaml` 已展示资料图、`identified.yaml` 已识别怪物/物品、已出场或隐藏面已揭示的原创内容、队伍角色卡基础信息、当前章节与地点。`bin/talkdnd scene "<地点>"` 把当前场景写进 `progress.yaml`、记日志并自动打印完整 `known`；`bin/talkdnd start` 的摘要里也带压缩版。
- **秘密词与防剧透检查**：
  - `bin/talkdnd secret add <词> [--chapter N] [--note "…"]`、`secret list`、`secret remove <词>`，手动秘密词只存在 `Saves/<存档>/secret-terms.yaml`（已 gitignore）。
  - 自动来源：`dm-secrets.md` 与 `dm-plan.md` 里的 `- term: X` / `[[X]]`、当前进度之后的章节标题、DM-only 小节的标题/`[[词]]`/连续大写英文专名、原创内容 `--hidden`。已出现在 `reveals.yaml` 里的词自动排除。
  - `bin/talkdnd guard "<草稿>"`（或 stdin / `-`）不区分大小写：英文按整词、中文按子串匹配，每个命中打印「剧透词：X（建议改写为玩家已知的说法；来源：…）」；有命中退出码 1，干净为 0。清单缓存在 `secret-terms.cache.json`，源文件变化后自动失效，检查很快。
- **怪物/物品名称遮蔽**：
  - 新增 `bin/talkdnd identify <东西> [--how check|reveal|lore] [--reason "…"]`，把识别记进 `Saves/<存档>/identified.yaml`；`bin/talkdnd identify --appearance <东西> "<描述>"` 可设置识别前对玩家使用的描述。
  - combat-sim 的 `status`、`log`、`simulate` 支持全局 `--player`：未识别怪物显示 appearance 或默认的「体型 + 类型」描述（如“一只大型野兽”），已识别的恢复真名，PC/同伴 NPC 永远显示真名。DM 视图不加 `--player`。
  - 存档位置从 `--campaign` 指向的 `<存档>/combat/campaign.yaml` 推导，也可用 `--save Saves/<存档>` 或环境变量 `TALKDND_SAVE` 指定。
- **原创内容隐藏面**：`bin/talkdnd original add … --hidden "<隐藏面>"`，隐藏面只写 `original.yaml`（DM-only），并进入 `guard` 的秘密词清单。`bin/talkdnd reveal add --original O-001 [--source "…"]` 记录隐藏面已揭示（自动加 `original:o-001` 标签；不写事实文本时自动生成一条揭示记录）。`bin/talkdnd original plan` 创建 `Saves/<存档>/dm-plan.md` 长期计划模板（只给 DM，已 gitignore）。`bin/talkdnd check` 会报告已登记隐藏面（或其 `[[词]]`）原样出现在会话日志里、却没有揭示记录的情况。
- **骰子显示开关**：`bin/talkdnd settings` 查看，`bin/talkdnd settings dice on|off` 切换；默认开，局中可改，每次改动写会话日志，`bin/talkdnd start` 显示当前设置。关闭后 DM 只叙述结果，不报骰值/DC/修正/HP 数字；所有骰子仍由程序掷并记录。
- **掷骰日志**：`Saves/<存档>/rolls.log` 追加记录 `talkdnd check`、`chargen roll`、起始金币、`npc random`、`level-up --hp roll`、`combat-sim roll`，以及 combat-sim 战斗状态命令（`attack`、`cast`、`use`、`save`、`check`、`damage`、`init`、`deathsave`、`rest` 等）中带骰式的行。只含骰式的行会写入，条件/叙述行不会，战斗 state 日志不变。`bin/talkdnd log rolls [--last N]` 随时查看。`combat-sim roll` 加 `--save` 或 `TALKDND_SAVE` 即写入同一份日志。
- **选项写法与防剧透文档**：新增 `LetsPlay/docs/option-writing.md`，给出“只从 `known` 出发、不报未公开 DC/做法、未发现内容不进选项、模糊情报保持模糊”的规则，以及三个改写自劫案式冒险的正误示例（只揭示机关被触发却报 DC、把未发现的暗门写进选项、拿粗略草图认定哪扇门有警报）。`LetsPlay/AGENTS.md` 压到 150 行并加入对应摘要；`docs/anti-spoiler.md`、`docs/dice.md`、`docs/combat.md`、`docs/deviation.md`、`docs/layout.md` 同步更新。

### 变更
- 存档 schema 升到 **5**：`save-meta.yaml` 新增 `settings: { dice_display: true }`。旧存档（schema 4 及以前）在 `talkdnd start` / `talkdnd migrate` 时先整份备份到 `Saves/_backups/`，再迁移、写会话日志；骰子显示默认为开，可随时切换。重复运行不会重复迁移。
- `original.yaml` 台账版本升到 **2**：新增可选 `hidden:` 字段。版本 1 的台账仍可直接读；迁移或下一次 `original` 写入时升到 2，旧条目 `hidden` 为空，不丢数据。
- `progress.yaml` 新增可选 `scene:` 字段（当前场景/地点），旧存档无需迁移。
- 所有程序版本统一升到 0.8.0（talkdnd、combat-sim、talkdnd-dev）。
- combat-sim 的 creature 数据新增可选 `appearance:` 字段（`deny_unknown_fields` 下仍兼容旧数据），并与 `base:` 继承合并。

### 修复
- 同一行被不同字体拆成多个 span 时，先按顺序拼接再分词、识别标题和统计词数，修复 `exam` + `ple` 被当成两个词、标题识别丢失的问题（fallback、figures、headings、rooms 共用 `textutil.line_text`）。
- pdf2md 把超出 MediaBox 的 CropBox 夹到 MediaBox 后再提取、OCR 和渲染整页图；单页读取或渲染失败不再中断整个分块。开书体检遇到同类页时复制单页、夹取副本的 CropBox 再读文字层，原 PDF 不变。`cropbox-clamped`、`page-fallback`、`page-error`、`render-failed` 写进 manifest 页条目，并计入 `pdf2md status` 的 `page fallback` 与 `pdf2md audit` 的 `fallback_pages`。
- pdf2md worker 锁改为操作系统文件锁（POSIX `flock`、Windows `msvcrt.locking`），锁文件同时记录 pid 与时间戳，不再有“先删后建”的空窗；活进程的锁不会被抢，死 pid 或超过 600 秒未更新的旧锁自动接管。释放前先写 `released: true`，Windows 上删除失败留下的文件也会被下次启动当作陈旧锁接管。
- `known` 与 `start` 摘要里的队伍行改为读取结构化角色卡的职业（不再显示 `?`），摘要中子项缩进显示。
- `guard` 的英文秘密词要求整词匹配（`Inn` 不再命中 `inner`），并去掉转换残留的 `_`/`*` 标记；单个英文单词的章节标题（如 `Wharf`）太泛，不再自动当作秘密词（DM-only 小节里的专名照常收集）。秘密词缓存版本升级，旧缓存自动失效。
- `adventure search` 先列出精确（归一化后）命中，再列出行首容错命中，后者标注「（模糊）」。
- combat-sim 玩家视图里多个同类未识别怪物带编号区分（如「一只中型野兽 #1」「#2」）。
- 建卡、战斗、迁移、校验、资料图和偏离度流程保持兼容（原有测试全部通过，并新增覆盖：guard 命中/干净/已揭示排除/stdin/退出码、名称遮蔽与识别、`known` 内容、超前阅读拒绝与 `--reason` 日志、骰子设置切换、schema 4→5 迁移、原创 `--hidden` 进入 guard 与日志泄漏检查、模糊搜索页码、intake/章节起点、CropBox 与坏页兜底、锁并发与接管）。

### 更新说明 / 迁移
- **覆盖式更新**：下载新发布包，解压到临时位置，把包内 `TalkDND/` 文件夹里的全部内容复制到现在的游戏文件夹，覆盖同名文件。存档目录原样保留。不要把旧包的 `TalkDND/` 整个替换掉（会丢存档、规则书和冒险书）。
- 更新后第一次 `talkdnd start` 会生成 schema 4→5 的迁移备份，并在会话日志写「存档结构迁移 schema 4→5 …；骰子显示默认开」。确认无误后可删除 `Saves/_backups/` 里的旧备份。
- 新文件 `identified.yaml`、`secret-terms.yaml`、`rolls.log`、`dm-plan.md` 都只在存档里按需创建，均已 gitignore；不要提交。
- 已经写好的旧命令不用改；`original add` 不带 `--hidden` 时行为与 0.7.0 相同。

### 已知限制
- `adventure section` 依赖本地转换后的 Markdown，并且只按 Markdown 标题切分；PDF 没转换、标题层级混乱或一本书有多个 `.md` 子目录时，可能找不到想要的小节。先看 `--list`，必要时查 `index.md` 或 PDF。
- `adventure search` 是逐行子串/前缀模糊匹配，不是语义检索；只查页码版，扫描页 OCR 把词认错时仍可能漏。命中多时用 `--limit`，`--limit 0` 会输出全部命中。
- 章节起点短语按“每页第一行开头”匹配，短语太常见会误把正文页当章节；先 `--list` 或看 `index.md` 的「Chapter starts」核对。CropBox 夹取只作用于转换/渲染和体检时的单页副本，原始 PDF 不会被修改。
- “超前章节”靠进度轨名称、章节编号和当前队伍等级推断；没有进度轨的自制冒险不会阻止任何章节，DM 仍要自觉按场景读。
- `guard` 是英文整词/中文子串提醒，不是语义判断；改写过的泄密说法可能漏掉，查到同名词也不一定是剧透。已揭示的词靠 `reveals.yaml` 排除，DM 忘了记录时仍会误报。
- 秘密词中的中文只从 `- term:`、`[[…]]`、章节标题和原文中的标记收集；不会自动从任意中文段落里猜专名。需要检查的中文词请显式标记。
- 名称遮蔽只覆盖 combat-sim 的 `status`/`log`/`simulate` 文本和 `--player` JSON；攻击、伤害等逐条命令的输出仍是 DM 视图。不要把 DM 视图直接投给玩家。
- 骰子显示关闭只是叙述约定，程序不会阻止 DM 说出数字；所有掷骰仍会记录在 `rolls.log`（战斗状态命令只记录带骰式的行），DM 不能以“关显示”为由跳过掷骰。
- `identified.yaml`、`secret-terms.yaml`、`rolls.log` 不在 `integrity.yaml` 的指纹清单里；它们是 DM 工具数据，手改不会触发存档校验警告。

## v0.7.0 — 2026-10-22

### 新功能
- **每个存档的「剧本偏离度」**：开新档时用一个档位和一个比例锁定 DM 可以离冒险书多远，之后不能修改（没有修改命令）。
  - `scope 0`：不偏离，只能补必要空白（给未命名 NPC 起名、对白措辞、场景描述）；补空白不算原创。
  - `scope 1`：可以加入原创遭遇、NPC、物品、地点；稀有度上限 `uncommon`。
  - `scope 2`：在 1 的基础上可以加入完整支线；稀有度上限 `rare`。
  - `ratio` 是每章原创内容占「原创 + 正典进度」的比例上限，整数 5–25%；scope 0 不设比例。
  - 开档示例：`bin/talkdnd new --ruleset 2014 --scope 2 --ratio 15`。交互终端不带参数时程序用中文询问；非交互环境默认 scope 0 并打印提示。scope 3 及以上会被明确拒绝。
- **原创内容台账 `bin/talkdnd original`**（只给 DM）：
  - `original add --type <encounter|npc|item|location|sidequest> --name "<名字>" [--rarity <common|uncommon|rare|very-rare|legendary>] [--note "<备注>"] [--hook player]`
  - `original list`：列出本档全部原创条目（编号 `O-001` 起）。
  - `original status`：显示档位、比例、允许类型、稀有度上限（档位与队伍等级取较低者）、当前章节、本章正典进度、本章原创数与占比、剩余可登记项、全战役总数与占比。
  - 规则：先登记再引入；物品类型必须给 `--rarity`；超类型、超稀有度或超预算都会拒绝，预算不足时提示**回到主线**。采用玩家钩子加 `--hook player`，同样算原创、占预算。
- **预算按章计算**：原创 + 1 <= floor(ratio% × (原创 + 1 + 本章正典数))。正典数取 `progress.yaml` 的 `xp_log` 条数 + `reveals.yaml` 的已揭示事实数；本章从最近一次 `level-up --milestone` 起算（含该条），没有里程碑时按全战役累计。全战役总数只展示，不作为拒绝依据。规则和示例见 `docs/deviation.md`。
- **`char item add` 来源检查**：规则数据里的武器、护甲、盾牌和常见装备直接通过；其他名称需要 `--canon`（DM 查过书）或 `--original O-xxx`（本档已登记的物品原创），否则拒绝并给出提示。原创标记只存在 `sheets/<id>.yaml`，不会出现在给玩家看的角色卡、资料图或摘要里。
- **`talkdnd check` 扩展**：报告 `save-meta.yaml`（含 `deviation:`）和 `original.yaml` 在工具以外被改、台账条目类型/稀有度超出 scope 或队伍等级上限、角色卡物品标了原创但台账没有对应条目；旧存档迁移继续保留原有的手改标记。
- **`talkdnd start` 摘要新增「剧本偏离度（只看自己）」**：档位、比例、允许类型、稀有度上限、当前章节、本章预算和全战役总数，方便开场前核对。

### 变更
- 存档 schema 升到 **4**：`save-meta.yaml` 新增 `deviation: { scope, ratio }`。`talkdnd start` / `talkdnd migrate` 会先把整份存档备份到 `Saves/_backups/`，再迁移并写会话日志；重复运行不会重复迁移。
- 完整性清单（`integrity.yaml`）现在也跟踪 `save-meta.yaml` 和 `original.yaml`；旧清单里没有这两个文件时，第一次工具写入会把它们纳入基线，不会误报。
- 所有程序版本统一升到 0.7.0（talkdnd、combat-sim、talkdnd-dev）。
- 文档：新增 `LetsPlay/docs/deviation.md`；`AGENTS.md` 增加「剧本偏离度（必须遵守）」；建卡文档、存档结构、校验文档同步更新。

### 修复
- 无行为修复；本版主要是新功能与文档。已有建卡、起始装备、背景装备、`char` 物品/装备、迁移和校验流程保持兼容（原有测试全部通过，并新增覆盖）。

### 更新说明 / 迁移
- **旧存档自动变成 scope 0**：升级后第一次 `talkdnd start`（或 `talkdnd migrate`）会给旧存档写 `deviation: { scope: 0, ratio: null }`，表示「不偏离」。旧存档不能登记原创；想要原创只能开新档。
- **覆盖式更新**：下载新发布包，解压到临时位置，把包内 `TalkDND/` 文件夹里的全部内容复制到现在的游戏文件夹，覆盖同名文件。存档目录原样保留，不需要手工改 `save-meta.yaml`。不要把旧包的 `TalkDND/` 整个替换掉（会丢存档、规则书和冒险书）。
- 更新后第一次运行会生成迁移备份；确认无误后可以删除 `Saves/_backups/` 里的旧备份。

### 已知限制
- 章节根据队伍最低等级和进度轨推断；自制冒险没有 `track` 时全部算「未分章」，预算按整场战役累计。
- 正典计数是记录条数（经验/里程碑 + 已揭示事实），不是剧情重要性；需要 DM 按正常流程记录，不能为了腾预算灌水。
- 名称来源检查只覆盖规则数据里的武器、护甲、盾牌和常见装备；魔法物品、书里特有装备需要 `--canon` 或登记为原创。
- 偏离度没有修改命令，这是有意为之；开档时务必先向玩家解释清楚再执行 `new`。
- 存档校验用于防手滑，不是防作弊的安全边界；`original.yaml`、`dm-secrets.md` 和摘要都只给 DM 看。

## v0.6.1 — 2026-10-22

### 修复
- **游戏文件夹可以放在任意位置、叫任意名字**：以前程序要求玩家保留 `TalkDND/LetsPlay` 这样的目录结构，或者必须在 `LetsPlay/` 里运行，否则找不到游戏文件夹。现在 `talkdnd` 按以下顺序找游戏文件夹，与当前目录和文件夹名字都无关：
  1. `--root <文件夹>`（相对路径按当前目录解析）；
  2. 环境变量 `TALKDND_ROOT`（不是游戏文件夹时直接报错）；
  3. 程序所在 `bin/` 的上一级（解压出来的包就是这种情况，从哪个目录运行都行）；
  4. 当前目录及其上级（开发仓库里的 `LetsPlay/` 子目录仍然支持）。
  游戏文件夹以根部新增的标记文件 `.talkdnd-root` 识别（旧的 `AGENTS.md` + `Saves/` 判断仍然有效）。路径里有空格或中文也没关系。
- **combat-sim 同样不依赖当前目录**：`--campaign`（或 `COMBAT_SIM_CAMPAIGN`）给的是相对路径、而当前目录下没有这个文件时，会到游戏文件夹（`TALKDND_ROOT`，或程序所在 `bin/` 的上一级）里找，所以 `bin/combat-sim --campaign Saves/<存档>/combat/campaign.yaml …` 在任何目录下都能用。存档清单里的库路径本来就是相对清单文件的，不受影响。
- pdf2md 总是在游戏文件夹里以 `uv run --project tools/pdf2md` 启动，不依赖当前目录（原本如此，本次核对确认）。

### 变更
- **发布包的顶层文件夹改名为 `TalkDND/`**（以前是 `LetsPlay/`，那是开发仓库里的目录名）。里面的内容是自包含的完整游戏文件夹，可以随意移动和改名。
- 发布流程新增冒烟测试：把刚打好的包解压后改名为带空格和中文的文件夹，从另一个目录运行 `talkdnd new`、`chargen status`、`start` 和 `combat-sim roll`，任何一步失败都不发布。
- 新增集成测试 `tools/talkdnd/tests/portable.rs`：把最小游戏文件夹复制到任意命名的临时目录，从别的目录运行建档、建卡、接档和战斗内容检查，并确认当前目录里不会写入任何东西。
- 文档：`AGENTS.md`、`docs/` 各篇、`Rules/defaults/` 裁定、各 README 的路径说明都改成“相对于游戏文件夹”，不再写死 `LetsPlay/`。README 的解压和更新步骤按新的包结构重写。

### 更新说明 / 迁移
- 已经在玩的：照常更新。下载新包，解压到临时位置，把 `TalkDND/` 文件夹**里面的全部内容**复制到你现在的游戏文件夹（即使它还叫 `LetsPlay`，或者被你改过名），覆盖同名文件。这样会带上新的 `.talkdnd-root` 标记文件。存档不需要迁移。
- 新玩家：解压到任意位置，文件夹随意改名，在里面启动 Agent 即可。
- 请不要删除游戏文件夹根部的 `.talkdnd-root`。

### 已知限制
- 如果把 `bin/` 里的程序单独复制到别处运行，程序就找不到游戏文件夹了，这时需要 `--root` 或 `TALKDND_ROOT`。
- 通过符号链接调用程序时，按链接指向的真实位置找 `bin/..`。
- combat-sim 不加 `--campaign` 时仍然不读取存档（与以前相同）；DM 手册里的命令都带 `--campaign`。
- Windows 包没有跑解压冒烟测试（只在 Linux、macOS 上跑）；Windows 上的路径查找由同一套代码和单元测试覆盖。

## v0.6.0 — 2026-10-22

### 新功能
- **覆盖式更新**：把新版发布包直接解压覆盖到旧的 `LetsPlay/` 文件夹即可更新。发布包里只有程序文件和说明，不含任何会覆盖用户数据的文件（存档、规则书和冒险书、本地导入 `local/`、`fixes/`、`handouts.yaml`、用户的 `Rules/rulings*.md`、`.talkdnd/` 都不在包里，打包时自动检查，命中即发布失败）。
- **存档结构版本与自动迁移**：每个存档有 `Saves/<存档>/save-meta.yaml`（结构版本、上次使用的程序版本、迁移记录）。当前结构版本为 3。`talkdnd start` 发现存档结构比程序旧时，先把整个存档备份到 `Saves/_backups/<存档>-<时间>-schema<N>/`，再依次迁移（1→2：Markdown 角色卡转为 `sheets/<id>.yaml` 结构化数据；2→3：写入 `save-meta.yaml`，给 `progress.yaml`、`reveals.yaml`、`party-state.yaml` 加版本字段），在会话日志写一行迁移记录，并更新完整性清单。迁移本身不会被当成手改，但迁移前已经被标为手改的文件仍然保持标记。重复运行不会重复迁移。存档结构比程序新时会报错，提示先更新程序。
- 新命令 `talkdnd migrate [--dry-run]`：手动预览或执行迁移。
- **本地导入数据版本**：`Rules/chargen/local/imported-<规则>.yaml` 记录导入器版本（当前为 2）；版本过旧或书有更新时，`start` 自动重新导入。新导入的子职业和背景数量比旧的少时（例如某本书的 Markdown 缺失或识别变差），旧文件保留为 `.bak` 并给出警告。
- **规则裁定不再被更新覆盖**：仓库自带的裁定移到 `Rules/defaults/rulings*.md`，你的 `Rules/rulings*.md` 不在发布包里。缺失时 `start` 从默认值复制；默认值有更新而你的文件改过时，只提示你去合并，不会覆盖。
- **旧版遗留文件清理**：发布包带 `LetsPlay/.talkdnd-files.txt`（随包文件清单和 sha256）。`start` 对比上一版的清单，把新版已删除、且你没改过的旧程序文件移到 `.talkdnd/obsolete/<时间>/`，不直接删除；改过的只提示。
- **发布说明由更新日志生成**：`talkdnd-dev release-notes <版本>` 输出 `CHANGELOG.md` 对应小节和下载说明，发布流程直接用它作为 Release 正文。
- `TALKDND_TIMING=1` 时 `talkdnd start` 在 stderr 输出各阶段耗时。
- 新增 `CHANGELOG.md`（本文件）和 `docs/versioning.md`（版本号规范：`0.0.x` 小修小补，`0.x` 功能增删或大改，`x.0` 大版本；各位逐位递增，不按十进制进位）。

### 变更
- **`talkdnd start` 提速**：战斗内容检查的结果缓存在 `Saves/<存档>/.cache/content-check.json`，以 combat-sim 程序和它读取的每个文件（大小+修改时间）为键，没变化就不再启动 combat-sim；去掉了每次启动都要跑的 `uv --version` 探测（直接启动 pdf2md，缺 uv 时报告“未启动”）；会话日志追加不再读取整个文件；每条命令只做一次完整性快照。
- 升级（`level-up`）后自动重算法术位、契约法术位、职业资源上限和施法攻击/DC，当前值按新上限截断；结构化角色卡同样适用。
- README 新增「更新到新版本」步骤。

### 修复
- **存档写入改为原子写**：进度、角色卡、建卡记录、已揭示信息、资料图登记、本地导入、队伍状态、完整性清单等都先写临时文件再替换，中途断电或崩溃不会留下半个文件。pdf2md 的状态文件写入也会先落盘再替换。
- 重新运行 `chargen sheet` 不再清空游戏中积累的数据（物品、备注、DM 备注、专长、语言、工具、战斗风格、钱币），只刷新建卡推导出的字段。
- 角色和 NPC 名字转换成文件名时统一做安全处理，id 只允许小写字母、数字和连字符，任何路径都不能跑出存档目录（例如 `../../evil`）。
- 等级是 -1、0、25 之类的异常值或生命骰面数为 0 时，不再崩溃，给出明确错误。
- 队伍短休先检查生命骰够不够，再扣除；不会扣到一半出错。
- `handout build` 时如果某张图找不到了，保留原登记和标注（标为 `missing`）并警告，不再丢掉。
- `pdf2md status` 遇到格式不对的清单时显示错误行，不再整体中止。
- `chargen set gold` 的提示改成真实存在的命令。
- 优势和劣势同时存在时一律抵消为普通掷骰（包括先攻），不再取决于先后顺序。
- 抗性和易伤按伤害类型合计后只计算一次（两次 1 点挥砍对抗性 = 1，而不是 0）。
- 手动 `heal` 会计入目标的“受到治疗”加成。
- `char damage` 把 HP 降到 0 时自动加上“昏迷”等濒死状态；治疗、死亡豁免掷出 20、长休、`combat-sim party set-hp` 回到 HP>0 时自动移除。
- 角色卡和完整性清单的版本号会被检查，比程序支持的新时报错并指出文件。

### 更新说明 / 迁移
- 先存档并退出正在运行的 talkdnd / combat-sim（Windows 上运行中的 `.exe` 不能被覆盖），再把新包解压到 `LetsPlay/` 的上一级目录并选择全部覆盖。不需要挑文件，也不需要先删旧版。
- 第一次 `talkdnd start` 会自动备份并迁移存档、按需重新导入本地书籍数据、复制缺失的默认裁定；输出里会说明做了什么。备份在 `Saves/_backups/`，确认没问题后可以自行删除。
- 如果你改过旧版的 `Rules/rulings*.md`：这些文件不会再被覆盖。新版默认裁定在 `Rules/defaults/`，`start` 提示时可自行对照合并。

### 已知限制
- 旧版遗留文件只能在 v0.6.0 之后的更新中识别（v0.6.0 之前的发布包没有文件清单）。
- 自动迁移只覆盖已知的旧结构；手改过的存档迁移后仍会被完整性检查标出，需要 DM 确认。
- 审阅中的以下问题本版未处理：暴击范围（19–20）、力竭的实际效果、场外检定不计状态、`charsheet` 重算 AC 时不考虑职业无甲防御和护甲熟练、`rest all` 跨队伍不是原子操作、`char set class` 不清理旧熟练和攻击、冒险章节数据写死在程序里、休息和死亡豁免的重复代码合并、pdf2md 锁接管的竞争问题。

## v0.5.0 — 2026-10-22

### 新功能
- **资料图（handout）**（#91）：把本地 pdf2md 转换出的冒险书图片（地图、信件、物品图）给玩家看。
  - 每本转换后的书一个登记表 `Adventures/…/.md/<书名>/handouts.yaml`（本地、gitignore），由 pdf2md 的 `index.json`（地图图注、插图、整页图）和 `img/` 生成。
  - **默认全部「仅城主」**，只有图注写明玩家版（Player Version）的地图自动标为玩家可见。
  - `talkdnd handout show <id|关键词> [--crop x,y,w,h] [--png]`：只导出玩家可见的图到 `Saves/<存档>/handouts/`，打印文件路径和 Markdown 图片行，同时写入已揭示账本和会话日志；城主图一律拒绝。
  - `handout list [--dm]`、`handout mark <id> --player|--dm [--crop …]`、`handout build`。登记了裁剪区域后只能导出这一块。
  - AGENTS.md 新增资料图规则（何时展示、绝不展示城主图、不同宿主的展示方式），新文档 `docs/handouts.md`。

### 修复
- **迁移后的 AC 叠加防御风格**（#91）：旧角色卡迁移时，原 AC（已含防御 +1）被当作无甲基础值，`set fighting-style defense` 又加了 1（19 → 20）。现在迁移会把原卡写到的护甲和盾牌变成已装备物品（能算出原 AC 时），AC 由物品计算；算不出来就保留原 AC 并标记 `ac_base_has_style`，不再重复加防御。

### 更新说明 / 迁移
- 已经手动加了护甲物品绕过此问题的存档不受影响，不需要改。
- 资料图需要冒险书已经用 pdf2md 转换过。

### 已知限制
- 程序无法判断图片内容；能否给玩家看完全靠城主判断，所以默认仅城主。
- 图注、区域来自 pdf2md 的启发式结果；整页图常带正文，多半要裁剪。裁剪只能是矩形。
- 原卡写了无法识别的护甲时，AC 固定为原值，直到用 `char … item add/equip` 加上真实护甲。

## v0.4.0 — 2026-10-22

### 新功能
- **角色卡字段对齐官方角色卡**（#89）：战斗风格（2014 职业特性 / 2024 专长）、背景特性/专长、工具、语言、背景装备与金币、职业起始装备（2014 装备组或掷金币，2024 方案 A/B/C）、专长、性格特点/理想/羁绊/缺点、阵营。`chargen set fighting-style | class-equipment | background-equipment | languages | tools | feats | traits | ideals | bonds | flaws | alignment`；装备和金币按数据自动发放并记账。
- **战斗风格效果进入 combat-sim**：巨武器战斗（2014 重骰 1–2 一次；2024 把 1–2 当 3；双手武器和可双手的多用武器）、防御（着甲 +1 AC）、对决（单手近战 +2 伤害）、箭术（远程攻击 +2 命中）。新增攻击字段 `reroll_damage_below`、`damage_die_min`。
- **结构化角色数据与 `talkdnd char` 命令**：`Saves/<存档>/sheets/<id>.yaml` 是唯一数据源，实时数值在 `combat/party-state.yaml`（与 combat-sim 共用），Markdown 角色卡由程序渲染。命令：伤害/治疗/临时 HP、资源与法术位使用/恢复、短休/长休与生命骰、状态、力竭、激励、死亡豁免、多币种金币、物品增删/装备/同调（AC 与攻击自动重算）、弹药、笔记、DM 备注、`set`、原子化 `batch`；每条一行输出，自动写日志并保持完整性清单干净。
- **完整规则书优先**：本地导入同时提取背景；`talkdnd start` 在本地数据缺失或过旧时自动导入，并把完整规则书列为首要规则来源（SRD 只兜底）；`data status` 显示规则来源。
- 新文档 `docs/character-data.md`；AGENTS.md 开新局流程 1–14 步、「角色卡只能用 `char` 命令改」。

### 更新说明 / 迁移
- 旧角色卡第一次用 `char` 命令时自动迁移为结构化数据，正文原样保留，数值不变。用 `char <角色> set …` 补缺的栏目（战斗风格、背景等）。

### 已知限制
- 背景导入是启发式的，2024 版部分背景会漏掉（OCR 质量差）。
- 保护、双武器等风格只记为备注，由 DM 处理；对决 +2 不检查副手。
- 占位装备项需要手动命名；怪物数据仍来自 SRD 导入。

## v0.3.0 — 2026-10-22

### 新功能
- **桌面诚信与防作弊**（#87）：AGENTS.md「桌面诚信」一节（场外指令、作弊、意图与结果、开局前约定的房规存进存档）；`docs/table-integrity.md`。
- **存档完整性清单**：`integrity.yaml` 记录 `progress.yaml`、角色卡、`combat/*.yaml`、`reveals.yaml`、`house-rules.md` 在每次程序写入后的指纹；`start`/`check` 提示被手改的文件；`talkdnd integrity status|accept --reason`。
- **`talkdnd check`（无参数）对账**：HP、金币、物品、XP 的变化与会话日志、`xp_log` 对账。
- **防剧透**：`docs/anti-spoiler.md`（信息纪律、按需阅读、回复前自检、说漏了怎么办）和已揭示账本 `talkdnd reveal add|list|check`；摘要显示最近揭示。

### 更新说明 / 迁移
- 旧存档第一次运行时自动建立完整性基线。

### 已知限制
- 完整性清单只能发现改动，不能阻止；对账依赖日志行格式。

## v0.2.0 — 2026-10-21

### 新功能
- **pdf2md 0.4.0 阅读版与分页版**（#75）：`<书>.md` 阅读版（无页锚点）和 `<书>.pages.md` 查页版；索引链接到分页版；阅读版漂移时校验失败；真正重转前先备份 `fixes/`。
- **pdf2md 0.4.1 数据块检查**（#78）：怪物数据块可疑处加 ⚠ 标记。
- **pdf2md 0.5.0 图片阶段**（#80）：从扫描页切出插图、整页图，`contact-sheet` 生成缩略图总览。

### 变更
- **移除可打印角色卡**（#81）：删除 `talkdnd sheet export`（PDF/HTML）和 `sheet blank`（#76 曾加入 v2 版式），依赖 krilla/lopdf/ttf-parser 一并去掉；DM 不再提供打印卡，`chargen sheet`（Markdown）保留。
- 文档：pdf2md 阅读版/分页版、数据块检查、图片阶段（#83）。

### 已知限制
- 图片阶段的插图切割是启发式的；扫描质量差的书效果有限。

## v0.1.7 — 2026-10-21

### 新功能
- **混合制成长**（#73）：事件给 XP、里程碑升级。`xp add [N] --category --reason [--difficulty] [--again]`，`xp_log` 记录、重复拒绝；2014 DMG / 2024 DMG 门槛与预算；XP 最多领先角色卡一级，超出部分记为暂扣；`level-up <角色|all> --milestone --reason`；`xp show` 显示金库劫案进度。

### 修复
- 只加载与战役规则一致的 `2014/`、`2024/` 子文件夹（#71），修复 2024 存档读到 2014 劫案数据导致的 95 个错误。

### 更新说明 / 迁移
- 旧 `progress.yaml` 原样可读（新字段默认为空）。

## v0.1.6 — 2026-10-21

### 新功能
- `talkdnd sheet export`：可填写的 A4 PDF/HTML 角色卡（#68）。（v0.2.0 已移除）

## v0.1.5 — 2026-10-21

### 修复
- combat-sim 加载器扫描（#67）：冒险目录有 `combat/` 时只加载它；跳过隐藏目录（`.md/`、`.git`）；目录里的非内容文件改为警告；带 `common/` 的嵌套库只通过 `library:` 加载。
- `start` 摘要每本书只列 `<书>.md` 和 `index.md`。

## v0.1.4 — 2026-10-21

### 新功能
- **XP、升级与战利品**（#63）：`xp add/show`、`level-up`（HP、熟练、生命骰、豁免、技能、命中自动更新，新特性与子职提示）、`loot add|spend|give|split|show`（多币种，个人钱袋与队伍公共池）。
- **SRD 5.1 批量导入与快速遭遇**（#61）：`combat-sim content import-srd`，生成 331 个怪物、302 个法术。
- **2014 PHB 子职导入**（#55、#59）：本地书中识别 28 个非 SRD 子职。
- **pdf2md 0.3.0**（#53）：校对规则、图片原位提取、地图与房间索引；冒险书缺口修复（#57）：OCR 补房间标题、朗读框转引用。
- **持久队伍状态**（#46）：`combat/party-state.yaml`、`combat-sim party show|rest|set-hp|sync`。
- **`talkdnd check` 检定**（#48）、建卡校验（等级、子职、护甲、法术数量、种族特性，#49）。

### 修复
- 规则修正（#44）：非本回合施法/使用只允许反应；短休花生命骰；长休结束专注、恢复一半生命骰；新增 `npc` 类型（同伴会做死亡豁免）。
- 生物死亡时结束自己的专注并移除他人专注造成的状态（#51）。

## v0.1.1 — 2026-10-21

### 新功能
- **talkdnd 游玩命令行**（#37）：`start` 接档摘要、`new`、可续接的 `chargen`（掷骰 6×4d6 去最低、记日志）、`npc random/sheet`、`checkpoint`、`log`、`options`。
- **精简 AGENTS.md 与玩家主导建卡**（#39）：每回合只做一个决定，玩家自己选。
- **建卡生成攻击、职业特性与资源；本地书籍数据导入**（#41）：`data status|import`。
- **pdf2md**（#18 起）：后台 PDF→Markdown，OCR、增量分块、审阅（#29）、标题修正（#22–#27）、审阅软门槛（#35）等。
- **2014 战斗数据**（#33）：SRD 守卫、活化盔甲、拟形怪，五个法术，`support` AI。
- **统一掷骰**（#15）：`combat-sim roll`，DM 掷骰规则；即兴叙事指引（#16）。

### 修复
- 测试偶发失败（#14）、Windows 路径（#30）、审阅召回率对行尾连字符（#31）。

## v0.1.0 — 2026-10-20

### 新功能
- 首个发布（#2、#4、#7、#8、#10）：`LetsPlay/` 游玩文件夹（DM 手册、战斗数据、规则裁定）和预编译 combat-sim；Linux x86_64 / Windows x86_64 / macOS arm64 三个平台包与 sha256；CI（fmt、clippy、三平台测试、cargo-deny、泄露检查、yamllint、链接检查）；项目章程。
- 发布包只含 git 跟踪的 `LetsPlay/` 文件；打包时拒绝 PDF、`dm-secrets.md`、存档和书籍内容。
