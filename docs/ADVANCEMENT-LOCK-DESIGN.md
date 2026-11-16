# 成长协作锁域与恢复协议（设计 · 简化协议）

状态：**仅设计**。本文按所有者选择的**简化协议**重写：用显式生命周期命令取代自动初始化竞争、自动复制识别与 `released` 的“等价完成”语义。不实现锁、不改代码、不迁移存档、不写 sidecar、不改变已批准的 B/D/Q、备份/恢复边界与“预览不写”。代码事实基线 `cc7876077d705598eed7f08ad7b365168efccfdc`；合成测试与故障注入**全部未执行**。依据 [身份契约](ADVANCEMENT-IDENTITY-CONTRACT.md) §3–§6、[恢复选项](ADVANCEMENT-RECOVERY-OPTIONS.md)、[安全审计](ADVANCEMENT-SAFETY-AUDIT.md)、[锁调查](CHECKPOINT-LOCK-INVESTIGATION.md) 与[成长设计](superpowers/specs/2026-11-10-character-advancement-design.md)。只读公开源码、测试与既有文档；未读取真实存档、书籍、DM 秘密。

标记约定：**事实**指能从当前公开代码/测试直接读出的行为；**推断**指由静态事实推出、未实测的后果；**UNKNOWN** 指本次不能判定，不等于安全。现有测试通过**不是**锁安全证据；#264 未解决前，任何“现有锁已够用”的结论都不成立。

## 0. 本文件回答什么、简化掉了什么

回答：成长提交会碰哪些写入对象、今天受哪些锁保护、按什么顺序写；简化协议的状态、记录与生命周期命令是什么；`init`/`adopt` 各步及其失败面；唯一“可开始普通写入”的谓词；D 流程各阶段何时持锁、失败怎么停、哪些动作可重试；三平台合成测试矩阵。

**简化删除的复杂度**（相对早先草案）：

1. **自动初始化竞争**：不再让“首次读取/提交”在无记录时竞争创建；改用显式 `init`，先写权威记录再建锁，失败只允许 `init --resume`/`--abort`。
2. **自动复制识别**：不再靠 `save_path`、inode 或“看起来是副本”推断；复制/移动存档必须显式 `adopt`，未 adopt 的副本一律失败关闭、不可提交。
3. **`released` 等价完成**：状态机删除 `released`；“之前没开始写”用 `failed` 表达，“全部验证过”用 `complete` 表达，部分发布或结果不明一律 `in_doubt`。
4. **多入口状态判定**：状态表只按状态排序一次；`initializing`/`adopting` 不参与普通判定，只允许各自的显式续作/中止。

保留：已核实的 §1 事实盘点、预览只读、内层锁停止接入、已批准的 D 备份/候选/逐文件发布/恢复边界、OD-1/2/7/9、证据矩阵结构。

不承诺/不回答：多文件原子性、消除 TOCTOU、约束旧二进制/外部编辑器/不遵守协议的进程、跨网络文件系统语义、#264 已解决、实现排期、改变任何既有安全承诺；提交路径不承诺有界总等待（原因见 §2.7）。

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

## 2. 简化协议（AC-2）

### 2.1 生命周期命令、状态与不变量

协议只有三个显式生命周期命令，普通读取/预览/提交**从不**创建 P 或锁文件：

| 命令 | 作用 | 前置 |
|---|---|---|
| `init` | 为一个尚无协议的存档建立记录与锁 | P 与锁文件**都不存在** |
| `init --resume` / `init --abort` | 完成或放弃一个 `initializing` 记录 | P 存在且 `state=initializing` |
| `adopt` / `adopt --resume` / `adopt --abort` | 为复制/移动的存档建立或转换身份 | 人工确认来源与无活写者；见 §2.4 |

状态（P 的 `state`）：`initializing`、`adopting`、`ready`、`active`、`complete`、`failed`、`in_doubt`、`adopted`。`adopted` 是 adopt 操作的终态；对“可开始普通写入”的判定，它与 `complete` 等价（§2.5）。**没有 `released`。**

| 不变式 | 内容 |
|---|---|
| I1 | 同一存档同一时刻最多一个遵守协议的持有者进入 `active`。 |
| I2 | 锁文件存在后**永不删除、永不 rename 覆盖**；只在同一 inode 上原地改写诊断记录（`abort` 是唯一例外，见 §2.3）。 |
| I3 | 锁文件路径指向的 inode 在持有期间不得改变；检测到改变即中止。 |
| I4 | 持有者存活只由 **OS 锁**判定；记录里的时间/pid/心跳不能作为抢占依据。 |
| I5 | 任何前置校验失败一律失败关闭：不降级、不猜、不“修复后继续”。 |
| I6 | 锁只覆盖一个存档；不同存档可并行。 |
| I7 | 既有内层锁（完整性、checkpoint）语义不变；本协议不取代它们。 |
| I8 | 失败必须留下可定位的（操作 ID、状态、阶段、备份、已写/未写）证据；不得删除唯一可恢复副本。 |
| I9 | 只有 §2.5 的谓词成立时才能开始普通写入；`initializing`/`adopting`/`active`/`in_doubt` 一律拒绝。 |
| I10 | 预览不写存档、不取锁、不建 P、不写探针（§2.6）。 |

### 2.2 记录、锁身份与位置

