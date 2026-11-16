# 成长协作锁域与恢复协议（设计 · 单文件模型）

状态：**仅设计**。本文按所有者选择的**单文件模型**整体重写：`Saves/<slot>/.talkdnd.advancement` 一个文件既是 OS 锁的对象，也是唯一权威操作记录；删除此前的 P/`current.yaml`/独立锁文件双文件模型及其模式分类。不实现锁、不改代码、不迁移存档、不写 sidecar、不改变已批准的 B/D/Q、备份/恢复边界与“预览不写”。代码事实基线 `cc7876077d705598eed7f08ad7b365168efccfdc`；合成测试与故障注入**全部未执行**。依据 [身份契约](ADVANCEMENT-IDENTITY-CONTRACT.md) §3–§6、[恢复选项](ADVANCEMENT-RECOVERY-OPTIONS.md)、[安全审计](ADVANCEMENT-SAFETY-AUDIT.md)、[锁调查](CHECKPOINT-LOCK-INVESTIGATION.md) 与[成长设计](superpowers/specs/2026-11-10-character-advancement-design.md)。只读公开源码、测试与既有文档；未读取真实存档、书籍、DM 秘密。

标记约定：**事实**指能从当前公开代码/测试直接读出的行为；**推断**指由静态事实推出、未实测的后果；**UNKNOWN** 指本次不能判定，不等于安全。现有测试通过**不是**锁安全证据；#264 未解决前，任何“现有锁已够用”的结论都不成立。

## 0. 本文件回答什么、单文件模型改了什么

回答：成长提交会碰哪些写入对象与今天的锁；单文件记录与状态机；显式 `init`/`adopt`；唯一“可开始普通写入”的谓词；唯一四步归档规范；D 流程各阶段的持锁与失败；三平台合成测试矩阵。

**相对上一版（双文件）的变化**：

1. **一个文件**：`Saves/<slot>/.talkdnd.advancement` 同时承载 OS 锁与权威记录；不再有独立的 `current.yaml`、锁文件、`.operations/` 权威记录，也没有“模式 N/I”与“记录缺失但锁存在”的分支。
2. **显式生命周期**：只有 `init` 能创建该文件；`AlreadyExists` 一律失败关闭。`adopt` 只能处理已存在的文件，不能创建。
3. **状态收敛**：`initializing`、`ready`、`active`、`complete`、`failed`、`in_doubt`、`adopted`（`adopted` 只出现在归档证据中，不作为活动文件状态）；没有 `released`。
4. **归档仍是证据，不是第二权威**：归档放在存档外的控制目录，按唯一四步规范写入；读取/判定只认活动文件。

保留：§1 已核实事实盘点；预览只读；内层锁停止接入；已批准的 D 备份/候选/逐文件发布/恢复流程；OD-1/2/7/9；证据矩阵结构。

不承诺：多文件原子性、消除 TOCTOU、约束旧二进制/外部编辑器/不遵守协议的进程、网络文件系统语义、#264 已解决、跨整盘镜像的复制识别、提交路径有界总等待（§2.6）。

## 1. 现有写入对象与现有锁（AC-1）

### 1.1 写入对象盘点

“成长预览/提交”尚未实现（**事实**：`tools/talkdnd/src` 没有等级提升的候选/预览/提交入口，`progress::level_up` 只做既有升级）。下表按“未来成长提交可能触及的对象”盘点**今天的**写入者与锁。

| 对象（存档内相对路径） | 今天的写入定位（代码事实） | 写入方式与顺序 | 现有锁 | 判定 |
|---|---|---|---|---|
| 角色结构化数据 `sheets/<id>.yaml` | `charsheet::store`（`tools/talkdnd/src/charsheet.rs`）；`progress::level_up` 经 `charsheet::commit` 调用 | 整文件 `fsutil::atomic_write`（临时文件 + rename + 尽力目录 sync） | 无保存级锁；仅命令结束后 `audit::after_command` 才取完整性锁 | **事实**：内容写入不在任何互斥内。**推断**：两个写者各自读到旧值后都能写成功，后者覆盖前者。 |
| 角色 Markdown `Characters/*.md` / `NPCs/*.md` | `charsheet::commit` → `atomic_write(markdown_path(...))`；旧式角色 `progress::level_up` 直接 `atomic_write(&s.file, ...)` | `commit` 顺序：YAML → party state → Markdown | 同上 | **事实**：三步非事务，中途失败会留下“YAML 新、Markdown 旧”或反之。 |
| party state `combat/party-state.yaml` | `charsheet::commit` → `PartyState::save`（`tools/combat-sim/src/party.rs`）；战斗命令 `combat-sim/src/cli.rs::run_state_command` → `write_party_state` | 单文件 `atomic_write`，随后 `integrity::after_write` 自行取一次完整性锁 | 完整性锁只覆盖清单读改写，不覆盖 party 内容 | **事实**：party 内容写入与清单更新是两次独立动作；`save` 的 `before` 快照在写前取，写后 `after_write` 才加锁。 |
| 进度/XP `progress.yaml` | `Progress::store`（`tools/talkdnd/src/progress.rs`） | 单文件 `atomic_write` | 无 | **事实**：无锁。 |
| 骰子日志 `rolls.log` | `rolls::append` → `save::append` | **追加**（`OpenOptions::append`，只读末字节决定换行），不重写 | 无 | **事实**：并发追加通常不截断彼此，但同一操作重复执行会重复追加；无法用追加语义判断“是否已应用”。 |
| 会话日志 `Sessions/*.md` | `save::append`（`progress::log`、`checkpoint` 文本、战斗日志） | 追加，同上 | 无 | **事实**：同上；日志是唯一记录“发生过什么”的载体，但可被非协作写者改写。 |
| 完整性清单 `integrity.yaml` + `.talkdnd.integrity.lock` | `combat_sim::integrity::lock` / `store` / `after_write`；`audit::after_command`、`migrate::carry_dirty_flags_guarded`、`checkpoint::restore_rebaseline` 都取该锁 | 清单读改写全程持锁；清单本身 `atomic_write` | **有**：OS advisory 排他锁，阻塞获取，无超时 | **事实**：只序列化清单本身，不保护其它文件。**推断**：它不能阻止两个写者先后写同一角色文件。 |
| checkpoint 快照与恢复 `checkpoints/`、`Saves/_backups/` | `checkpoint::snapshot` / `restore_inner` | 取 `.talkdnd.lock` 后：快照=复制覆盖文件 + 写 `manifest.yaml`；恢复=整档备份 → 暂存 → 逐文件 rename →（失败则镜像回滚）→ 迁移 → 追加日志 → 完整性重定基 | **有**：`.talkdnd.lock`（`create_new` 文件创建式） | **事实**：它不是 OS 锁，见 §1.2。 |
| 迁移备份 `Saves/_backups/<slot>-<stamp>-schema<n>` | `migrate::backup_save` + `migrate::copy_tree` | 名称用 `exists` 递增，非原子分配；递归复制普通文件/目录，跳过 `_backups`、`.cache`、`.talkdnd` | 无 | **事实**：特殊文件被静默跳过；`.talkdnd.integrity.lock` 与 `.talkdnd.lock` **不在**跳过列表（名称不等于 `.talkdnd` 目录），会被复制进备份。**推断**：该备份不是“完整”证明，也不是锁内快照。 |
| 遭遇清单 `combat/campaign.yaml` | `new_save`（`lib.rs`）、`library::sync_campaign_packs`（`atomic_write` 且**不**调用完整性更新）；读取方 `charsheet::party`、`combat-sim` loader | 单文件重写（按行替换 `library:`） | 无 | **事实**：`start` 不在完整性追踪集合内（`lib.rs::run` 对 `Cmd::Start` 取 `tracked=None`），manifest 重写后可无清单更新。 |
| 遭遇状态 `combat/*.state.json` | `EncounterState::save`（`combat-sim/src/state.rs`），`cli::run_state_command` 顺序：load → 执行 → `state.save` → `write_party_state` → 追加战斗日志 | 单文件 `atomic_write` + 清单更新；三步非事务 | 无保存级锁 | **事实**：战斗命令整套流程没有互斥；与 `talkdnd` 进程之间也没有。 |
| 候选树 / 阶段记录 / 操作 ID | **不存在**（未来 D 流程才引入） | — | — | **事实**：今天没有操作 ID、阶段记录或候选目录，因此也没有可被锁保护的候选写入。 |
| `.cache/`、`digest.md` | `start.rs::digest`、`cache_path` | 直接 `atomic_write` | 无 | **事实**：属于可再生成的缓存/派生输出；按 §5 OD-7 作为控制元数据对待，不属于用户数据备份范围。 |