- **锁路径**：`Saves/<slot>/.talkdnd.advancement.lock`。
- **持久操作记录 P（权威）**：`Saves/_backups/.operations/<slot>/current.yaml`，位于存档之外（存档恢复不移动它），`temp + rename` 原子替换；终态与恢复前快照归档到同目录 `history/`（`create_new` 只写一次，不改写）。
- P 字段（设计）：`schema`、`protocol_version`、`save_path`（**仓库根相对的规范化 POSIX 路径**，如 `Saves/2026-10-19-01`）、`save_id`（随机，不取自内容，不是认证）、`owner_id`、`operation_id`、`mode`（`init`/`adopt`/`advance-commit`/`recovery`/`checkpoint`/`migrate`）、`state`、`stage`、`lock_identity`（锁文件标识：Unix `dev`+`ino`，Windows 文件索引/ID 或等价可复核标识）、`capabilities`（`init`/`adopt` 时的平台与锁能力探测结果）、`backup_dir`、`candidate_dir`、`published[]`（路径 + 前像摘要 + 目标摘要 + done）、`pending[]`（已写 intent、未标 done）、`last_error`、`prior_record`（归档相对路径 + 摘要）、`adopted_from`（来源、来源 `save_id`（可得时）、证据摘要、确认字段与理由）、`created_at`、`updated_at`。
- P 是控制元数据，不含日志正文、角色内容或秘密。临时名 `*.tmp-<rand>`；失败/写一半的归档隔离为 `*.partial-<rand>`。
- **锁身份**：`init`/`adopt` 创建（或接管既有）锁文件后写入 `lock_identity`；普通操作开始前必须存在并与当前打开句柄一致，否则 `LOCK_REPLACED` 失败关闭。它只证明“还是同一个被记录的锁文件”，**不能**发现旧 inode 的其他持有者，也不证明跨安装复制（§2.4）。
- **记录是诊断，不是锁**：先拿 OS 锁，再**只读**读 P；判定通过后才写。

### 2.3 `init`（显式初始化）

前置：目标 `Saves/<slot>` 目录存在且可读（是否为新存档由操作者确认；程序不据此猜测历史）；`lstat` 显示 P 不存在且锁文件不存在。任一存在 → 拒绝（`init` 不接管、不覆盖；已有 `initializing` 走 `--resume/--abort`，已有正常记录走普通流程或 `adopt`）。

成功路径：

1. 能力探测：在存档目录创建一次性探针 `.talkdnd.probe.<rand>`，取锁并用第二个句柄确认被拒，删除探针；失败/无法删除 → `init` 失败并报告残留路径，**不创建 P/锁**。结果写入 `capabilities`。
2. **先写权威记录**：`create_new` 创建 `current.yaml`，`state=initializing`，写入 `schema`/`protocol_version`/`save_path`/新 `save_id`/`owner_id`/`operation_id`/`created_at`；`sync_all` 文件。`create_new` 失败（已被并发 init 创建）→ **立即停止，不创建锁、不继续**；按现状提示 `init --resume`/`--abort` 或普通流程。
3. **再建锁**：`create_new` 创建锁文件；用不带 `create` 的 `open` 取 OS 锁（有界 deadline）；把当前文件标识写入 P 的 `lock_identity`，并把 `state` 改为 `ready`（原子替换 + 文件 `sync_all` + `operations/` 目录同步）。
4. 原地写锁记录（`owner_id`、`operation_id`、`state=ready`、时间），读回比对；释放 OS 锁。`init` 结束。

失败与崩溃：

- 任一步失败/崩溃都留下 `state=initializing`（可能无锁文件，可能有锁但未写 `lock_identity`）。**普通读取/提交一律拒绝**，错误指向 `init --resume` 或 `init --abort`。
- `init` 不得删除已创建的 P、锁或探针来“当作没发生”；残留路径必须在错误中给出。
- 若 P 已改为 `ready` 但锁记录写失败：下一次看到 `state=ready`、`lock_identity` 已写、锁空闲 → 按 §2.5 正常判定（`ready` 允许开始操作）；锁记录只是诊断，不影响该判定，但必须报告上次写失败。
- 写入 `lock_identity`/`ready` 的 P 替换若 rename 成功但目录 `sync` 失败：**结果不明**。本次 `init` 必须报告失败且不得在同一会话继续写入；下一次可能看到 `ready`（按 §2.5 判定）或 `initializing`（只允许 resume/abort）。不得断言“P 一定还是 initializing”。

`init --resume`：要求 P 存在且 `state=initializing`；操作者显式执行。若锁文件缺失则可 `create_new` 创建（这是 init 的唯一建锁延续）；已存在则只以既有文件 `open` 取锁，绝不替换 inode。取得 OS 锁后重新校验 P（schema、`save_path`、`save_id`），写 `lock_identity` 与 `state=ready`、同步、释放。另一进程持有锁 → 失败；P 不是 `initializing` → 拒绝。resume 不生成新 `save_id`。

`init --abort`：要求人工确认（`--yes` 且记录理由）。先把 P 与锁文件字节归档到 `Saves/_backups/.operations/<slot>/aborted/<operation_id>.yaml`（写 + `sync_all` + 摘要 + 目录同步），**成功后**才允许删除锁文件与 `current.yaml`。任一步失败 → 停止并保留现状（仍失败关闭）。abort 是 I2 的唯一删除例外，且必须记录：这是放弃协议记录的显式动作，不代表存档内容未被写过。

### 2.4 `adopt`（显式复制/移动）

用途：存档被复制或移动后，在新位置建立/转换协作身份。**没有自动识别**：`save_path` 相同、inode 相同或内容摘要相同都**不能**证明是副本；未执行 `adopt` 的复制存档在普通流程一律失败关闭、不可提交。

前置（人工）：操作者必须确认目标“是副本或已知移动、且没有活着的协作写者”，并提供可记录字段（见 OD-10）。来源或写者状态不可证明 → 拒绝或标 UNKNOWN，不是“尽力 adopt”。

两种模式：

- **`--mode copy`**：目标 P **必须不存在**。用 `create_new` 创建 `state=adopting` 的 P（新 `save_id`、`save_path`、`owner_id`、`operation_id`）作为**唯一仲裁**；`AlreadyExists` → 拒绝。仲裁成功后才继续（**仲裁先于备份**，避免两个并发 adopt 各自备份后互相覆盖记录）。目标锁：存在则只以既有文件 `open` 取锁；不存在则 `create_new` 一次。取锁失败/超时 → 保留 `adopting` 记录，停止。
- **`--mode relocate`**：目标 P **必须存在**且 `save_path` 不匹配（移动后的正常形态）。**不得** `create_new` 排除；先只读校验旧 P（schema/字段/`save_id`），再取**既有**锁。锁缺失 → 失败关闭（无法证明没有写者；不新建锁）。取锁后若 P 为未结束状态 → 不 adopt，转入检查/恢复。

仲裁成功后的步骤（仍持锁）：

1. 能力探测（同 §2.3）：探针 + 双句柄确认 + 删除；结果写入 P 的 `capabilities`；失败 → 停止（保留 `adopting`）。
2. 保护当前树：按 D 备份规则给目标存档做完整备份；特殊文件/权限/空间不足 → 停止。
3. 保留证据：把目标锁文件字节、可指定来源 slot 的 P/history（**只读**读取，不修改来源）复制到 `Saves/_backups/.operations/<target_slot>/evidence/`，记录来源、摘要、时间与“可能不完整”。
4. 建立身份：
   - `copy`：`adopted_from` 记来源（若可得）、来源 `save_id`（若可得）、证据摘要、人工确认字段与理由；来源 P 不可得时**必须**显式写 `source_unknown` 与理由，不得静默重置。
   - `relocate`：保留原 `save_id`；`adopted_from` 记移动事实、人工确认字段与理由；先按归档四步归档旧 P（`history/<old_op_id>-pre-adopt.yaml`）。
5. 写新 P 为 `state=adopted`（`lock_identity` 指向当前锁）。
6. **释放前**按归档四步把该 `adopted` 终态归档到 `history/<operation_id>.yaml`；归档失败 → 不显示成功，保持失败关闭并报告。

失败与崩溃：`adopting` 记录**保留**（不删除、不改写成“无副作用”），普通写入拒绝；只允许 `adopt --resume`（校验并继续）或 `adopt --abort`（按 §2.3 同样的归档+人工确认清理）。adopt 的任何中断都不得动用户数据之外的判定捷径；已备份/留证的可复用，未完成的部分在 resume 中重新校验。

不承诺：adopt 不判断人物/历史关系、不检测整体重放旧备份，只建立独立协作身份。跨安装根复制若路径恰好相同，**路径不能作为证据**；只能靠 `lock_identity` 与 P 记录不一致触发失败关闭，再走授权 adopt 或人工隔离。文件系统快照/镜像恢复可能保留 inode 编号，使比较误判为一致——保持 UNKNOWN，不承诺检测。

### 2.5 唯一“可开始普通写入”谓词与状态表

普通获取一律用 `OpenOptions::new().read(true).write(true).open(path)`（**不带 `create`**）打开锁文件；`NotFound` 立即失败关闭，不新建文件、不进入写阶段。

**前置校验**（在任何状态判定之前；任一失败 → 拒绝：一般记 UNKNOWN，锁身份不符另记 `LOCK_REPLACED`，均不进入下表）：完整枚举（存档目录、`.operations/<slot>/`、`history/`、临时与 `.partial` 列表可读）；P 可解析；`schema`/`protocol_version` 已知；必填字段齐全；`save_id` 合法；`save_path` 与当前路径的同一表示一致；锁文件存在；`lock_identity` 已写且与当前打开句柄一致（`initializing`/`adopting` 例外，它们不进本表）。

**谓词**：只有下列之一且前置校验全过，才允许开始普通写入——① 状态 `ready`（初始化完成、尚无操作）；② `complete` 且归档一致；③ `adopted` 且 adopt 身份链与归档一致；④ `failed` 且**已证明零目标写入**（`published[]` 空、`pending[]` 空、无本次日志/完整性写入，且归档一致）。其余一律拒绝。

| 状态 | 额外条件 | 判定 | 允许的动作 |
|---|---|---|---|
| `initializing` | —（不进本表） | 初始化未完成 | 只允许 `init --resume` / `init --abort` |
| `adopting` | —（不进本表） | adopt 未完成 | 只允许 `adopt --resume` / `adopt --abort` |
| `in_doubt` | — | 部分发布或结果不明 | 只允许显式恢复与人工检查 |
| `active` | — | 上次操作未正常结束（能取得锁说明原持有者已退出，但仍按未结束处理） | 拒绝普通写入；显式恢复 |
| `failed` | `published[]` 空、`pending[]` 空、无本次日志/完整性写入 | 写入前停止 | **允许**新操作（先归档；报告上一操作） |
| `failed` | 任一已发布、未决 intent 或日志/完整性结果不明 | 与 `failed` 语义矛盾 | 失败关闭；按 `in_doubt` 处理（恢复） |
| `complete` | 候选比对、日志/完整性完成、无 `pending`、归档一致 | 已完成 | **允许**新操作（先归档） |
| `adopted` | adopt 证据与身份链完整、归档一致 | 已 adopt、尚无操作 | **允许**新操作（先归档） |
| `ready` | 初始化后无操作记录 | 干净 | **允许**新操作 |
| 终态（`complete`/`failed`/`adopted`） | 归档缺失或摘要不符 | 终态缺归档 | 失败关闭；只允许显式 `archive` 补齐后重判（`failed` 无目标写入时同理） |
| 其它组合 | — | UNKNOWN | 失败关闭 |

`complete` 只在全部候选比对、日志/完整性完成且无未决 intent 后写；任何部分发布或结果不明 → `in_doubt`；`failed` 只用于尚未开始目标写入的停止。这些是写入终态时的**强制**语义，不是命名约定。