`.pdf2md/worker.lock`（`tools/talkdnd/src/library.rs` 读取、`LetsPlay/tools/pdf2md` 写入）是另一域的锁：它协调 PDF 转 Markdown 的后台任务，既不写存档内容，也不参与成长提交。**事实**：它与本文的锁域无交集；除非未来有流程让 pdf2md 写存档（目前没有），不应把两者混成一把锁。

### 1.2 现有两把锁的机制（逐条事实）

**checkpoint 锁 `.talkdnd.lock`**（`tools/talkdnd/src/checkpoint.rs`）：

1. 获取 = `OpenOptions::create_new(true)` 创建文件，写入本进程 PID（`Lock::acquire`）。**事实**：不持有任何 OS 文件锁。
2. 冲突 = 文件已存在；若 `stale()` 判定（mtime 距今 ≥ `LOCK_STALE_SECS = 600` 秒）则 `remove_file` 后重试一次（`stolen` 只允许抢一次），否则报“另一个 checkpoint 操作正在进行”。
3. 释放 = `Drop` 里 `remove_file(&self.path)`。**事实**：无归属校验、无 inode 校验。
4. 推论（**推断**，未实测）：(a) 进程被 `SIGKILL` 后锁文件留在原处，最长 600 秒内所有 checkpoint 操作被拒，之后任意后来者删除并接管；(b) 若 A 的快照超过 600 秒（大存档复制），B 会删掉 A 的锁并开工，两个写者同时操作同一存档；(c) 若 B 已接管并新建锁文件，A 结束时 `Drop` 会删除 **B 的** 锁文件，第三个写者因此可以进入；(d) `stale` 只看 mtime，不看持有者是否存活，也不看进程 PID——PID 只被写入、从未被读取。**这不是安全基底。**

**完整性锁 `.talkdnd.integrity.lock`**（`tools/combat-sim/src/integrity.rs`）：

1. 获取 = `OpenOptions::create(true).truncate(false).write(true)` 打开固定路径，再 `File::lock()`（Rust 标准库文件锁；Unix 为 `flock`，Windows 为 `LockFileEx`）。**事实**：阻塞等待，无超时；同一进程再次获取会自锁死（源码注释明确要求用 `*_locked` 变体）。
2. 释放 = 句柄 `Drop` 或进程退出，由 OS 释放。**事实**：崩溃不会留下“陈旧锁”；锁文件永不删除、从不参与完整性指纹。
3. 覆盖范围 = 只有 `integrity.yaml` 的读改写。**事实**：`audit::after_command`、`integrity::ensure/changes/after_write`、`migrate::carry_dirty_flags_guarded`、`checkpoint::restore_rebaseline` 在同一把锁下更新清单。

**顺序事实**：`checkpoint.rs` 模块注释声明 checkpoint 锁先、完整性锁后，“never the reverse”；`restore_inner` 全程持 checkpoint 锁，并在其中调用 `migrate::ensure`（内部会取完整性锁）与 `restore_rebaseline`。**事实**：这是当前代码里唯一成文的锁序。除此之外没有全序表，也没有进程内的“已持有集合”检查。

**普通命令的写序列（无保存级锁）**：`lib.rs::run` = 解析 →（多数命令）`integrity::snapshot`（无锁）→ `dispatch`（真正写入：`charsheet::commit`、`Progress::store`、`save::append` …）→ `audit::after_command`（此时才取完整性锁）。**事实**：写与清单更新之间没有互斥；`snapshot`/`dispatch`/`audit` 三段之间，另一个进程可以完整插入。

### 1.3 现有锁文件在备份/回滚中的位置（事实与风险）

- `checkpoint::copy_tree` 只跳过 `.talkdnd.lock`；`integrity::LOCK`（`.talkdnd.integrity.lock`）会被复制进备份/回滚镜像。
- `checkpoint::remove_extras` 同时跳过 `.talkdnd.lock` 与 `integrity::LOCK`（不回删）。
- 镜像回滚用 `copy_tree(&backup, save)` 写回；`std::fs::copy` 对已存在的目标是**截断写同一 inode**，不是替换 inode，但会**改写**该锁文件的字节内容。**推断**：若回滚发生时另一个进程正持有该 inode 的排他锁，OS 锁仍然有效，但基于文件内容的持有者记录会被污染；因此任何协议都应在取得 OS 锁之后才读记录，并且**不以锁记录作为“有无未结束操作”的唯一依据**（见 §2.2 的持久操作记录）。
- `migrate::copy_tree` 不跳过任何锁文件，备份里会出现这些锁文件的副本。**推断**：用它恢复/复制时可能把锁文件带到新位置，`(路径, inode)` 级别的锁身份不能因此被当成同一把锁。