### 2.6 预览必须只读

已批准边界是“预览不写”。预览：

- 不取锁、不打开/创建锁文件、不建 P、不写探针、不写 `.cache`；只做只读发现与读取。需要缓存时只能写到调用方指定的**存档外**路径，并标注“非证据、可能过期”。
- 绑定：`preview_id` = 对（完整枚举清单 + 每文件摘要 + 规则集 + 授权状态 + 遭遇状态 + 身份映射 + 计划变更）的摘要，由调用方保存；预览不持久化绑定。
- 输出 `consistency: unverified`；不声称快照一致。
- 提交必须取锁、重新发现并重算 binding，与传入的 `preview_id` 比较；不一致拒绝并要求重新预览。只提供单一摘要时，提交**不得**声称列出具体差异类别，只能报“绑定变化，原因未知”；只有调用方同时提供可校验分项清单时才允许列类别。未提供 `preview_id` → 拒绝。
- 锁缺失/只读目录/能力未探测：预览照常可跑，但必须报 `write_capability: unverified`，不承诺可提交；可提交性只在提交时由前置校验与 §2.5 决定。
- 若未来要求预览把绑定写进存档，那是改变“预览不写”的已批准决定，须所有者重新授权；本文不这样做。

### 2.7 等待预算与内层锁（停止接入）

- 持有不是租约：不用 TTL 判定死亡，不因超时删锁、改名或接管。
- 获取有 deadline（默认 5 秒，可显式覆盖）；等待方失败，不无限阻塞。
- 总预算 = `D_save` + 各阶段内层锁 `D_inner` 之和；实现必须为每次内层获取指定 deadline 与取消路径。
- **事实**：完整性锁 `File::lock()` 是阻塞、无超时；checkpoint 锁是 `create_new`、不等待。因此当前任何在 SAVE 内获取完整性锁的路径（`audit::after_command`、`PartyState::save`、`migrate::ensure`、`integrity::*`）**都无法给出有界总等待**。
- **接入停止条件**：在内层锁获得有界获取/取消能力之前，含完整性锁的提交路径**不得接入**；不得把 `D_save` 或并发测试结果外推为“整条提交路径有界”。可选做法（需所有者选择，见 OD-9）：给内层锁加 deadline/取消；把清单更新移出关键段；或保持无限阻塞并明确不承诺有界等待（则该路径不接入）。
- 心跳仅用于诊断；人工不得“强制解锁”：只有 `init`/`adopt` 的 abort 能清理记录（§2.3/§2.4）。

### 2.8 崩溃、锁替换与平台原语

- 崩溃/被杀：OS 释放锁；P 留在 `active`（或 `initializing`/`adopting`）。下一次获取后按 §2.5：`active` → 拒绝普通写入，显式恢复；`initializing`/`adopting` → 只允许各自 resume/abort。
- 锁文件被删除/替换（A 持旧 inode、B 新建文件）：**权威是 P**。B 取得新文件的锁后仍按 §2.5 判定；未结束一律拒绝。路径→句柄复核只能发现 B 自己锁的路径被换，**不能**发现 A 仍持有旧 inode；不得声称能发现旧持有者。
- 释放：写终态、归档、释放 OS 锁；**永不** unlink 锁文件（`init --abort`/`adopt --abort` 除外），路径已被替换时只记录、不改动替换后的文件。
- 平台原语（**事实**：现有代码已用 `std::fs::File::lock`/`try_lock`）：Linux `flock`（advisory，按打开文件描述）；macOS `flock`；Windows `LockFileEx`（字节区锁；共享模式/删除语义待实测）。网络盘/同步目录/FUSE **UNKNOWN**。
- 能力探测只在 `init`/`adopt` 执行（§2.3），结果记录在 P；普通命令不创建探针。平台不支持 → `init`/`adopt` 失败关闭；已有记录的存档在探测记录缺失或不支持时也失败关闭。

### 2.9 与现有锁共存、锁域之外的写者

- `SAVE` 是最外层；总序 `SAVE → checkpoint .talkdnd.lock → integrity .talkdnd.integrity.lock`；反向获取算协议违规（进程内“已持有集合”直接报错）。checkpoint 与完整性锁语义不变。
- 子进程不继承锁，也不允许“父进程持锁、子进程写”；跨进程工作必须释放后重新获取。
- 必须先修掉的旧行为（后续实现任务）：checkpoint `Drop` 无条件删除锁文件；`stale()` 按 mtime 抢占；备份/回滚复制锁文件；`migrate::backup_save` 静默跳过特殊文件。修掉之前，新锁只约束遵守新协议的写者。
- 不受约束：旧版本二进制、外部编辑器、脚本、同步客户端、杀毒软件、备份工具、其它机器客户端、PDF 转换进程、手工复制。
- 当前 `combat-sim` 也不在锁域内（写遭遇/party 不取 `SAVE`）。接入前，提交把“可能的遭遇写入”当并发风险处理（活动遭遇只预览、检出变化即停）。
- 残余 TOCTOU 无法消除：每文件 rename 前复核仍有窗口；只能缩小、记录复核值、失败时说明“可能已覆盖某次外部写入”。#264 未解决期间同样适用；本设计不解释、不修复首次 `WouldBlock`。

## 3. 锁在已批准 D 流程中的位置（AC-3）

D 流程（完整备份 → 候选构建 → 逐文件发布 → 日志/完整性 → 完成/恢复）与恢复边界沿用[恢复选项](ADVANCEMENT-RECOVERY-OPTIONS.md) §3 与[身份契约](ADVANCEMENT-IDENTITY-CONTRACT.md) §3.5，不因简化而改变。差别只在状态命名：普通操作只使用 `active` → `complete`/`failed`/`in_doubt`；`initializing`/`adopting` 不属于 D 流程。

| 阶段 | 持锁与状态 | 动作（设计） | 失败停止 | 可重试边界 |
|---|---|---|---|---|
| `admit` 受理 | 全程持 `SAVE`；`ready`/`complete`/`adopted`/`failed(零写入)` → 归档旧记录 → 写 `state=active, stage=admit` | 前置校验（§2.5）；生成 `operation_id`；归档旧 P（四步）；写 P 与锁记录 | 前置校验失败/归档失败 → 不写目标；保持原状态；报告操作 ID/阶段 | 整次重新开始（新 `operation_id`） |
| `backup` 完整备份 | 持锁，`active` | 存档外独占创建备份目录；复制全部用户数据（含既有用户备份；控制元数据排除见 OD-7）；逐文件路径/类型/大小/摘要；校验副本；同步文件与目录；P 记 `backup_verified` | 空间不足/特殊文件/权限/同步失败 → 停止；保留不完整备份；不写目标 | 同操作内用新目录重做（旧的不完整目录保留） |
| `candidate` 候选构建 | 持锁，`active` | 同一文件系统/卷建暂存树（跨设备拒绝）；按已验证前像生成候选；校验 schema/身份/关系与每文件目标摘要；P 记候选清单 | 空间不足/校验失败/候选期间检测到外部变化 → 停止；保留备份与候选 | 重建候选；**不得**把不完整候选标为已验证 |
| `publish` 逐文件发布 | 持锁，`active` | 固定顺序（`sheets/*.yaml` → `combat/party-state.yaml` → `Characters/*.md` → `progress.yaml` → 追加日志 → 完整性清单）；每文件：P 写 intent（`pending`）→ 复核当前=前像 → 同目录临时文件+rename → 同步文件与父目录 → P 标 done | intent 写失败 → 不发布该文件；复核不符/写失败/同步失败 → **立即停止**；已发布/未发布清单写回 P | 不自动重试发布；恢复按“当前=目标→已应用；当前=前像→可应用；都不是→停止”逐文件判断 |
| `logs` 日志与完整性 | 持锁，`active`（完整性清单另取内层锁） | 追加日志按操作 ID 去重；不截断、不改写既有行；尾部异常停止；清单更新保留既有 dirty | 追加失败/清单失败 → 停止；P 置 `in_doubt` 并报告“日志可能已追加、清单可能未更新” | 日志仅在能证明未追加时重试；清单可重算但不洗掉 dirty |
| `complete` 完成 | 持锁，`active` → 终态 | 重新发现并比对候选清单：无意外新增/删除/变化、`published[]` 与候选一致、`pending[]` 空、日志/完整性完成；写 `state=complete` 并按四步归档终态 | 比对不符/未决 intent/结果不明 → `in_doubt`，拒绝普通写入 | 校验可重复；清理不属于成功证明 |
| `recovery` 显式恢复 | 持锁（自身一个 `operation_id`） | 覆盖 `current.yaml` 前先按四步归档旧 P（`history/<old_op_id>-pre-recovery.yaml`）；保护恢复时当前树；校验备份完整性；确认目标身份；逐文件比较当前与前像/目标；由人显式选择“回滚到前像”或“继续到目标”；逐文件执行并同步 | 归档/目录同步失败、既非前像也非目标、日志已有后续追加、身份不明、备份校验失败 → 停止；保留两份证据；不删除后续日志 | 不自动重试；再次中断重新进入 `in_doubt`，由人重新选择，不自动续跑/重放 |

阶段报告固定字段：操作 ID、状态、阶段、锁状态、P 位置、备份/候选位置、已发布/未发布清单、结论（未写/部分写/已写）、需要的人工动作、不得自动执行的动作。

**明确不承诺**：多文件原子性；崩溃后自动恢复；恢复保留未在备份中的后续内容；锁能阻止非协作写者；目录同步/掉电语义（`fsutil::sync_parent` 在 Unix 忽略目录打开与同步错误、非 Unix 不执行；`atomic_write` 成功不证明目录项已持久化）。

**幂等**：属性值、XP、资源、专长选择等“只加一次”的效果由 P 与每目标标记保证恰好一次；日志去重只能用操作 ID 标记。校验、摘要、复制到新目录可重试；发布、日志追加、恢复动作在不能证明状态前不可重试。

## 4. 合成测试矩阵（AC-4）

全部 **NOT_RUN**。下表描述未来在合成 fixture（临时存档目录，无真实数据）上执行的测试；“CI”指三平台 GitHub Actions 可跑，“人工/特权”指需要多主机、网络文件系统、特权或物理操作。断言是期望，不是当前结果。