### 1.4 现有测试能证明什么（不能证明锁安全）

| 测试入口（事实） | 实际覆盖 | 为什么不是锁安全证据 |
|---|---|---|
| `tools/talkdnd/tests/checkpoint.rs::a_held_lock_fails_the_second_call_and_a_retry_after_release_works` | 手工写入锁文件后，同一进程的快照/恢复被拒；删除后恢复 | 只验证“文件存在即拒绝”，不验证真实竞争、进程死亡或锁文件被替换 |
| `tools/talkdnd/tests/checkpoint.rs::concurrent_snapshots_exactly_one_wins` | **同进程两个线程**竞争一次快照 | 没有第二个进程；不覆盖 OS 锁、`Drop` 删除他人锁、stale 抢占 |
| `tools/talkdnd/tests/checkpoint.rs::restore_rebaseline_keeps_a_concurrent_writers_fingerprints`、`rollback_keeps_the_integrity_lock_a_concurrent_writer_holds` | 恢复/回滚期间清单锁与另一个“写者”的指纹 | 写者是测试自己构造的句柄/子进程，且 `rollback...` 正是 #264 首次失败点；#264 未解决 |
| `tools/combat-sim/src/integrity.rs` 测试（如清单锁失败不泄漏锁） | 清单读改写的互斥与错误路径 | 只覆盖清单这一把锁，不覆盖角色/party/进度/日志 |

**结论（事实）**：现有测试证明的是若干局部不变量，不是“成长提交在并发下安全”。#264 的首次 `WouldBlock` 根因仍 UNKNOWN，[锁调查](CHECKPOINT-LOCK-INVESTIGATION.md) 的后续通过不推翻这一点。

## 2. 单文件协议（AC-2）

### 2.1 文件、身份与锁

- **唯一文件**：`Saves/<slot>/.talkdnd.advancement`。它同时是 OS 锁的对象与唯一权威操作记录；没有第二个记录文件或独立锁文件。
- **打开方式**：普通操作一律 `OpenOptions::new().read(true).write(true).open(path)`（**不带 `create`**）；`NotFound` 立即失败关闭，不创建、不进入写阶段。只有 `init` 允许一次 `create_new`。
- **锁**：取得该文件的 OS 排他锁（Unix `flock`，Windows `LockFileEx`），用 `try_lock` 在有界 deadline 内轮询；失败即报告持有者不可见/超时，不删除文件、不改名。
- **锁身份**：每次完整写入都把当前文件标识（Unix `dev`+`ino`；Windows 文件索引/ID 或等价物）记入 `file_identity`。读取后先复核“路径 → 当前打开句柄 → 记录中的 `file_identity`”三者一致；不一致即失败关闭（视为被替换或复制，走显式 `adopt` 或人工处理）。它只证明“还是同一个被记录的文件”，**不能**发现旧 inode 的其它持有者，也不保证识别保留身份与内容的整盘镜像（见 §2.4）。
- **同一文件内写入**：不 rename、不替换 inode（rename 会破坏已持有的锁语义）。写入流程：构造完整新内容（`revision` 递增、带 `checksum`）→ 原地截断写 → 文件 `sync_all`。崩溃/短写可能留下空或不完整内容；读取时按解析与 `checksum`/`revision` 判定为**无效**，按 UNKNOWN 拒绝，不做猜测性修复。

### 2.2 记录文件：内容与语义

字段（设计）：`schema`、`protocol_version`、`save_path`（仓库根相对的规范化 POSIX 路径，如 `Saves/2026-10-19-01`）、`save_id`（随机，不取自内容，不是认证）、`state`、`stage`、`operation_id`、`owner_id`、`file_identity`、`revision`、`checksum`、`capabilities`（锁自检结果）、`backup_dir`、`candidate_dir`、`published[]`（路径 + 前像摘要 + 目标摘要 + done）、`pending[]`（已写 intent 未 done）、`last_error`、`prior_archive`（最近一次归档的相对路径 + 摘要）、`adopted_from`（来源、来源 `save_id`（可得时）、证据摘要、人工确认字段与理由）、`created_at`、`updated_at`。

记录是**权威**，不是诊断副本；但也**不是锁**：锁由 OS 对同一文件持有，记录内容不能替代锁，任何基于内容/时间/`pid` 的抢占都不允许。归档文件只是证据，不参与判定（§2.7）。

状态语义：

| 状态 | 含义 |
|---|---|
| `initializing` | 文件已存在但尚未形成有效记录（init 中断、空文件、校验失败）。**不是正常写入态。** |
| `ready` | 初始化或 adopt 完成，当前没有进行中的操作。 |
| `active` | 有操作进行中（`stage` 记录进度）。 |
| `complete` | 操作已完成且通过全部验证。 |
| `failed` | 操作在**尚未开始任何目标写入**前停止。 |
| `in_doubt` | 部分发布或结果不明；只允许显式恢复。 |
| `adopted` | **仅出现在归档证据中**（adopt 完成的历史副本）；活动文件在 adopt 成功后为 `ready`。活动文件若出现 `adopted`，按 UNKNOWN 拒绝。 |

没有 `released`。

### 2.3 显式 `init`

前置：`lstat` 显示 `.talkdnd.advancement` **不存在**。存在（含 0 字节）→ `AlreadyExists`，一律失败关闭（不接管、不覆盖、不“修复”）。

1. `create_new` 创建文件。
2. **立刻**用不带 `create` 的 `open` 取得 OS 锁（有界 deadline）。
3. **锁能力自检**：用第二个句柄对同一文件 `try_lock`，必须被拒；否则失败关闭（平台/文件系统不支持跨句柄互斥）。
4. 写入完整记录：`save_id`（新随机）、`schema`/`protocol_version`、`save_path`、`state=ready`、`file_identity`、`revision=1`、`checksum`、`capabilities`、时间；文件 `sync_all`。
5. 复核路径→句柄→记录身份一致；释放 OS 锁。`init` 结束。

失败与崩溃：

- 步骤 1 之后、步骤 4 完成之前失败/崩溃 → 文件可能为空或内容不完整，逻辑状态为 `initializing`。普通读取/提交一律拒绝，只允许 `init --resume` 或 `init --abort`。
- 步骤 4 的文件 `sync_all` 失败/结果不明 → 不得报告成功；保持失败关闭，下一次按实际内容判定（完整可解析则可能已是 `ready`，否则 `initializing`）。不得断言“一定还是 initializing”。
- `init` 不得删除文件来“当作没发生”。