| ID | 场景与注入 | Linux 期望 | Windows 期望 | macOS 期望 | 环境 | 判定要点 |
|---|---|---|---|---|---|---|
| T1 | 两个协作写者同时提交 | 一方取锁进入 `active`，另一方 deadline 失败且未写目标 | 同左（`LockFileEx`） | 同左（`flock`） | CI | 失败方输出含持有者信息；目标字节不变 |
| T2a | 持锁期间锁文件被外部成功删除/替换 | 下一阶段边界检出标识变化，立即中止 | 同左（若 OS 允许删除） | 同左 | CI（子进程注入） | 不宣称阻止替换 |
| T2b | 外部替换被 OS 拒绝 | 无状态变化；记录“拒绝”与错误 | 记录实际观察值（可能允许） | 同左 | CI | 不要求不可观察的尝试 |
| T2c | A 持旧 inode，路径被删，B 新建空锁并取锁 | B 读 P 为 `active` → 拒绝普通写入；报告“旧持有者可能仍在” | 同左 | 同左 | CI | 不以空/新建锁为干净；不说路径复核能发现 A |
| T2d | 两进程都看到无 P/无锁，竞争显式 `init` | P `create_new` 唯一赢家建立 `initializing` 并最终 `ready`；输家不创建 P/锁、失败关闭 | 同左；断言输家未创建锁文件 | 同左 | CI（两进程） | 初始化仲裁在 P，不在锁 |
| T2e | `lstat` 与 `open` 之间锁路径被删（普通写入） | `open` `NotFound` → 失败关闭；不新建锁、不进入写阶段 | 同左 | 同左 | CI（删除时序） | 正常获取永不建锁 |
| T2f | 持锁期间外部删除锁路径，另一进程建新 inode 并取锁 | 旧持有者中止；新进程按 P 拒绝；两个 inode 可能同时存在，如实报告 | 记录 OS 是否允许删除 | 同左 | CI（外部进程） | 不得声称发现旧 inode 持有者 |
| T3 | 活持有者超时（等待方 deadline 到期） | 等待方失败，不删锁、不接管 | 同左 | 同左 | CI | 不出现第二个 `active` |
| T4a | 崩溃/被杀（每阶段边界 `SIGKILL`/`TerminateProcess`） | OS 释放锁；P `active` → `in_doubt`，普通写入拒绝 | 同左 | 同左 | CI | 不自动续跑 |
| T4b1 | P temp 写入/文件 `fsync` 失败（rename 前） | 旧 P 完整；temp 保留；不写目标 | 同左 | 同左 | 需实现后故障注入 | 不笼统断言“旧 P 仍在”之外的效果 |
| T4b2 | P rename 成功、目录 `sync` 失败 | 结果不明；停止且不发布依赖该记录的写入；下次按实际所见判定 | 同左 | 同左 | 需实现后故障注入 | 报告 `record_sync_unverified` |
| T4b3 | 锁记录写失败，且 P 的 `failed`/`in_doubt` 补偿也失败 | 记录保留原样；不写目标；下次按实际状态拒绝 | 同左 | 同左 | 需实现后故障注入 | 不假设补偿成功 |
| T4b4a | 归档 `create_new` 短写 / 文件 `sync_all` 失败 | 隔离 `.partial`；原 `current` 不动；停止 | 同左 | 同左 | 需实现后故障注入 | 四步未全过不得引用归档 |
| T4b4b | 同名归档完整、摘要一致、既有文件 `sync_all` 与目录同步成功 | 允许复用并报告 | 同左 | 同左 | 需实现后故障注入 | 复用也必须对既有文件 `sync_all` |
| T4b4c | 同名归档内容/摘要不符、读不全、既有文件 `sync_all` 失败或目录同步失败 | 停止并要求人工；不覆盖、不删除 | 同左 | 同左 | 需实现后故障注入 | 不把“同名存在”当已归档 |
| T5a | `init` 在 P 创建后、建锁前崩溃 | 留下 `initializing`；普通写入拒绝；`init --resume` 可完成 | 同左 | 同左 | CI（分阶段 kill） | 不删除 P 伪称未发生 |
| T5b | `init` 在锁创建后、写 `lock_identity`/`ready` 前崩溃 | `initializing` + 锁；resume 完成并写身份；普通写入在此之前拒绝 | 同左 | 同左 | CI | 身份未写前不得普通写 |
| T5c | `init --resume` 遇到非 `initializing` 记录，或另一进程持锁 | 拒绝；不改记录 | 同左 | 同左 | CI | resume 不重置 `save_id` |
| T5d | `init --abort`：归档证据后清理；清理中途失败 | 证据完整归档后才清理；部分清理失败 → 仍失败关闭并报告 | 同左 | 同左 | CI（故障注入） | abort 是唯一删锁例外，必须留证 |
| T5e | P 缺失 + 锁存在（如复制/旧残留） | 普通写入失败关闭；`init` 拒绝（锁存在）；提示 adopt/人工 | 同左 | 同左 | CI | 不自动重建记录 |
| T5f | P 存在 + 锁缺失 | 普通写入失败关闭；`initializing` 才允许 resume，否则人工 | 同左 | 同左 | CI | 正常获取不建锁 |
| T5g | P 不可解析、未知 schema、缺字段、`save_id`/`save_path` 不符、`lock_identity` 不符 | 一律 UNKNOWN/`LOCK_REPLACED` 失败关闭 | 同左 | 同左 | CI（构造记录） | 不进入状态表、不猜 |
| T6a | 顺序反转（checkpoint 快照/恢复 vs 成长提交） | 按总序串行，一方等待或失败，**不死锁**；测试级超时 | 同左 | 同左 | CI（两进程+超时） | 挂起即失败 |
| T6b | 内层完整性锁被另一进程长期持有 | 提交必须在预算内失败并报告内层锁；或该路径标记“未接入” | 同左 | 同左 | CI（超时） | 不把 `D_save` 当整体有界 |
| T7a | 非协作编辑，注入器已知值（预览后/发布前/intent 后） | 在定义检查点检出并拒绝；产品未覆盖注入字节 | 同左 | 同左 | 需实现后故障注入 | 注入值仅测试侧 |
| T7b | 非协作编辑发生在最后复核后、rename 前 | 产品只能报“无法排除覆盖”，列出已观察前像与目标；不写“实际值” | 同左 | 同左 | 需实现后故障注入 | 不宣称检测成功 |
| T8 | 预览过期（绑定变化） | 提交拒绝；单一 `preview_id` 时只报“绑定变化，原因未知” | 同左 | 同左 | CI | 无分项清单不得列差异类别 |
| T9a | 恢复前归档旧 P：写入/文件 sync/重读摘要失败 | `.partial` 隔离；原 `current` 不动；停止 | 同左 | 同左 | 需实现后故障注入 | 四步未全过不得覆盖 |
| T9b | 恢复中断（归档后、新 P 替换前后 kill） | 重新进入 `in_doubt`，人工选择；不自动续跑/重放 | 同左 | 同左 | CI（分阶段 kill） | 不重复应用 |
| T9c | 归档文件 `sync_all` 成功、`history/` 目录同步失败 | 新 P 不得引用；停止；下次重读+同步目录，仍失败则人工 | 同左 | 同左 | 需实现后故障注入 | 目录同步失败＝归档不可用 |
| T9d | 归档写完后、文件 `sync_all` 前崩溃；复用归档时既有文件 `sync_all` 失败 | 陈旧/不可复用；下次按同名规则复核，失败则人工 | 同左 | 同左 | CI（kill）/故障注入 | 不用“文件存在”推断已归档 |
| T10 | 释放语义（持有期间锁文件被替换后释放） | 释放不删替换后的文件；路径仍存在 | 同左（删除失败单独断言） | 同左 | CI | 不拆被持有的 inode |
| T11 | `init`/`adopt` 能力探测失败（只读目录、探针无法创建/删除、网络挂载） | 失败关闭；报告残留探针 | 同左 | 同左 | 人工/特权（网络挂载、只读卷） | 不降级 |
| T12 | 跨主机/网络文件系统互斥 | **UNKNOWN**：本地探针不证明跨客户端互斥 | **UNKNOWN**（SMB/OneDrive） | **UNKNOWN**（NFS/云同步） | 人工/特权 | 未证明前按“不支持” |
| T13a | `adopt --mode copy`（目标无 P，锁为副本/缺失） | 先 P `create_new` 仲裁，再取/建锁；备份、留证；新 `save_id` + `adopted_from`；来源不变 | 同左 | 同左 | CI | 仲裁在备份前；来源证据可定位 |
| T13b | `adopt --mode relocate`（P 存在，路径不符） | 不 `create_new` 排除；只读校验旧 P；取既有锁（缺失→失败关闭）；保留 `save_id`；归档旧 P 后写 `adopted` | 同左 | 同左 | CI | 不新建缺失锁 |
| T13c | 缺来源 P | 只允许显式 `source_unknown` + 人工确认；否则拒绝；不静默重置 | 同左 | 同左 | CI | 记录理由与确认字段 |
| T13d | 并发 adopt（同目标） | 只有 P `create_new` 成功者继续；输家不建锁/不备份 | 同左 | 同左 | CI（两进程） | 唯一赢家 |
| T13e | adopt 期间存在活写者 | 取锁失败/超时 → 停止；**不修改用户数据、不覆盖已有记录**；保留 `adopting` 记录供显式恢复 | 同左 | 同左 | CI（超时） | 不删除记录伪称无副作用 |
| T13f | adopt 在 `adopting` 创建后崩溃、取锁超时、各步骤中断 | 记录保留；普通写入拒绝；只允许 resume/abort；用户数据未改 | 同左 | 同左 | CI（分阶段 kill） | 仲裁成功≠adopt 完成 |
| T13g | adopt 成功、释放前归档 `adopted` 终态 | 四步归档成功 → 下次判定允许新操作；失败 → 不显示成功，失败关闭 | 同左 | 同左 | CI（归档故障） | 释放前必须可验证 |
| T13h | 复制 slot 但未 adopt（目标有锁、无 P） | 普通写入失败关闭、不可提交；`init` 拒绝；提示 adopt | 同左 | 同左 | CI | 不自动认领 |
| T13i | 跨安装根复制、相对路径相同 | 路径不作证据；锁身份/P 不符或缺失 → 失败关闭；只能经授权 adopt 或人工隔离 | 同左 | 同左 | 人工/特权 | 不按路径当 clean |
| T14 | 终态语义：完整验证 → `complete`；部分发布 → `in_doubt`；不存在 `released`；伪造 `complete`/`adopted` | 仅完整验证允许 `complete`；部分发布只能 `in_doubt` 且普通写入拒绝；伪造终态失败关闭 | 同左 | 同左 | CI（构造状态） | 状态名不替代前置条件 |

**只能由实现后的故障注入证明的结论**：T2a/T2c/T2d/T2e/T2f/T4b1–T4b3/T4b4a–T4b4c/T7a/T9/T9a/T9c/T9d/T13e/T13f/T13g/T14 的阶段边界中止、前置校验、初始化唯一赢家、正常获取不建锁、记录/归档各窗口（含既有归档 `sync_all` 失败）、恢复不重复应用、adopt 仲裁与终态归档。`T7b` 只能证明“如实报告不可排除”（不输出注入值）；T11/T12/T13i 需人工/特权；T13c/T13d 依赖人工确认或进程时序。**T6b 在接入前按“路径未接入”记录，不得当作通过。**

**不进入 CI 的**：掉电/设备谎报同步、磁盘控制器缓存、文件系统修复、杀毒软件拦截、容器/虚拟机快照恢复、真实玩家存档与非协作文本编辑器；全部保持 UNKNOWN。

## 5. 边界、决策与后续（AC-5）

### 5.1 本文件没有改变的东西

- 仅新增本文件；无代码、测试、配置、依赖、CI、既有设计/章程、规则数据、MANIFEST、存档或秘密变更。
- 简化**不改变**已批准的 B/D/Q、备份/恢复边界、“预览不写”、遭遇“活动只预览”、旧程序不受约束、schema 11 目标。
- 不新增锁实现，不产生“已安全”的结论；#264 保持未解决风险。

### 5.2 需要项目所有者决定（`NEEDS_HUMAN_DECISION`）