`init --resume`：文件存在；不带 `create` 打开并取 OS 锁（被他人持有 → 失败关闭）；校验内容。若为有效 `ready` → 报告“已初始化”，不重写；若为 `initializing` 且 `save_id`/`save_path` 可校验、无任何目标写入证据 → 补齐为 `ready`（保留原 `save_id`，更新 `file_identity`/`revision`/`checksum`）并同步；若文件为 0 字节（create 后立即崩溃）→ 可按首次初始化写入新 `save_id` 并标 `init-resume` 来源，但仍须确认没有用户写入证据；非空但不可解析或校验失败（可能隐藏写入）→ 拒绝 resume，要求人工；`init --abort` 也只在可判定为 `initializing` 时允许。

`init --abort`：文件必须存在；**先取得 OS 锁**——无法取得（有活持有者）→ 失败关闭，不删除。只有状态为 `initializing` 且没有用户写入证据时允许：按四步归档当前字节（含空文件，记摘要与状态）→ 人工确认（`--yes` + 理由）→ 删除文件。`ready`/`active`/`complete`/`failed`/`in_doubt` 一律拒绝 abort；非空但不可解析且无法排除写入 → 拒绝，交人工。abort 是本模型**唯一**删除该文件的动作，且必须留下归档证据。

不存在“记录已建、锁未建”的窗口：文件与锁是同一个对象，`create_new` 成功后才可能持有锁；失败只留下 `initializing` 文件。

### 2.4 显式 `adopt`

用途：存档被复制或移动后，在新位置确认/重建协作身份。**没有自动识别**；未 adopt 的复制在可观察证据不符时失败关闭，但内部证据完全相同的整体镜像无法保证检出（见末尾）。

前置：目标 `.talkdnd.advancement` **必须存在**（副本随 slot 一起复制）；不存在 → 拒绝，要求先 `init`（`adopt` 不能创建）。人工确认目标“是副本或已知移动、且没有活着的协作写者”，并记录确认字段（OD-10）。

1. 不带 `create` 打开文件；取 OS 锁。取不到（有活持有者）→ 失败关闭，不做任何改动。
2. 读取旧内容（可为有效记录或不可解析），复核路径与 `file_identity`；如内容有效则校验 `schema`/字段。
3. 保护当前树：按 D 备份规则做完整备份；特殊文件/权限/空间不足 → 停止。
4. **归档旧内容**：按唯一的四步归档规范（§2.7）把当前文件字节写入 `Saves/_backups/.operations/<slot>/history/<operation_id>-pre-adopt.yaml`，校验摘要与目录同步。
5. 建立身份并写回活动文件：
   - `copy`：生成**新 `save_id`**；`adopted_from` 记来源（可得时）、来源 `save_id`（可得时）、证据摘要、人工确认字段与理由。来源不可得时必须显式 `source_unknown` + 理由，不得静默重置。
   - `relocate`：保留原 `save_id`；`adopted_from` 记移动事实与确认字段。
   - 写入 `state=ready`，更新 `file_identity`（当前文件）、`revision`、`checksum`，同步。
6. 按四步归档一份 adopt 证据副本（内容取自新的 `ready` 记录、标注 `state=adopted` 与 `adopted_from`）；失败 → 不报告成功，保持失败关闭并保留旧归档与活动文件。
7. 释放 OS 锁。

失败与崩溃：任一步中断 → 文件保留当时内容；下一次按状态表判定（`ready` 可能已写、也可能仍是旧内容；旧内容为 `active`/`complete` 等则按原状态处理）。不要求 `adopt --resume` 自动续跑；需要就在锁内重新执行 adopt（重新备份/归档，旧归档保留）。若旧内容为未结束操作，adopt 拒绝，转入检查/恢复。

**识别边界（如实说明）**：正常复制会得到不同的文件标识，而记录中的 `file_identity` 指向源文件 → 可观察不符，失败关闭并指向 adopt。**但**整盘/快照级复制可能同时保留文件内容与文件标识，使比较误判为一致；这种情形无法保证检出，仍**必须显式 adopt**，本文不承诺一律识别。未初始化的旧存档被复制后没有任何记录可比对，也无法与“首次 init”区分——这是 UNKNOWN，只能靠操作者纪律与显式 adopt。

### 2.5 唯一“可开始普通写入”谓词与状态表

普通操作开始前**逐项**校验（`init --resume/--abort` 不走本节，按 §2.3 自身规则），任一失败即拒绝（一般记 UNKNOWN，身份不符另记 `IDENTITY_MISMATCH`），不进入状态表：

1. 文件存在，且以不带 `create` 的 `open` 成功；
2. 已取得该文件的 OS 排他锁；
3. 内容可解析、`checksum`/`revision` 一致、`schema`/`protocol_version` 已知、必填字段齐全；
4. `save_path` 与当前路径的同一表示一致；`save_id` 合法；
5. `file_identity` 与当前打开句柄一致；
6. 归档/证据目录可枚举（需要检查归档时读取失败 → UNKNOWN）。

**谓词**：只有下列之一且上述校验全过，才允许开始普通写入——① 前态 `ready`；② 前态 `complete` 且其归档存在、摘要一致；③ 前态 `failed` 且**已证明零目标写入**（`published[]` 空、`pending[]` 空、无本次日志/完整性写入）且归档存在、摘要一致。其余一律拒绝。

| 优先级 | 状态 | 额外条件 | 判定 | 允许的动作 |
|---|---|---|---|---|
| 1 | `initializing` | — | 初始化未完成 | 只允许 `init --resume` / `init --abort` |
| 2 | `adopted`（活动文件） | — | 不应出现在活动文件 | 失败关闭（UNKNOWN）；按归档证据人工检查 |
| 3 | `in_doubt` | — | 部分发布或结果不明 | 只允许显式恢复与人工检查 |
| 4 | `active` | — | 上次操作未正常结束（能取得锁说明原持有者已退出，但仍按未结束处理） | 拒绝普通写入；显式恢复 |
| 5 | `failed` | `published[]` 空、`pending[]` 空、无本次日志/完整性写入，归档一致 | 写入前停止 | **允许**新操作（先归档；报告上一操作） |
| 6 | `failed` | 任一已发布、未决 intent 或日志/完整性结果不明 | 与 `failed` 语义矛盾 | 失败关闭；按 `in_doubt` 处理（恢复） |
| 7 | `complete` | 候选比对、日志/完整性完成、无 `pending`、归档一致 | 已完成 | **允许**新操作（先归档） |
| 8 | `ready` | 无进行中操作 | 干净 | **允许**新操作 |
| 9 | `complete`/`failed` | 归档缺失或摘要不符 | 终态缺归档 | 失败关闭；只允许显式 archive 补齐后重判 |
| 10 | 其它组合 | — | UNKNOWN | 失败关闭 |