| ID | 问题 | 选项与影响 | 建议 |
|---|---|---|---|
| OD-1 | 支持的平台/文件系统边界 | (a) 只承诺本地常见文件系统，网络盘/同步目录一律拒绝；(b) 依赖探测并允许网络盘“尽力”；(c) 每平台逐项验证后再承诺 | (a)：可判定、失败关闭；代价是云同步目录不能提交 |
| OD-2 | **在覆盖所有相关新版本写者的前提下**，确认入口清单与接入顺序 | (a) 单次发布全部写入入口；(b) 分批接入，未接入入口在同一存档上拒绝写入或声明只读；(c) 先只读诊断/预览，再按 (b)。**“只覆盖成长提交”不是选项**：那会缩小已批准范围，须重新授权 | (a) 或 (b)+(c) |
| OD-7 | 备份范围与保留 | **完整备份必须包含既有用户备份**（已批准范围，不能作为选项排除）；控制元数据（锁文件、`.operations/`、探针、暂存索引）的排除、保留期与清理责任可另议。排除既有用户备份须重新授权 | 包含用户备份；只排除控制元数据；先确认排除清单 |
| OD-9 | 完整性锁（内层锁）的有界化策略 | (a) 给现有锁加 deadline/取消（改变阻塞语义）；(b) 清单更新移出关键段（改变失败语义）；(c) 保持无限阻塞，则提交路径不承诺有界等待、T6b 按“未接入” | 先 (a) 或 (c)；未决前按 §2.7 停止接入 |
| OD-10 | `adopt` 的**人工确认字段与授权人**（不再讨论是否实现 adopt） | 必填字段候选：来源 slot/路径、来源 `save_id`（可得时）、人工确认人、确认时间、理由、证据摘要；`source_unknown` 的理由与最低证据；谁有权授权（所有者/指定 DM）；是否要求二次确认 | 全部必填 + 所有者或指定人授权；`source_unknown` 也须同一授权；未定前 adopt 不实现 |

**随简化不再作为本设计选项的旧条目**（为避免误解，列出而不重开）：OD-4（预览绑定存储）——预览一律不写，已关闭；OD-6（强制解锁）——已删除，只保留 `init`/`adopt` 的 abort；OD-3（记录隐私）——按工程默认处理：不写主机名与用户绝对路径，只写仓库相对路径与本地标识；OD-5（恢复授权）——已并入 D 流程的“由人显式选择”，不再单列；OD-8（旧版写者迁移）——属兼容/发布问题，不在本锁设计内。

### 5.3 可由工程决定的事

`init`/`adopt` 的字段名与记录 schema、`D_save` 与轮询退避、心跳写入点、阶段名与文案、确定性发布顺序、探针实现与残留报告、进程内“已持有集合”的检查方式、归档命名与摘要算法、临时/`.partial` 命名与保留、测试夹具与故障注入点、诊断输出格式（不含敏感数据）。

### 5.4 建议后续任务顺序（依赖顺序，不是排期）

1. **锁原语、`init` 与状态机骨架**（独立任务）：实现 P 前置校验、`init`/`resume`/`abort`、归档四步、锁身份校验、§2.5 判定；**不写目标文件**；跑 T1/T2a–T2f/T3/T4b1–T4b4c/T5a–T5g/T10/T14。前置：本设计经审查。
2. **旧锁行为修正**（独立任务，需行为变更审查）：checkpoint `Drop` 归属校验、取消 mtime 抢占、备份/回滚排除控制文件、`backup_save` 对特殊文件报错；不声称修复 #264。
3. **接入所有新版本写者**（依赖 1、2 与 OD-2）：按确认清单与顺序接入；跑 T6a；不改变业务语义。
4. **D 阶段机与备份验证**（依赖 3）：`admit`/`backup`/`candidate`，只到“候选已验证、未发布”；跑 T4a 发布前分支与 OD-7 确认的备份范围。
5. **发布、日志/完整性与幂等**（依赖 4 与 OD-9）：跑 T2a/T7a/T7b/T8；T6b 未接入前按“路径未接入”记录。
6. **显式恢复**（依赖 5）：跑 T9/T9a–T9d；恢复前归档、保护当前树、人工选择。
7. **`adopt`**（依赖 1 与 OD-10；OD-10 未决前不实现、副本不可提交）：实现后跑 T13a–T13i。
8. **#264 与跨平台/网络文件系统调查**（可与 1–7 并行，不能替代）：按[锁调查](CHECKPOINT-LOCK-INVESTIGATION.md) 的现场采集要求做，禁止公开私有路径。
9. **sidecar 入籍与 9/10→11 迁移**（依赖 3–6 与 Q 的具体设计）：按[身份契约](ADVANCEMENT-IDENTITY-CONTRACT.md) §5 的顺序，不得以本设计代替身份验证。

首阶段全部验收后暂停，不进入全量专长自动化。

## 6. 证据与限制

九字段验收证据矩阵发布在 Issue #277 对应的 PR 评论，`commit_sha` 绑定该 PR 的最终 head；本文件内不复制可能随 head 变化的运行结果。代码事实基线 `cc78760`；若源码变化，§1 的定位需重新核对。

已知限制：只做静态阅读与设计；所有合成测试、故障注入、平台实验均 NOT_RUN；未读取真实存档、书籍或秘密；未验证 Windows/macOS 文件锁、网络文件系统、目录/掉电持久性；预览一致性为 `unverified`；`init`/`adopt`/归档/状态机均未实现（`adopt` 在 OD-10 决定前不得实现，副本存档不可提交）；内层锁仍无界（§2.7、OD-9）；文件系统快照可能保留 inode 编号，跨安装复制保持 UNKNOWN；#264 根因仍 UNKNOWN；现有测试通过不构成锁安全证明。

回退：本文件为普通文档，可整体 revert，不触及存档、schema 或任何运行行为。