`complete` 只在全部候选比对、日志/完整性完成且无未决 intent 后写；任何部分发布或结果不明 → `in_doubt`；`failed` 只用于尚未开始目标写入的停止。这是写入终态时的强制语义。

### 2.6 等待预算与内层锁（停止接入）

- 持有不是租约：不用 TTL 判定死亡，不删文件、不改名、不接管。
- 获取有 deadline（默认 5 秒，可显式覆盖）；等待方失败，不无限阻塞。
- 总预算 = `D_save` + 各阶段内层锁 `D_inner` 之和；每次内层获取都必须有 deadline 与取消路径。
- **事实**：完整性锁 `File::lock()` 阻塞且无超时；checkpoint 锁是 `create_new`、不等待。因此当前任何在 SAVE 内获取完整性锁的路径（`audit::after_command`、`PartyState::save`、`migrate::ensure`、`integrity::*`）都无法给出有界总等待。
- **接入停止条件**：内层锁获得有界获取/取消能力之前，含完整性锁的提交路径**不得接入**；不得把 `D_save` 或并发测试结果外推为整条提交路径有界（可选做法见 OD-9）。
- 心跳/时间仅用于诊断；不存在“强制解锁”命令：只有 `init --abort` 能删除记录（§2.3），且必须留证。

### 2.7 唯一四步归档规范

所有归档（`complete` 终态、恢复前快照、adopt 旧内容与 adopt 证据）共用同一规范：

1. **独占创建**：`create_new` 在目标目录创建文件（已存在 → 走同名规则）。
2. **写 + 文件同步**：写入完整内容，`sync_all`。
3. **重读校验**：重新读取，校验字节与摘要一致。
4. **父目录同步**：对归档所在父目录执行同步并确认成功。

- 四步全过，才允许活动记录引用该归档（`prior_archive` 等）。任一步失败/结果不明 → 保留原文件与证据，停止，不覆盖、不删除；失败产物隔离为 `.partial-<rand>`。
- **同名规则**：已存在时必须重读完整内容 + 校验摘要 + 对**既有文件**执行 `sync_all` + 同步父目录，全部成功才可复用；任何不符/读不全/同步失败 → 人工处理。
- 归档位置：`Saves/_backups/.operations/<slot>/history/`（存档外的控制元数据；恢复不会移动它）。归档不含日志正文、角色内容或秘密。

### 2.8 预览必须只读

- 不打开、不创建、不锁 `.talkdnd.advancement`；不写探针、不写 `.cache`、不写任何记录；只做只读发现与读取。
- `preview_id` = 对（完整枚举清单 + 每文件摘要 + 规则集 + 授权状态 + 遭遇状态 + 身份映射 + 计划变更）的摘要，由调用方保存；预览不持久化。
- 输出 `consistency: unverified`；读取期间可能有并发写者，不声称快照。
- 提交必须取锁、重新发现并重算 binding，与 `preview_id` 比较；不一致拒绝并要求重新预览。只给单一摘要时，提交**不得**列具体差异类别，只能报“绑定变化，原因未知”；提供可校验分项清单时才可列类别。未提供 `preview_id` → 拒绝。
- 文件缺失/只读目录/无法探测：预览仍可跑（它不需要锁），但必须报 `write_capability: unverified`，不承诺可提交。

### 2.9 崩溃、替换、平台原语与共存

- 崩溃/被杀：OS 释放锁；记录停在 `active`（或 `initializing`）。下一次取锁后：`active` → 拒绝普通写入，显式恢复；`initializing` → 只允许 resume/abort。
- 文件被删除/替换：持有者在每个阶段边界复核“路径→句柄→`file_identity`”；不一致立即中止，不再写目标。另一进程可能对新的 inode 取锁并读到旧/复制内容；若记录身份与当前不符或状态未结束 → 失败关闭，不得当作干净。路径/句柄复核不能发现旧 inode 的其它持有者，不宣称能做到。
- 从不 rename/替换该文件；只有 `init --abort` 在显式条件下删除（§2.3）。
- 平台：Linux/macOS `flock`（advisory，按打开文件描述）；Windows `LockFileEx`（字节区锁，共享/删除语义待实测）。网络盘/同步目录/FUSE **UNKNOWN**。`init` 用“同一文件第二句柄 `try_lock` 必须被拒”做能力自检（不创建探针文件）；失败关闭。
- 共存：总序 `SAVE → checkpoint .talkdnd.lock → integrity .talkdnd.integrity.lock`；反向获取算协议违规。checkpoint/完整性锁语义不变。必须先修的旧行为：checkpoint `Drop` 无条件删除锁文件、mtime 抢占、备份/回滚复制锁文件、`migrate::backup_save` 静默跳过特殊文件——修掉前新锁只约束遵守新协议的写者。
- 不受约束：旧版本二进制、外部编辑器、脚本、同步客户端、杀毒软件、备份工具、其它机器客户端、PDF 转换进程、手工复制。当前 `combat-sim` 也尚未接入（写遭遇/party 不取 SAVE）；接入前把遭遇写入当并发风险，活动遭遇只预览。
- 残余 TOCTOU 无法消除：每文件 rename 前复核仍有窗口；只能缩小、记录复核值、失败时说明“可能已覆盖某次外部写入”。#264 未解决期间同样适用；本设计不解释、不修复首次 `WouldBlock`。

## 3. 锁在已批准 D 流程中的位置（AC-3）

D 流程（完整备份 → 候选构建 → 逐文件发布 → 日志/完整性 → 完成/恢复）与恢复边界沿用[恢复选项](ADVANCEMENT-RECOVERY-OPTIONS.md) §3 与[身份契约](ADVANCEMENT-IDENTITY-CONTRACT.md) §3.5，不因单文件模型改变。普通操作只使用 `active` → `complete`/`failed`/`in_doubt`；`initializing` 不属于 D 流程。

| 阶段 | 持锁与状态 | 动作（设计） | 失败停止 | 可重试边界 |
|---|---|---|---|---|
| `admit` 受理 | 全程持锁；`ready`/`complete`/`failed(零写入)` → 归档旧记录 → 写 `state=active, stage=admit` | 前置校验（§2.5）；生成 `operation_id`；按四步归档旧记录；更新活动文件 | 校验失败/归档失败 → 不写目标；保持原状态 | 整次重新开始（新 `operation_id`） |
| `backup` 完整备份 | 持锁，`active` | 存档外独占创建备份目录；复制全部用户数据（含既有用户备份；控制元数据排除见 OD-7）；逐文件路径/类型/大小/摘要；校验副本；同步文件与目录；记录 `backup_verified` | 空间不足/特殊文件/权限/同步失败 → 停止；保留不完整备份 | 同操作内新目录重做（旧目录保留） |
| `candidate` 候选构建 | 持锁，`active` | 同一文件系统/卷建暂存树（跨设备拒绝）；按已验证前像生成候选；校验 schema/身份/关系与每文件目标摘要 | 空间不足/校验失败/检测到外部变化 → 停止；保留备份与候选 | 重建候选；不得把不完整候选标为已验证 |
| `publish` 逐文件发布 | 持锁，`active` | 固定顺序（`sheets/*.yaml` → `combat/party-state.yaml` → `Characters/*.md` → `progress.yaml` → 追加日志 → 完整性清单）；每文件：写 intent（`pending`）→ 复核当前=前像 → 同目录临时文件+rename → 同步文件与父目录 → 标 done | intent 写失败 → 不发布该文件；复核不符/写失败/同步失败 → **立即停止**；已发布/未发布写回记录 | 不自动重试发布；恢复按“当前=目标→已应用；当前=前像→可应用；都不是→停止”逐文件判断 |
| `logs` 日志与完整性 | 持锁，`active`（完整性清单另取内层锁） | 追加日志按操作 ID 去重；不截断、不改写既有行；尾部异常停止；清单更新保留既有 dirty | 追加失败/清单失败 → 停止；置 `in_doubt` 并报告“日志可能已追加、清单可能未更新” | 日志仅在能证明未追加时重试；清单可重算但不洗掉 dirty |
| `complete` 完成 | 持锁，`active` → 终态 | 重新发现并比对候选清单：无意外新增/删除/变化、`published[]` 与候选一致、`pending[]` 空、日志/完整性完成；写 `state=complete` 并按四步归档终态 | 比对不符/未决 intent/结果不明 → `in_doubt`，拒绝普通写入 | 校验可重复；清理不属于成功证明 |
| `recovery` 显式恢复 | 持锁（自身一个 `operation_id`） | 覆盖活动文件前按四步归档其当前内容（`history/<old_op_id>-pre-recovery.yaml`）；保护恢复时当前树；校验备份完整性；确认目标身份；逐文件比较当前与前像/目标；由人显式选择“回滚到前像”或“继续到目标”；逐文件执行并同步 | 归档/目录同步失败、既非前像也非目标、日志已有后续追加、身份不明、备份校验失败 → 停止；保留两份证据；不删除后续日志 | 不自动重试；再次中断重新进入 `in_doubt`，由人重新选择，不自动续跑/重放 |

阶段报告固定字段：操作 ID、状态、阶段、锁状态、记录文件位置、备份/候选位置、已发布/未发布清单、结论（未写/部分写/已写）、需要的人工动作、不得自动执行的动作。

**明确不承诺**：多文件原子性；崩溃后自动恢复；恢复保留未在备份中的后续内容；锁能阻止非协作写者；目录同步/掉电语义（`fsutil::sync_parent` 在 Unix 忽略目录打开与同步错误、非 Unix 不执行；`atomic_write` 成功不证明目录项已持久化）。**单文件模型的额外限制**：记录是原地重写、不能 rename；短写/崩溃可能留下无效内容，只能检测并失败关闭，不能保证记录始终可解析。

**幂等**：属性值、XP、资源、专长选择等“只加一次”的效果由记录与每目标标记保证恰好一次；日志去重只能用操作 ID 标记。校验、摘要、复制到新目录可重试；发布、日志追加、恢复动作在不能证明状态前不可重试。

## 4. 合成测试矩阵（AC-4）

全部 **NOT_RUN**。下表描述未来在合成 fixture（临时存档目录，无真实数据）上执行的测试；“CI”指三平台 GitHub Actions 可跑，“人工/特权”指需要多主机、网络文件系统、特权或物理操作。断言是期望，不是当前结果。

| ID | 场景与注入 | Linux 期望 | Windows 期望 | macOS 期望 | 环境 | 判定要点 |
|---|---|---|---|---|---|---|
| T1 | 两个协作写者同时提交 | 一方取锁进入 `active`，另一方 deadline 失败且未写目标 | 同左（`LockFileEx`） | 同左（`flock`） | CI | 失败方不写目标；不出现第二个 `active` |
| T2a | 持锁期间文件被外部删除/替换 | 阶段边界检出“路径→句柄→`file_identity`”变化，立即中止 | 同左（删除是否被拒记录实际值） | 同左 | CI（子进程注入） | 不再写目标；不宣称阻止替换 |
| T2b | 外部替换被 OS 拒绝 | 无状态变化；记录拒绝与错误 | 记录实际观察值 | 同左 | CI | 不要求不可观察的尝试 |
| T2c | A 持旧 inode，路径被新文件替换，B 锁新文件 | B 读到旧/复制内容且身份不符 → 失败关闭；不当作干净 | 同左 | 同左 | CI（两进程） | 路径/句柄复核不能发现 A |
| T2d | 两进程都看到文件不存在，竞争 `init` | `create_new` 唯一赢家；输家失败关闭、未创建文件 | 同左 | 同左 | CI（两进程） | 唯一赢家；无第二个文件 |
| T2e | `lstat` 与 `open` 之间文件被删（普通操作） | `NotFound` → 失败关闭；不创建文件、不进入写阶段 | 同左 | 同左 | CI（删除时序） | 正常操作永不建文件 |
| T3 | 活持有者超时（等待方 deadline 到期） | 等待方失败，不删文件、不接管 | 同左 | 同左 | CI | 不出现第二个持有者 |
| T4a | 崩溃/被杀（每阶段边界 `SIGKILL`/`TerminateProcess`） | OS 释放锁；记录 `active` → `in_doubt`，普通写入拒绝 | 同左 | 同左 | CI | 不自动续跑 |
| T4b | 活动文件原地写入短写/`sync_all` 失败 | 内容无效或结果不明；报告失败；不声称成功；下一次按校验拒绝 | 同左 | 同左 | 需实现后故障注入 | 单文件无 rename 兜底 |
| T5a | `init` 在 `create_new` 后、写内容前崩溃 | 空文件 → `initializing`；普通操作拒绝 | 同左 | 同左 | CI（分阶段 kill） | 不删除文件伪称未发生 |
| T5b | `init --resume` 补齐 `initializing` | 校验后保留原 `save_id`，写 `ready` | 同左 | 同左 | CI | resume 不换身份 |
| T5c | `init --resume` 遇到有效 `ready` | 报告已初始化，不重写 | 同左 | 同左 | CI | 幂等 |
| T5d | `init --abort`：有活持有者；状态为 `ready`/`active`/`complete`/`failed`/`in_doubt` | 取锁失败或状态不符 → 拒绝，不删除 | 同左 | 同左 | CI | 只有 `initializing` 可 abort |
| T5e | `init --abort` 于 `initializing`：归档后删除；归档失败 | 四步归档成功 + 人工确认后才删除；归档失败保留文件 | 同左 | 同左 | CI（故障注入） | 唯一删除动作且留证 |
| T5f | 文件不存在（普通操作/`adopt`） | 普通操作与 adopt 均失败关闭；adopt 要求先 `init` | 同左 | 同左 | CI | adopt 不创建 |
| T5g | 文件被复制（身份不符） | 校验失败 → 失败关闭 → 提示显式 adopt | 同左 | 同左 | CI | 不自动认领 |
| T5h | 非空但不可解析/校验失败 | UNKNOWN 拒绝；不自动修复、不 abort | 同左 | 同左 | CI（构造内容） | 可能隐藏写入 |
| T6a | 顺序反转（checkpoint 快照/恢复 vs 成长提交） | 按总序串行，一方等待或失败，**不死锁**；测试级超时 | 同左 | 同左 | CI（两进程+超时） | 挂起即失败 |
| T6b | 内层完整性锁被另一进程长期持有 | 提交必须在预算内失败并报告内层锁；或该路径标记“未接入” | 同左 | 同左 | CI（超时） | 不把 `D_save` 当整体有界 |
| T7a | 非协作编辑，注入器已知值（预览后/发布前/intent 后） | 在定义检查点检出并拒绝；产品未覆盖注入字节 | 同左 | 同左 | 需实现后故障注入 | 注入值仅测试侧 |
| T7b | 非协作编辑发生在最后复核后、rename 前 | 只能报“无法排除覆盖”，列出已观察前像与目标；不写“实际值” | 同左 | 同左 | 需实现后故障注入 | 不宣称检测成功 |
| T8 | 预览过期（绑定变化） | 提交拒绝；单一 `preview_id` 时只报“绑定变化，原因未知” | 同左 | 同左 | CI | 无分项清单不得列类别 |
| T9a | 恢复前归档：写入/文件 sync/重读摘要/父目录 sync 失败 | `.partial` 隔离；原文件不动；停止 | 同左 | 同左 | 需实现后故障注入 | 四步未全过不得覆盖 |
| T9b | 恢复中断（归档后、写回前后 kill） | 重新进入 `in_doubt`，人工选择；不自动续跑/重放 | 同左 | 同左 | CI（分阶段 kill） | 不重复应用 |
| T9c | 归档父目录同步失败 | 归档不可引用；停止；下次重读+目录同步复核，仍失败人工 | 同左 | 同左 | 需实现后故障注入 | 目录同步失败＝不可用 |
| T9d | 归档写完、文件 `sync_all` 前崩溃；复用归档时既有文件 sync 失败 | 不可复用；按同名规则复核，失败人工 | 同左 | 同左 | CI（kill）/故障注入 | 不靠“文件存在”推断已归档 |
| T10 | 释放语义（持有期间文件被替换后释放） | 释放不删除替换后的文件；路径仍存在 | 同左（删除失败单独断言） | 同左 | CI | 不拆被持有的 inode |
| T11 | `init` 锁能力自检失败（同一文件第二句柄也能取锁、网络盘） | 失败关闭；报告平台/文件系统 | 同左 | 同左 | 人工/特权（网络挂载、只读卷） | 不降级、不建探针 |
| T12 | 跨主机/网络文件系统互斥 | **UNKNOWN**：本地自检不证明跨客户端互斥 | **UNKNOWN**（SMB/OneDrive） | **UNKNOWN**（NFS/云同步） | 人工/特权 | 未证明前按“不支持” |
| T13a | `adopt --mode copy`（文件为副本） | 取锁→读旧内容→备份→四步归档旧内容→新 `save_id`+`adopted_from`→写 `ready`→归档 adopt 证据 | 同左 | 同左 | CI | 不创建文件；来源证据可定位 |
| T13b | `adopt --mode relocate`（移动后的文件） | 同上但保留原 `save_id`；`file_identity` 更新为当前文件 | 同左 | 同左 | CI | 不换身份 |
| T13c | 文件不存在时 `adopt` | 拒绝；要求先 `init` | 同左 | 同左 | CI | adopt 不创建 |
| T13d | adopt 期间存在活持有者（取锁失败/超时） | 失败关闭；不修改内容、不做备份之外的写 | 同左 | 同左 | CI（超时） | 活持有者优先 |
| T13e | adopt 时旧内容为 `active`/`in_doubt` | 拒绝 adopt，转入检查/恢复 | 同左 | 同左 | CI（构造内容） | 不覆盖未结束操作 |
| T13f | adopt 各步中断（取锁后/备份后/归档后/写回前后） | 文件保留当时内容；不需要自动续跑；重新 adopt 需重新备份/归档 | 同左 | 同左 | CI（分阶段 kill） | 不产生“已 adopt”假象 |
| T13g | 整盘/快照级复制（内容与文件标识都被保留） | 可能无法检出；**不得**宣称识别；必须显式 adopt | 同左 | 同左 | 人工/特权 | 记录 UNKNOWN 限制 |
| T13h | 未初始化的旧存档被复制（无记录） | 无记录可比对；与首次 `init` 不可区分；`adopt` 拒绝、`init` 可作为引导 | 同左 | 同左 | CI | 明示 UNKNOWN |
| T14 | 终态语义：完整验证 → `complete`；部分发布 → `in_doubt`；不存在 `released`；活动文件出现 `adopted`；伪造 `complete` | 只允许验证后 `complete`；部分发布只能 `in_doubt` 且拒绝普通写入；`adopted`/伪造终态失败关闭 | 同左 | 同左 | CI（构造状态） | 状态名不替代前置条件 |

**只能由实现后的故障注入证明的结论**：T2a/T2c/T2d/T2e/T4b/T5a/T5e/T7a/T9/T9a/T9c/T9d/T13f/T14 的阶段边界中止、前置校验、初始化唯一赢家、正常操作不建文件、原地记录的短写/校验、归档窗口与恢复不重复应用、adopt 各步。`T7b` 只能证明“如实报告不可排除”（不输出注入值）；T11/T12/T13g 需人工/特权；T13g/T13h 只能证明“不宣称识别”，不能证明检出。**T6b 在接入前按“路径未接入”记录，不得当作通过。**

**不进入 CI 的**：掉电/设备谎报同步、磁盘控制器缓存、文件系统修复、杀毒软件拦截、容器/虚拟机快照恢复、真实玩家存档与非协作文本编辑器；全部保持 UNKNOWN。

## 5. 边界、决策与后续（AC-5）

### 5.1 本文件没有改变的东西

- 仅新增本文件；无代码、测试、配置、依赖、CI、既有设计/章程、规则数据、MANIFEST、存档或秘密变更。
- 单文件模型**不改变**已批准的 B/D/Q、备份/恢复边界、“预览不写”、遭遇“活动只预览”、旧程序不受约束、schema 11 目标。
- 不新增锁实现，不产生“已安全”的结论；#264 保持未解决风险。

### 5.2 需要项目所有者决定（`NEEDS_HUMAN_DECISION`）

| ID | 问题 | 选项与影响 | 建议 |
|---|---|---|---|
| OD-1 | 支持的平台/文件系统边界 | (a) 只承诺本地常见文件系统，网络盘/同步目录一律拒绝；(b) 依赖自检并允许网络盘“尽力”；(c) 每平台逐项验证后再承诺 | (a)：可判定、失败关闭；代价是云同步目录不能提交 |
| OD-2 | **在覆盖所有相关新版本写者的前提下**，确认入口清单与接入顺序 | (a) 单次发布全部写入入口；(b) 分批接入，未接入入口在同一存档上拒绝写入或声明只读；(c) 先只读诊断/预览，再按 (b)。**“只覆盖成长提交”不是选项**：缩小范围须重新授权 | (a) 或 (b)+(c) |
| OD-7 | 备份范围与保留 | **完整备份必须包含既有用户备份**；控制元数据（`.talkdnd.advancement` 的归档副本、暂存索引）的排除、保留期与清理责任可另议。排除既有用户备份须重新授权 | 包含用户备份；只排除控制元数据；先确认清单 |
| OD-9 | 完整性锁（内层锁）的有界化策略 | (a) 给现有锁加 deadline/取消（改变阻塞语义）；(b) 清单更新移出关键段（改变失败语义）；(c) 保持无限阻塞，则提交路径不承诺有界等待、T6b 按“未接入” | 先 (a) 或 (c)；未决前按 §2.6 停止接入 |
| OD-10 | `adopt` 的**人工确认字段与授权人**（不再讨论是否实现 adopt） | 必填字段候选：来源 slot/路径、来源 `save_id`（可得时）、人工确认人、确认时间、理由、证据摘要；`source_unknown` 的理由与最低证据；授权人（所有者/指定 DM）；是否要求二次确认 | 全部必填 + 所有者或指定人授权；`source_unknown` 也须同一授权；未定前 adopt 不实现 |

**随单文件模型不再作为选项的旧条目**（列出以免误解）：双文件记录/独立锁文件、自动初始化与模式 N/I、`released`、自动复制识别、`adopting` 状态（改为 adopt 自身完成或在锁内重做）、独立探针文件（改为同一文件第二句柄自检）。

### 5.3 可由工程决定的事

记录字段名与 schema、`D_save` 与轮询退避、`checksum`/`revision` 算法、心跳/时间写入点、阶段名与文案、确定性发布顺序、归档命名、`.partial` 命名与保留、进程内“已持有集合”的检查方式、测试夹具与故障注入点、诊断输出格式（不含敏感数据）。

### 5.4 建议后续任务顺序（依赖顺序，不是排期）

1. **单文件原语与状态机骨架**（独立任务）：实现打开/锁/身份复核、`init`/`resume`/`abort`、四步归档、§2.5 判定；**不写目标文件**；跑 T1/T2a–T2e/T3/T4b/T5a–T5h/T10/T14。前置：本设计经审查。
2. **旧锁行为修正**（独立任务，需行为变更审查）：checkpoint `Drop` 归属校验、取消 mtime 抢占、备份/回滚排除控制文件、`backup_save` 对特殊文件报错；不声称修复 #264。
3. **接入所有新版本写者**（依赖 1、2 与 OD-2）：跑 T6a；不改变业务语义。
4. **D 阶段机与备份验证**（依赖 3）：`admit`/`backup`/`candidate`，只到“候选已验证、未发布”；跑 T4a 发布前分支与 OD-7 确认的备份范围。
5. **发布、日志/完整性与幂等**（依赖 4 与 OD-9）：跑 T2a/T7a/T7b/T8；T6b 未接入前按“未接入”记录。
6. **显式恢复**（依赖 5）：跑 T9/T9a–T9d；恢复前四步归档、保护当前树、人工选择。
7. **`adopt`**（依赖 1 与 OD-10；未决前不实现、副本不可提交）：跑 T13a–T13h。
8. **#264 与跨平台/网络文件系统调查**（可与 1–7 并行，不能替代）：按[锁调查](CHECKPOINT-LOCK-INVESTIGATION.md) 的现场采集要求做，禁止公开私有路径。
9. **sidecar 入籍与 9/10→11 迁移**（依赖 3–6 与 Q 的具体设计）：按[身份契约](ADVANCEMENT-IDENTITY-CONTRACT.md) §5 的顺序，不得以本设计代替身份验证。

首阶段全部验收后暂停，不进入全量专长自动化。

## 6. 证据与限制

九字段验收证据矩阵发布在 Issue #277 对应的 PR 评论，`commit_sha` 绑定该 PR 的最终 head；本文件内不复制可能随 head 变化的运行结果。代码事实基线 `cc78760`；若源码变化，§1 的定位需重新核对。

已知限制：只做静态阅读与设计；所有合成测试、故障注入、平台实验均 NOT_RUN；未读取真实存档、书籍或秘密；未验证 Windows/macOS 文件锁、网络文件系统、目录/掉电持久性；预览一致性为 `unverified`；`init`/`adopt`/四步归档/状态机均未实现（`adopt` 在 OD-10 决定前不得实现、副本不可提交）；单文件原地写入不能保证记录始终可解析，短写/崩溃只能检测并失败关闭；`file_identity` 不能识别保留身份与内容的整盘镜像；未初始化的副本与首次 init 不可区分；内层锁仍无界（§2.6、OD-9）；#264 根因仍 UNKNOWN；现有测试通过不构成锁安全证明。

回退：本文件为普通文档，可整体 revert，不触及存档、schema 或任何运行行为。
