# 成长协作锁域与恢复协议（设计 · 最小可用 v1）

状态：**设计 + v1 库已实现（未接线）**。本文按所有者授权收窄为**最小可用 v1**：只覆盖显式 `init`、稳定锁文件 + 原子状态文件、当前存档的正常成长提交与失败停止；**不支持** `resume`/`abort`/`adopt`、复制或移动存档的成长提交、历史归档。2026-11-16 所有者确认 OD-1/2/7/9（§5.2）；TASK-0143 已实现 `tools/talkdnd/src/advancement_lock.rs` 与合成测试，**未接入任何命令**（`progress::level_up` 未改），未改 schema/迁移、未写 sidecar、未改变已批准的 B/D/Q、备份/恢复边界与“预览不写”。§1 的代码事实基线 `cc7876077d705598eed7f08ad7b365168efccfdc`；实现与验证基线 `30066e4f495501645413017b3c8c5bd3dad0bdfe`。合成测试已在 Linux 本地执行（覆盖范围与证据见 §4 的「实现状态」）；Windows/macOS 由 CI 执行，网络盘、掉电持久性与整盘镜像类实验仍**未执行**。依据 [身份契约](ADVANCEMENT-IDENTITY-CONTRACT.md) §3–§6、[恢复选项](ADVANCEMENT-RECOVERY-OPTIONS.md)、[安全审计](ADVANCEMENT-SAFETY-AUDIT.md)、[锁调查](CHECKPOINT-LOCK-INVESTIGATION.md) 与[成长设计](superpowers/specs/2026-11-10-character-advancement-design.md)。只读公开源码、测试与既有文档；未读取真实存档、书籍、DM 秘密。

标记约定：**事实**指能从当前公开代码/测试直接读出的行为；**推断**指由静态事实推出、未实测的后果；**UNKNOWN** 指本次不能判定，不等于安全。现有测试通过**不是**锁安全证据；#264 未解决前，任何“现有锁已够用”的结论都不成立。

## 0. 本文件回答什么、v1 收窄了什么

回答：成长提交会碰哪些写入对象与今天的锁；v1 的 lock/state 两个文件；显式 `init`；唯一“可开始普通写入”的谓词与状态表；D 流程各阶段的持锁与失败；三平台合成测试矩阵。

**v1 明确不做**（范围收窄，不改变已批准边界）：`resume`/`abort`、`adopt`、复制/移动存档的成长提交、长期历史归档与四步归档、`save_id` 迁移/重绑、跨安装副本识别自动化、永久操作历史。复制存档的独立分支、恢复自动化与历史保留留到后续任务。**例外**：`admit` 覆盖前快照与恢复工作证据（§2.9）是**非永久控制元数据**，用于失败后判定，不属于“历史归档”。

**v1 的硬规则**：

1. **锁文件不可变**：`.talkdnd.advancement.lock` 只由 `init` 用 `create_new` 创建，只写一次不可变头；永不删除、永不 rename、永不改写。所有操作对它取 OS 锁，持有同一句柄。
2. **状态文件原子替换**：`.talkdnd.advancement.state` 从不原地截断；每次用同目录唯一 temp 写入 → `sync_all` → rename 覆盖 → 父目录 sync。零字节/缺失/校验失败 = 未知，失败关闭，**绝不**当作首次初始化。
3. **按失败类别处理**：`init` 若 state 未发布，留下 lock（可能）而无有效 state，正常操作失败关闭，直到人工在协议外备份后处理；若 state 已 rename 但目录 sync 结果不明，后续可能看到有效 `ready`，持久化风险必须报告，不承诺一定失败关闭。提交失败时，尚未开始任何目标写入的记为 `failed`（记录可重开），已发布或结果不明的记为 `in_doubt`（失败关闭，只允许人工按 D 备份恢复）。工具不提供 `resume`/`abort`。

保留：§1 已核实事实盘点；预览只读；内层锁停止接入；已批准 D 备份/候选/逐文件发布/恢复流程；OD-1/2/7/9 的 2026-11-16 决定（§5.2）；证据矩阵结构。

不承诺：多文件原子性、消除 TOCTOU、约束旧二进制/外部编辑器/不遵守协议的进程、网络文件系统语义、#264 已解决、复制/移动副本的检测或支持、永久历史、提交路径有界总等待（§2.6）。

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
| `.cache/`、`digest.md` | `start.rs::digest`、`cache_path` | 直接 `atomic_write` | 无 | **事实**：属于可再生成的缓存/派生输出。设计上作为控制元数据；OD-7 已决定控制元数据单独保留、不并入用户数据完整备份，具体是否清理仍待定。 |

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

## 2. v1 协议（AC-2）

### 2.1 两个固定路径与职责

| 路径 | 作用 | 规则 |
|---|---|---|
| `Saves/<slot>/.talkdnd.advancement.lock` | 稳定锁文件与初始化标记 | **仅 `init` 用 `create_new` 创建**；只写一次不可变头；**永不删除、永不 rename、永不改写**。所有操作对它取 OS 锁，持有同一句柄，不按路径重新打开。 |
| `Saves/<slot>/.talkdnd.advancement.state` | 唯一权威状态 | 从不原地截断；temp 写入 → 文件 `sync_all` → **首次发布用同目录 `hard_link`（目标已存在即失败，绝不覆盖）→ 删除 temp**；后续更新用 rename 覆盖 → 父目录 sync。零字节/缺失/不可解析/校验失败 = 未知，失败关闭。 |

- 锁文件存在只说明“协议已初始化”，**不说明状态干净**；是否可写只由 §2.4 用 state 判定。
- 正常提交**不创建** lock 与 state 这两个固定文件（提交更新 state 时的 temp 是更新机制），但会创建操作证据快照（§2.9）；预览**不创建任何文件**。
- 不提供 `resume`/`abort`/`adopt`；不保留长期历史归档。state 只保存当前状态，下一次操作会覆盖它；操作前快照与恢复工作证据是非永久控制元数据（§2.9），恢复的数据依据仍是 D 备份。

### 2.2 锁头、状态字段与身份

**锁头（不可变，单次写入）**：`schema`、`save_id`、`created_at`。头写好后不再修改；解析失败不可修复。

**状态字段（设计）**：`schema`、`revision`（单调递增）、`checksum`（覆盖正文）、`save_id`、`operation_id`、`state`、`stage`、`lock_identity`（当前锁文件标识：Unix `dev`+`ino`，Windows `creation_time` 或等价物）、`save_path`（由库从**可信 root 与实际 save_dir** 推导的仓库根相对规范化 POSIX 路径；调用方不自报）、`published[]`（路径 + 前像摘要 + 目标摘要 + done）、`pending[]`（已写 intent 未 done）、`backup_dir`、`candidate_dir`、`last_error`、`created_at`、`updated_at`。

**身份关系**：v1 不支持 `adopt`，因此 `state.save_id` 必须等于锁头 `save_id`；不等 → UNKNOWN 失败关闭。`lock_identity` 用于发现“当前锁不是写 state 时那把锁”（正常复制或替换会改变文件标识）；它不能发现保留标识的整盘镜像（§2.5）。

### 2.3 显式 `init`（唯一创建入口）

前置：`lstat` 显示 lock 与 state **都不存在**。

1. `create_new` 创建 lock 文件，保留它返回的**原始句柄 H**。
2. **立即**对 H 取 OS 锁（有界 deadline）；不按路径重新 `open` 来加锁。
3. 持锁后比较 H 的文件标识与路径当前指向文件标识；不一致 → **停止**，不写头、不写 state（路径已被替换，锁保护的是旧 inode）。
4. 用第二个句柄对同一路径 `try_lock`：它必须与 H 指向**同一文件**（身份比较），且必须被拒绝；只把预期的“已被锁”错误当作成功，其它错误或身份不符都失败关闭。第二句柄只用于自检，不用于持锁。
5. **写头前再次复核**“路径 → H → 文件标识”一致；不一致停止。
6. 通过 H 写入不可变头（`schema`、新 `save_id`、`created_at`），文件 `sync_all`。这是该文件的**唯一一次**写入。
7. 写 state temp：同目录唯一 temp 写入完整内容（`state=ready`、`revision=1`、`checksum`、`save_id` 与锁头相同、`lock_identity`、`save_path`、时间），随后对 **temp 文件** `sync_all`。
8. **发布前复核**“路径 → H → 文件标识”一致；不一致 → **停止，不发布 state**。若锁头已写，只能保证“不再发布 state”，**不能**声称此前未写头。
9. 用同目录 `hard_link` 把 temp 链成 state（目标已存在则失败并保留原 state；不支持硬链接的文件系统失败关闭），成功后删除 temp；随后同步父目录。目录 sync 不可用的平台（Windows）**不得当成功**：`hard_link` 之后 `init` 返回 `state_published_not_durable`（state 可能已发布、持久性未知、不得自动修复），保留两个文件，并声明该平台的提交不受支持。
10. 释放 OS 锁。

失败与边界：

- lock 或 state 任一已存在（含 0 字节）→ 失败关闭（不接管、不覆盖、不删除）。
- 路径在**写头前**被替换 → 不写头、不写 state；在**写头后、rename 前**被替换 → 停止且不发布 state，但锁头已经写入、无法撤回，**不得声称未写头**。最后一次复核与 rename 之间的非协作替换仍有 TOCTOU 窗口（§2.8），只能如实报告。
- 失败按**state 是否已发布**分类：
  - state **未发布**（temp 写入/文件 sync 失败、复核不一致，或 rename 前停止）→ 留下 lock（有头或无头）而无 state；后续正常操作失败关闭，需人工在协议外处理。
  - 发布 **成功**、但父目录 sync 失败或结果不明 → 无法证明 state 已持久。**本次 `init` 必须立即报告失败且不得继续**；由于没有持久标记，后续进程**不能声称知道这一次 init 曾同步失败**，只能报告一般性的持久化限制。后续可能看到有效的 `state=ready`（也可能在崩溃后消失），**不承诺“后续一定失败关闭”**。平台根本没有目录 sync（Windows）时，`init` 返回 `state_published_not_durable`：不返回 `Ok`、不声称持久、不允许提交，两个文件保留且不得自动修复。
- `init` 不删除或改写锁头；工具不提供 `resume`/`abort`。
- 人工恢复/清理在**协议外**：必须先备份，由人确认后手工处理（例如移除未完成的协议文件），本文不提供命令，也不假装工具能判断。
- 不存在“state 已建、lock 未建”的合法路径：state 永远在 lock 之后写。

### 2.4 唯一“可开始普通写入”谓词与状态表

**正常写入的打开顺序**：① 不带 `create` 打开 lock → ② 取得其 OS 锁（同一句柄，不重新按路径打开）→ ③ 复核“路径 → 句柄 → 文件标识”一致 → ④ 不带 `create` 打开 state → ⑤ 校验。

**前置校验**（任一失败即拒绝：一般记 UNKNOWN，身份不符另记 `IDENTITY_MISMATCH`，均不进入状态表）：lock 与 state 都存在且可读；state 可解析、`checksum`/`revision` 一致；`schema` 已知（锁头与 state 都要能解析）；必填字段齐全；`state.save_id` 与锁头一致；`save_path` 由库从**可信 `root` 与实际 `save_dir`** 推导（调用方不得自报；拒绝根外、链接、`..`、`.`、重复/尾随分隔符等歧义形式），并与 state 记录一致（不符 → 见 §2.5 的复制/移动拒绝）；`lock_identity` 与当前锁句柄一致；目录**逐条枚举**（迭代错误也要传播，不能只取迭代器）；`state=ready` 必须 `stage=init` 且 `pending`/`published` 为空（自洽 checksum 不能让矛盾状态合法）。

**每次更新前的复核**：每次状态更新（包括 `check_writable`）都重新读取磁盘 state，校验存在、可解析、`checksum`、`schema`、`save_id`、推导出的 `save_path`、`lock_identity`，并要求它与本持有者读到的 revision **完全相同**。缺失、零字节、损坏或被替换 → 失败关闭：不写 temp、不 rename、不重建。这次读取返回的原始字节就是 §2.9 快照记录的内容，**不再二次读取**。谓词还先检查平台的目录持久能力：平台不支持时即使 `ready` 也拒绝（该能力不是可写证明）。读取与随后的 rename 仍是两次系统调用，中间的非协作写入仍在残余 TOCTOU 内（§2.8）。

**revision/checksum 的检测边界**：它们能检出可验证的损坏与（在有可验证先前 `revision` 或外部可信证据时）倒退。**若把 state 整体替换为一个自洽的旧版本或整盘镜像，且没有可验证锚、重放不可观察**，v1 可能把旧记录当作合法；**不能保证失败关闭**。只有存在已知或外部证据表明发生了重放时，才停止并交人工处理。该限制必须随错误/诊断信息明确记录，不得写成“必然拒绝”；要保证检测需要可信锚或外部确认，属后续任务。

**谓词**：只有下列之一且前置校验全过，才允许开始普通写入——① 前态 `ready`；② 前态 `complete` 且候选比对/日志/完整性完成、无 `pending`；③ 前态 `failed` 且**已证明零目标写入**（`published[]` 空、`pending[]` 空、无本次日志/完整性写入）。其余一律拒绝。

| 优先级 | 状态 | 额外条件 | 判定 | 允许的动作 |
|---|---|---|---|---|
| 1 | `active` | — | 上次操作未正常结束（能取得锁说明原持有者已退出，但仍按未结束处理） | 拒绝普通写入；人工按 D 备份恢复 |
| 2 | `in_doubt` | — | 部分发布或结果不明 | 拒绝普通写入；人工按 D 备份恢复 |
| 3 | `failed` | `published[]` 空、`pending[]` 空、无本次日志/完整性写入 | 写入前停止 | **允许**新操作（覆盖前保留备份） |
| 4 | `failed` | 任一已发布、未决 intent 或日志/完整性结果不明 | 与 `failed` 语义矛盾 | 失败关闭；按 `in_doubt` 处理 |
| 5 | `complete` | 候选比对、日志/完整性完成、无 `pending` | 已完成 | **允许**新操作（覆盖前保留备份） |
| 6 | `ready` | 无进行中操作 | 干净 | **允许**新操作 |
| 7 | 其它组合 | — | UNKNOWN | 失败关闭 |

缺失、零字节或校验失败的 state **不进入本表**：一律失败关闭。`complete` 只在全部候选比对、日志/完整性完成且无未决 intent 后写；任何部分发布或结果不明 → `in_doubt`；`failed` 只用于尚未开始目标写入的停止。这是写终态时的强制语义，不是命名约定。

### 2.5 复制/移动存档：v1 不支持

- `state.save_path` 由库从可信 `root` 与实际 `save_dir` 推导后与记录比较；不符 → 失败关闭，错误明确写“v1 不支持复制存档的成长提交；请恢复原路径或等待后续任务”。调用方无法通过自报路径绕过这一比较。推导按**平台编码字节**逐组件校验，不使用有损 UTF-8 转换：任何非 UTF-8 组件、含反斜杠的组件、根外、链接、`..`/`.`、重复或尾随分隔符都拒绝；无法无损记录为 POSIX 字符串时直接 `invalid_request`。
- 正常复制通常还会让 `lock_identity` 与当前锁句柄不符 → 同样失败关闭（这只能发现“不是写 state 时那把锁”，不是完整识别）。
- **相同相对路径的整盘/快照级镜像**可能同时保留 `save_path`、`save_id` 与文件标识：v1 **可能无法检测**，也**不声称**能检出。文档与错误信息必须说“不支持、可能不可检测、需人工确认”，不得暗示自动识别。Windows 的尽力标识只是 `creation_time`，保留它的副本同样可能不可检测。
- v1 **没有** `adopt`、副本独立分支或移动后重绑；这些都留到后续任务。失败后的人工处理（恢复原路径、从备份恢复、等待支持）在协议外，且必须先备份。

### 2.6 等待预算与内层锁（停止接入）

- 持有不是租约：不用 TTL 判定死亡，不删 lock、不改头、不接管。
- 获取有 deadline（默认 5 秒，可显式覆盖）；等待方失败，不无限阻塞。
- 总预算 = `D_save` + 各阶段内层锁 `D_inner` 之和；每次内层获取都必须有 deadline 与取消路径。
- **事实**：完整性锁 `File::lock()` 阻塞且无超时；checkpoint 锁是 `create_new`、不等待。因此当前任何在 SAVE 内获取完整性锁的路径（`audit::after_command`、`PartyState::save`、`migrate::ensure`、`integrity::*`）都无法给出有界总等待。
- **接入停止条件**：内层锁获得有界获取/取消能力之前，含完整性锁的提交路径**不得接入**；不得把 `D_save` 或并发测试结果外推为整条提交路径有界。OD-9 已决定按“新路径有界获取”执行（默认 5 秒 deadline，可显式覆盖），现有完整性锁**保持**无限阻塞语义，因此本条停止条件仍然有效；未接入前 T6b 按“未接入”记录。
- 心跳/时间仅用于诊断；不存在强制解锁或 abort 命令。
- **平台边界**：目录 sync 不可用的平台（当前 Windows）**不能**建立提交，且**可写谓词本身**必须拒绝：`check_writable` 先检查平台目录持久能力，即使 `ready` 也返回 `unwritable_state`（该能力不是可写证明）；`init` 在 state 已发布后返回 `state_published_not_durable`，`begin_operation` 同样被拒；不得把“`init` 建了文件”当成“可提交”。本库也没有任何准入/拒绝网络盘或同步目录的强制门：第二句柄自检只证明同一客户端的锁 API。

### 2.7 预览必须只读

- **不打开、不创建、不锁** lock 或 state；不写探针、不写 `.cache`、不写任何文件；只做只读发现与读取。
- `preview_id` = 对（完整枚举清单 + 每文件摘要 + 规则集 + 授权状态 + 遭遇状态 + 身份映射 + 计划变更）的摘要，由调用方保存；预览不持久化。
- 输出 `consistency: unverified`；读取期间可能有并发写者，不声称快照。
- 提交必须按 §2.4 的顺序取锁、重新发现并重算 binding，与 `preview_id` 比较；不一致拒绝并要求重新预览。只给单一摘要时，提交**不得**列具体差异类别，只能报“绑定变化，原因未知”；提供可校验分项清单时才可列类别。未提供 `preview_id` → 拒绝。
- lock/state 缺失、只读目录、无法探测：预览仍可跑（它不需要锁），但必须报 `write_capability: unverified`，不承诺可提交。

### 2.8 崩溃、替换、平台原语与共存

- 崩溃/被杀：OS 释放锁。lock 与锁头保留；state 因原子替换要么是旧值要么是新值。下一次按 §2.4 判定：`active` → 拒绝普通写入，人工按备份恢复；state 缺失/损坏 → 失败关闭。
- 锁文件被删除/替换：持有者在阶段边界复核“路径 → 句柄 → 文件标识”，不一致立即中止、不再写目标。之后：lock 缺失 → 一切正常操作失败关闭，`init` 也拒绝（state 存在）；出现新 lock（非协作创建）→ `lock_identity` 不符 → `IDENTITY_MISMATCH` 失败关闭。路径/句柄复核不能发现旧 inode 的其它持有者。
- state 被删除/零字节/校验失败：失败关闭；**绝不**当作首次初始化，也绝不自动重建。恢复只能按 D 备份人工处理。
- 平台：Linux/macOS `flock`（advisory，按打开文件描述）；Windows `LockFileEx`（字节区锁）。**Windows 实测（TASK-0143 CI）**：持有排他锁时，其它句柄连**读取被锁字节区**都会被拒（`ERROR_LOCK_VIOLATION`），因此持锁期间只能用元数据查询做身份复核，不能另开句柄读取锁文件内容；本库按此实现。网络盘/同步目录/FUSE **UNKNOWN**。`init` 用“第二句柄须与 H 同一文件、且 `try_lock` 必须被拒”做能力自检（不创建探针文件）；失败关闭。该自检只证明同一客户端的锁 API。
- 共存：总序 `SAVE → checkpoint .talkdnd.lock → integrity .talkdnd.integrity.lock`；反向获取算协议违规。checkpoint/完整性锁语义不变。必须先修的旧行为：checkpoint `Drop` 无条件删除锁文件、mtime 抢占、备份/回滚复制锁文件、`migrate::backup_save` 静默跳过特殊文件——修掉前新锁只约束遵守新协议的写者。
- 不受约束：旧版本二进制、外部编辑器、脚本、同步客户端、杀毒软件、备份工具、其它机器客户端、PDF 转换进程、手工复制。当前 `combat-sim` 尚未接入（写遭遇/party 不取 SAVE）；接入前把遭遇写入当并发风险，活动遭遇只预览。
- 残余 TOCTOU 无法消除：每文件 rename 前复核仍有窗口；只能缩小、记录复核值、失败时说明“可能已覆盖某次外部写入”。#264 未解决期间同样适用；本设计不解释、不修复首次 `WouldBlock`。

### 2.9 操作前快照与恢复工作证据（控制元数据）

存储位置：`Saves/_backups/.operations/<slot>/`（存档外的控制元数据）。它是**操作证据**，不是永久历史，也不是用户数据。**OD-7 已决定**：它是与用户数据备份分开保留的控制元数据，不并入 D 的用户数据完整备份；保留期与清理责任仍未定，未定前不得把它当可任意清理的缓存。在相关操作未解决前**不得删除**。保留期与清理不在 v1 承诺内。

**`admit` 覆盖前快照**（每次操作、包括首次）：

1. 持锁并完成 §2.4 前置校验后，先 `create_new` `<operation_id>-prestate.yaml`。
2. 写入操作前的 **state 原始内容**（或规范化后正文）及其 `revision`/`checksum`、`lock_identity`、`save_path`、原 `operation_id`/`state`/`stage`、时间；文件 `sync_all`。
3. 用 §2.4 那次读取并校验的**原始字节**构造快照（不再二次读取），文件 `sync_all` 后重读校验；随后同步**包含该文件目录项的 `<slot>/` 目录**。若本次新建了目录，必须**从首个已存在祖先到证据目录逐层建并在每层后同步其父目录**；**已存在的层级也要重新同步**（可能是上次失败留下的），无法确认即失败关闭。任一同步失败/**平台不支持**（当前 Windows）→ 停止，保留原 state，不写 `active`。
4. 只有以上全部成功，才允许原子替换 state 为 `state=active`。任一步失败/结果不明 → **停止**，保留原 state，不写 `active`，不写任何目标。
5. 首次操作（前态 `ready`）同样必须先做快照；不能因为“没有旧操作”而跳过。

**`recovery` 工作证据**（在任何目标写入之前）：

1. 持锁后先 `create_new` `<recovery_id>-work.yaml` 与受保护快照，内容至少包含：原未完成 state 的原始字节/`revision`/`checksum`/`operation_id`/`stage`/`pending[]`、相关备份位置、人工选择（回滚到前像 / 继续到目标）、新恢复 ID、原操作引用、时间。
2. 写+`sync_all`+重读校验；并同步包含记录目录项的 `<slot>/` 目录（新建目录时连同新目录及父目录）——失败/不明 → 停止，不做任何目标写入。
3. 恢复过程中每文件的 intent/done 通过原子替换更新**工作记录**（temp + `sync_all` + rename + 父目录 sync），不修改受保护快照。
4. 再次中断时，凭工作记录与受保护快照可判定“原操作是哪次、恢复到哪一步”；无法判定 → `in_doubt`，只允许人工处理，不自动续跑/重放。

这些文件是“失败后仍能判定”的最小证据；v1 不承诺永久保留，也不把它们当作可任意清理的缓存。


## 3. 锁在已批准 D 流程中的位置（AC-3）

D 流程（完整备份 → 候选构建 → 逐文件发布 → 日志/完整性 → 完成/恢复）与恢复边界沿用[恢复选项](ADVANCEMENT-RECOVERY-OPTIONS.md) §3 与[身份契约](ADVANCEMENT-IDENTITY-CONTRACT.md) §3.5，不因 v1 收窄而改变。**两类数据必须分开**：① **操作证据快照**（`admit` 覆盖前 state 快照、恢复工作记录/受保护快照）是控制元数据，用于失败后判定，不替代用户数据；② **D 完整备份**是用户数据的恢复依据。OD-7 已决定完整备份包含既有用户备份，控制元数据单独保留、不并入该备份；保留期与清理仍未定。v1 不保留永久历史；操作证据在覆盖/恢复期间存在，未定前不得假定可排除或可清理。

| 阶段 | 持锁与状态 | 动作（设计） | 失败停止 | 可重试边界 |
|---|---|---|---|---|
| `admit` 受理 | 全程持锁；`ready`/`complete`/`failed(零写入)` → **先 §2.9 操作前快照** → 写 `state=active, stage=admit` | 按 §2.4 打开与校验；生成 `operation_id`；独占创建并校验 `<operation_id>-prestate.yaml`（含原 state 字节/revision/checksum/身份与阶段）；只有快照成功后，才原子替换 state 为 `active` | 校验失败、快照创建/写/sync/重读/目录同步失败或结果不明 → **停止**，保留原 state，不写 `active`，不写目标 | 整次重新开始（新 `operation_id`，新快照）；首次操作也不得跳过快照 |
| `backup` 完整备份 | 持锁，`active` | 存档外独占创建备份目录；复制全部用户数据（含既有用户备份；控制元数据按 OD-7 单独保留、不并入本备份）；逐文件路径/类型/大小/摘要；校验副本；同步文件与目录；state 记 `backup_verified` | 空间不足/特殊文件/权限/同步失败 → 停止；保留不完整备份 | 同操作内新目录重做（旧目录保留） |
| `candidate` 候选构建 | 持锁，`active` | 同一文件系统/卷建暂存树（跨设备拒绝）；按已验证前像生成候选；校验 schema/身份/关系与每文件目标摘要 | 空间不足/校验失败/检测到外部变化 → 停止；保留备份与候选 | 重建候选；不得把不完整候选标为已验证 |
| `publish` 逐文件发布 | 持锁，`active` | 固定顺序（`sheets/*.yaml` → `combat/party-state.yaml` → `Characters/*.md` → `progress.yaml` → 追加日志 → 完整性清单）；每文件：state 写 intent（`pending`）→ 复核当前=前像 → 同目录临时文件+rename → 同步文件与父目录 → 标 done | intent 写失败 → 不发布该文件；复核不符/写失败/同步失败 → **立即停止**；已发布/未发布写回 state | 不自动重试发布；恢复按“当前=目标→已应用；当前=前像→可应用；都不是→停止”逐文件判断 |
| `logs` 日志与完整性 | 持锁，`active`（完整性清单另取内层锁） | 追加日志按操作 ID 去重；不截断、不改写既有行；尾部异常停止；清单更新保留既有 dirty | 追加失败/清单失败 → 停止；置 `in_doubt` 并报告“日志可能已追加、清单可能未更新” | 日志仅在能证明未追加时重试；清单可重算但不洗掉 dirty |
| `complete` 完成 | 持锁，`active` → 终态 | 重新发现并比对候选清单：无意外新增/删除/变化、`published[]` 与候选一致、`pending[]` 空、日志/完整性完成；写 `state=complete` | 比对不符/未决 intent/结果不明 → `in_doubt`，拒绝普通写入 | 校验可重复；清理不属于成功证明 |
| `recovery` 显式恢复 | 持锁（自身一个 `operation_id`） | **任何目标写入前**先按 §2.9 建立恢复工作证据：`create_new` `<recovery_id>-work.yaml` + 受保护快照（原未完成 state 字节/revision/checksum/operation_id/stage/pending、备份位置、人工选择、原操作引用），`sync`+重读校验+目录同步；之后保护恢复时当前树（D 备份）；逐文件比较当前与前像/目标；工作记录逐文件记 intent/done；由人显式选择“回滚到前像”或“继续到目标”；逐文件执行并同步 | 工作证据创建/校验失败、备份校验失败、既非前像也非目标、日志已有后续追加、身份不明、无法判定原操作或恢复进度 → 停止；保留证据；不删除后续日志 | 不自动重试；再次中断凭工作记录与快照判定原操作与进度，由人重新选择，不自动续跑/重放 |

阶段报告固定字段：操作 ID、状态、阶段、锁状态、lock/state 位置、备份/候选位置、已发布/未发布清单、结论（未写/部分写/已写）、需要的人工动作、不得自动执行的动作。

**明确不承诺**：多文件原子性；崩溃后自动恢复；恢复保留未在备份中的后续内容；锁能阻止非协作写者；目录同步/掉电语义（`fsutil::sync_parent` 在 Unix 忽略目录打开与同步错误、非 Unix 不执行；`atomic_write` 成功不证明目录项已持久化）；v1 不提供长期协议历史——操作前的 state 由受保护的操作证据保存（非永久），**用户数据的恢复依据是 D 备份**；操作证据被清理后的可恢复性不作保证。

**幂等**：属性值、XP、资源、专长选择等“只加一次”的效果由 state 与每目标标记保证恰好一次；日志去重只能用操作 ID 标记。校验、摘要、复制到新目录可重试；发布、日志追加、恢复动作在不能证明状态前不可重试。

## 4. 合成测试矩阵（AC-4）

设计阶段全部 **NOT_RUN**；TASK-0143 实现后的实际覆盖见本节末「实现状态」，其余行仍为未执行。下表只保留 v1 范围；“CI”指三平台 GitHub Actions 可跑，“人工/特权”指需要多主机、网络文件系统、特权或物理操作。断言是期望，不是当前结果。

| ID | 场景与注入 | Linux 期望 | Windows 期望 | macOS 期望 | 环境 | 判定要点 |
|---|---|---|---|---|---|---|
| T1 | 两个协作写者同时提交 | 一方取锁进入 `active`，另一方 deadline 失败且未写目标 | 同左（`LockFileEx`） | 同左（`flock`） | CI | 失败方不写目标；不出现第二个 `active` |
| T2a | 持锁期间 lock 被外部删除/替换 | 阶段边界检出“路径→句柄→文件标识”变化，立即中止 | 同左（删除是否被拒记录实际值） | 同左 | CI（子进程注入） | 不再写目标；不宣称阻止替换 |
| T2b | 外部替换被 OS 拒绝 | 无状态变化；记录拒绝与错误 | 记录实际观察值 | 同左 | CI | 不要求不可观察的尝试 |
| T2c | 外部删除 lock 并为非协作进程新建 lock | `lock_identity` 不符 → `IDENTITY_MISMATCH` 失败关闭；`init` 因 state 存在而拒绝 | 同左 | 同左 | CI（两进程） | 不以“lock 存在”为干净 |
| T2d | 两进程都看到 lock/state 不存在，竞争 `init` | lock 的 `create_new` 唯一赢家；输家失败关闭、未创建任何文件 | 同左 | 同左 | CI（两进程） | 初始化仲裁在 lock 的 `create_new` |
| T2e | 正常操作时 lock 或 state 缺失（含 `lstat`→`open` 之间被删） | `NotFound` → 失败关闭；不创建文件、不进入写阶段 | 同左 | 同左 | CI（删除时序） | 正常操作永不建文件 |
| T2f1 | `init` `create_new` 成功、**首次身份比较前**路径被替换 | 对原始 H 取锁后比较身份不一致 → 停止；不写头/state | 同左 | 同左 | CI（替换时序注入） | 不按路径重开持锁 |
| T2f2 | 锁头**已写**、state temp **已同步**、rename 前路径被替换 | rename 前复核发现不一致 → **不 rename、不发布 state**；不得声称未写头；最后复核与 rename 之间的非协作替换属已声明 TOCTOU，只能报告 | 同左 | 同左 | CI（rename 前注入） | 第二句柄须验同一文件，只把预期竞争错误当成功 |
| T3 | 活持有者超时（等待方 deadline 到期） | 等待方失败，不删 lock、不改头、不接管 | 同左 | 同左 | CI | 不出现第二个持有者 |
| T4a | 崩溃/被杀（每阶段边界 `SIGKILL`/`TerminateProcess`） | OS 释放锁；state `active` → 拒绝普通写入，人工按备份恢复 | 同左 | 同左 | CI | 不自动续跑 |
| T4b | state 写入各窗口：temp 写/`sync_all` 失败；rename 成功但父目录 sync 失败（提交与 `init` 均覆盖） | temp 失败 → 旧 state 完整；rename 后目录 sync 失败 → 结果不明，按实际内容判定；`init` 场景下后续可能看到有效 `ready`，必须报告持久化风险，不能只断言拒绝或旧 state | 同左 | 同左 | 需实现后故障注入 | 原子替换不等于目录持久 |
| T4c | lock 头写入短写/崩溃 | 头不可解析 → 失败关闭；无 resume，只能人工处理 | 同左 | 同左 | CI（分阶段 kill） | 头只写一次、不可改写 |
| T4d | `admit` 操作前快照失败（首次操作与有旧 state 两种）：创建/写/文件 sync/重读/**`<slot>/` 目录同步失败或平台不支持** | 不写 `active`、不写目标；原 state 保持；报告快照失败 | 同左 | 同左 | 需实现后故障注入 | 首次操作也不得跳过快照；目录同步失败必须停止 |
| T4e | `admit` 快照成功后、写 `active` 前崩溃（含“旧 D 备份不是最新”） | 原 state 仍是旧值；无 `active`、无目标写入；下一次正常判定；不从旧备份推断成功 | 同左 | 同左 | CI（分阶段 kill） | 恢复依据是操作前快照，不假设最新 D 备份 |
| T5a | `init` 在 lock 创建后、写头前崩溃 | lock 为 0 字节、state 缺失；正常操作拒绝；无自动恢复 | 同左 | 同左 | CI（分阶段 kill） | 不删除、不重建 |
| T5b | `init` 在写好头、写 state 前崩溃 | lock 头完整、state 缺失；正常提交拒绝；只能人工处理 | 同左 | 同左 | CI | state 永远在 lock 之后 |
| T5c | `init` 时 lock 存在 + state 缺失；state 存在 + lock 缺失；两者都在 | **`init` 一律拒绝**（不接管、不重建、不创建）；正常提交则要求两文件都存在，缺一即失败关闭 | 同左 | 同左 | CI（构造目录） | 区分“init 拒绝”与“正常提交缺文件” |
| T5d1 | state 零字节/不可解析/`checksum` 不符 | 一律失败关闭；绝不首次初始化、不自动重建 | 同左 | 同左 | CI（构造内容） | 无法判定即拒绝 |
| T5d2 | `revision` 倒退：存在可验证先前 `revision`/可信锚 | 检出并失败关闭 | 同左 | 同左 | CI（构造内容） | 只在有锚时声称检出 |
| T5d3 | 整体替换为自洽旧 state 或整盘镜像（无可验证锚，重放不可观察） | **可能把旧记录当作合法，不能保证失败关闭**；只有存在已知/外部证据表明重放时才停止并人工处理；必须记录该限制，不得写成必然拒绝 | 同左 | 同左 | 人工/特权（镜像） | 可信锚/外部确认留后续任务 |
| T5e | lock 头不可解析/不完整（非空） | 失败关闭，人工处理 | 同左 | 同左 | CI（构造内容） | 头不可修复 |
| T5f | `state.save_path` 与当前路径不符（复制/移动） | 失败关闭，错误说明“v1 不支持复制存档的成长提交” | 同左 | 同左 | CI | 无 adopt、无重置 |
| T5g | `lock_identity` 与当前锁句柄不符（正常复制/替换） | `IDENTITY_MISMATCH` 失败关闭 | 同左 | 同左 | CI | 只证明“不是那把锁” |
| T5h | 目录不可读/枚举失败 | UNKNOWN 失败关闭 | 同左 | 同左 | CI | 不当作空 |
| T5i | `init` state rename 成功、父目录 sync 失败或结果不明 | 本次报告失败、不得继续；**后续可能看到有效 `ready`（也可能崩溃后消失）**，存在持久化风险，不得断言必然拒绝或旧 state | 同左 | 同左 | 需实现后故障注入 | 与“state 未发布”分开判定 |
| T6a | 顺序反转（checkpoint 快照/恢复 vs 成长提交） | 按总序串行，一方等待或失败，**不死锁**；测试级超时 | 同左 | 同左 | CI（两进程+超时） | 挂起即失败 |
| T6b | 内层完整性锁被另一进程长期持有 | 提交必须在预算内失败并报告内层锁；或该路径标记“未接入” | 同左 | 同左 | CI（超时） | 不把 `D_save` 当整体有界 |
| T7a | 非协作编辑，注入器已知值（预览后/发布前/intent 后） | 在定义检查点检出并拒绝；产品未覆盖注入字节 | 同左 | 同左 | 需实现后故障注入 | 注入值仅测试侧 |
| T7b | 非协作编辑发生在最后复核后、rename 前 | 只能报“无法排除覆盖”，列出已观察前像与目标；不写“实际值” | 同左 | 同左 | 需实现后故障注入 | 不宣称检测成功 |
| T8 | 预览过期（绑定变化） | 提交拒绝；单一 `preview_id` 时只报“绑定变化，原因未知” | 同左 | 同左 | CI | 无分项清单不得列类别 |
| T9a | 恢复工作证据创建失败：work 记录/受保护快照的独占创建、写、文件 sync、重读摘要或目录同步失败 | 停止；不做任何目标写入；原未完成 state、备份位置与 intent 保持 | 同左 | 同左 | 需实现后故障注入 | 证据未成立不得开始恢复 |
| T9b | 恢复中断：工作证据后、每文件 intent/done 各点 kill；再次进入 | 凭 work 记录与快照判定原操作与恢复进度；不自动续跑/重放；无法判定 → `in_doubt`，人工选择 | 同左 | 同左 | CI（分阶段 kill） | 不重复应用；不依赖永久 history |
| T10 | 释放/替换语义：lock 从不 unlink；state 从不原地截断；外部替换后释放 | 释放不删除 lock；state 以 rename 更新；被替换的 lock 不被改动 | 同左（删除失败单独断言） | 同左 | CI | 不拆被持有的 inode |
| T11 | `init` 锁能力自检失败（同一 lock 第二句柄也能取锁、网络盘、只读卷） | 失败关闭；报告平台/文件系统；不建探针 | 同左 | 同左 | 人工/特权（网络挂载、只读卷） | 不降级 |
| T12 | 跨主机/网络文件系统互斥 | **UNKNOWN**：本地自检不证明跨客户端互斥 | **UNKNOWN**（SMB/OneDrive） | **UNKNOWN**（NFS/云同步） | 人工/特权 | 未证明前按“不支持” |
| T13a | 复制存档（可检测：路径或锁标识不符） | 失败关闭并提示 v1 不支持；不写目标、不改 state | 同左 | 同左 | CI | 不做 adopt、不重置身份 |
| T13b | 相同相对路径的整盘/快照镜像（路径与标识可能一致） | **可能无法检测**；不得宣称识别；文档/错误必须要求人工确认 | 同左 | 同左 | 人工/特权 | 只证明“不宣称检出” |
| T13c | 移动存档（路径不符） | 失败关闭；要求恢复原路径或等待后续支持 | 同左 | 同左 | CI | 不支持移动后写入 |
| T14 | 终态语义：完整验证 → `complete`；部分发布 → `in_doubt`；伪造 `complete`；`failed` 带发布记录 | 只允许验证后 `complete`；部分发布只能 `in_doubt` 且拒绝普通写入；伪造/矛盾终态失败关闭 | 同左 | 同左 | CI（构造状态） | 状态名不替代前置条件 |

**只能由实现后的故障注入证明的结论**：T2a/T2c/T2d/T2e/T2f1/T2f2/T4b–T4e/T5a/T5b/T5i/T5d1/T7a/T9a/T9b/T14 的阶段边界中止、前置校验、初始化唯一赢家与同句柄锁定/复核、正常操作不建固定文件、state 原子替换与目录持久窗口、操作前快照与恢复工作证据、恢复不重复应用。`T7b` 只能证明“如实报告不可排除”（不输出注入值）；T11/T12/T13b/T5d3 需人工/特权；T13b、T5d3 只能证明“不宣称识别/不保证检出”，不能证明检测能力或必然拒绝。**T6b 在接入前按“路径未接入”记录，不得当作通过。**

**实现状态（TASK-0143，库与合成测试，未接线）**：`tools/talkdnd/src/advancement_lock.rs` 实现 §2.1–§2.6 与 §2.9 前半（`admit` 操作前快照），测试在 `tools/talkdnd/tests/advancement_lock.rs`。已执行的合成场景：init 唯一赢家与部分协议状态拒绝（T2d、T5a–T5c）、init 两处路径替换中止（T2f1、T2f2）、init 首次发布不覆盖已出现的 state（T5i 的覆盖分支）、正常操作不建固定文件（T2e）、零字节/缺失/不可解析/未知字段/校验失败/矛盾 `ready`/伪造终态失败关闭（T5d1、T14）、`ready`/`complete`/`failed(零写入)`/`active`/`in_doubt` 判定与终态条件（T14）、两写者超时与第二句柄自检（T1、T3）、持有期间替换锁文件（T2a）、持锁后删除/置零/损坏/替换 state 后拒绝更新且不重建（§2.4 的每次更新复核）、复制/移动拒绝（T5f、T5g、T13a，含同卷 rename 与根外/链接拒绝）、只读成功路径与只读目录下的写失败（T4b 的 temp 分支）、目录无法枚举时失败关闭、原子替换无残留 temp（T10）、`admit` 快照成功/嵌套新建目录/目录无法创建/平台无目录 sync 时失败（T4d）。多进程用例在三个平台都跑；路径替换注入、只读权限注入与同相对路径副本用例是 Unix-only。Windows 没有目录 sync，因此 `init` 在 state 已发布后返回 `state_published_not_durable`（不返回 `Ok`，两个文件保留），`check_writable` 即使 `ready` 也拒绝，`begin_operation` 必然失败：本版不支持 Windows 本地提交。第二轮复审后补充：`init` 首次发布用 `hard_link` 不覆盖、发布后目录 sync 不可用即返回 `state_published_not_durable`（不返回 `Ok`）；可写谓词纳入平台目录持久能力；每次更新前单次读取并复用该字节构造快照；`ensure_dir` 对已存在层级也重新同步；非 UTF-8 路径按平台编码字节拒绝。新增用例：`a_non_utf8_save_component_is_refused_without_touching_its_target`、`a_directory_named_with_the_replacement_character_round_trips_losslessly`、`a_failed_parent_sync_after_mkdir_is_rechecked_on_retry`、`a_state_changed_between_open_and_begin_operation_is_refused`、`init_reports_published_but_not_durable_and_refuses_every_commit`（Windows）。**NOT_RUN**：`readdir` 迭代中途错误（无可移植注入）、Windows 同长度改写检测。这些结果只证明合成 fixture，不证明真实存档安全。

**不进入 CI 的**：掉电/设备谎报同步、磁盘控制器缓存、文件系统修复、杀毒软件拦截、容器/虚拟机快照恢复、真实玩家存档与非协作文本编辑器；全部保持 UNKNOWN。

## 5. 边界、决策与后续（AC-5）

### 5.1 v1 是范围收窄，没有改变已批准事项

- 设计阶段只新增本文件；实现阶段（TASK-0143）只新增 `tools/talkdnd/src/advancement_lock.rs` 与其合成测试，并在 `tools/talkdnd/src/lib.rs` 注册模块、在本文件与 `docs/PROJECT-CHARTER.md` 记录四项决定。无 CLI、无接线、无 schema/迁移、无依赖、无 CI、无规则数据、无 MANIFEST、无存档或秘密变更。
- v1 **不改变**已批准的 B/D/Q、备份/恢复边界、“预览不写”、遭遇“活动只预览”、旧程序不受约束、schema 11 目标。
- 新增的只是 v1 原语；它**不产生“已安全”的结论**，不消除 TOCTOU，也不解释 #264。#264 保持未解决风险。
- 被推迟到后续任务（不是取消已批准目标）：`resume`/`abort`、`adopt`、复制/移动存档的成长提交、副本独立分支、四步历史归档、历史保留与恢复自动化。

### 5.2 已决定（所有者 2026-11-16，TASK-0143）

四项在 v1 设计里标注为待定的决策已由项目所有者确认。下表保留原选项与影响，并在「决定」列写明本次选择；未列入本表的仍按第 5 节决策门处理。

| ID | 问题 | 选项与影响 | 决定（2026-11-16） |
|---|---|---|---|
| OD-1 | 支持的平台/文件系统边界 | (a) 只承诺本地常见文件系统，网络盘/同步目录一律拒绝；(b) 依赖自检并允许网络盘“尽力”；(c) 每平台逐项验证后再承诺 | **(a)**：决定是只支持本地常见文件系统；网络盘、FUSE、同步目录不支持。**强制门尚未实现**：第二句柄自检只证明同一客户端的锁 API，不识别网络盘/FUSE/同步目录，本库没有任何准入拒绝，因此现在**不能**声称这些位置已被拒绝；提交入口在所有相关写者覆盖前不得启用。跨主机/网络互斥仍 UNKNOWN。代价是云同步目录里的存档不能提交。 |
| OD-2 | **在覆盖所有相关新版本写者的前提下**，确认入口清单与接入顺序 | (a) 单次发布全部写入入口；(b) 分批接入，未接入入口在同一存档上拒绝写入或声明只读；(c) 先只读诊断/预览，再按 (b)。**“只覆盖成长提交”不是选项**：缩小范围须重新授权 | **(b)+(c)，并加一条硬约束：成长提交入口在所有相关新版本写者被覆盖之前不启用。** 未接入的入口必须在同一存档上拒绝写入或声明只读；只读预览/诊断可以先上线。缩小到“只覆盖成长提交”仍须重新授权。本任务**不启用**提交入口，也没有接入任何写者。 |
| OD-7 | 备份范围与保留 | **完整备份必须包含既有用户备份**；控制元数据（lock 文件、操作证据快照/恢复工作记录）的排除、保留期与清理责任可另议，未确认前不得假定获准排除。排除既有用户备份须重新授权 | **完整备份包含既有用户备份**；lock 文件、操作证据快照与恢复工作记录作为**控制元数据单独保留**，不并入 D 的用户数据完整备份。保留期与清理责任仍未定，未定前不得假定可以删除或排除；排除既有用户备份仍须重新授权。 |
| OD-9 | 完整性锁（内层锁）的有界化策略 | (a) 给现有锁加 deadline/取消（改变阻塞语义）；(b) 清单更新移出关键段（改变失败语义）；(c) 保持无限阻塞，则提交路径不承诺有界等待、T6b 按“未接入” | **(a) 限于新路径**：新成长路径按 §2.6 有界获取（默认 5 秒 deadline，可显式覆盖，超时失败关闭）。现有完整性锁**保持**无限阻塞语义不变，因此含完整性锁的提交路径仍不得接入，整条提交路径**不承诺**有界等待，T6b 按“未接入”记录。 |

**v1 移除的决策项**：OD-10（adopt 的人工确认字段与授权人）随 adopt 一起留到后续任务；不再讨论是否实现 adopt（本版明确不实现）。

### 5.3 可由工程决定的事

锁头与 state 的字段名/schema、`D_save` 与轮询退避、`checksum`/`revision` 算法、temp 命名、心跳/时间写入点、阶段名与文案、确定性发布顺序、进程内“已持有集合”的检查方式、测试夹具与故障注入点、诊断输出格式（不含敏感数据）。

### 5.4 建议后续任务顺序（依赖顺序，不是排期）

1. **v1 原语与状态机骨架**（独立任务）：实现 lock/state 打开与锁、init 同句柄锁定与多点身份复核、锁头只写一次、state 原子替换、身份/路径/校验、§2.4 判定、§2.9 操作前快照与恢复工作证据（含目录同步）；**不写目标文件**；跑 T1/T2a–T2e/T2f1/T2f2/T3/T4b–T4e/T5a–T5i/T5d1–T5d3/T10/T14。前置：本设计经审查。**已交付（TASK-0143，库与合成测试，未接线）**：覆盖 T1、T2d、T2e、T2f1、T2f2、T3、T4b（temp 写失败分支）、T4d、T5a–T5c、T5d1、T5f、T5g、T10、T14 的合成子集，以及 `admit` 操作前快照（§2.9 前半）。**未交付**：恢复工作证据（§2.9 后半，属第 6 项）、T2c/T4a/T4c/T4e/T5d2/T5d3/T5h/T5i/T6a/T6b/T7a/T7b/T9a/T9b/T11/T12/T13b、真实掉电/网络盘/整盘镜像。库**没有调用方**，所以“不写目标文件”目前靠未接线成立，不是运行时拒绝。
2. **旧锁行为修正**（独立任务，需行为变更审查）：checkpoint `Drop` 归属校验、取消 mtime 抢占、备份/回滚排除控制文件、`backup_save` 对特殊文件报错；不声称修复 #264。
3. **接入所有新版本写者**（依赖 1、2 与 OD-2）：跑 T6a；不改变业务语义。
4. **D 阶段机与备份验证**（依赖 3）：`admit`/`backup`/`candidate`，只到“候选已验证、未发布”；跑 T4a 发布前分支与 OD-7 确认的备份范围。
5. **发布、日志/完整性与幂等**（依赖 4 与 OD-9）：跑 T2a/T7a/T7b/T8；T6b 未接入前按“未接入”记录。
6. **显式恢复**（依赖 5）：跑 T9a/T9b；恢复任何目标写入前先建立工作证据（原 state/备份/intent/人工选择），再保护当前树、人工选择。
7. **#264 与跨平台/网络文件系统调查**（可与 1–6 并行，不能替代）：按[锁调查](CHECKPOINT-LOCK-INVESTIGATION.md) 的现场采集要求做，禁止公开私有路径。
8. **后续任务（不在 v1）**：复制/移动存档支持（`adopt`、副本独立分支、历史归档与恢复自动化）与 sidecar 入籍及 9/10→11 迁移；按[身份契约](ADVANCEMENT-IDENTITY-CONTRACT.md) §5 的顺序，不得以本设计代替身份验证。

首阶段全部验收后暂停，不进入全量专长自动化。

## 6. 证据与限制

TASK-0143 的九字段验收证据矩阵发布在 PR #283 的评论，`commit_sha` 绑定该 PR 的最终 head；本文件内不复制可能随 head 变化的运行结果。代码事实基线 `cc78760`（§1 的现有锁盘点）；实现基线 `30066e4`；若源码变化，§1 的定位与 §4 的「实现状态」需重新核对。

已知限制：设计部分只做静态阅读；实现部分只有合成测试，故障注入只覆盖可注入的窗口（路径替换、预置快照文件、只读目录/文件权限、超时）；未读取真实存档、书籍或秘密；未验证 Windows/macOS 文件锁、网络文件系统、目录/掉电持久性；预览未实现且一致性仍为 `unverified`；`init` 与状态机**已实现但未接线**，没有 CLI 入口；**Windows 本地提交不受支持**（无目录 sync：`init` 返回 `state_published_not_durable` 而非 `Ok`，`check_writable` 与 `begin_operation` 均拒绝）；**没有强制准入**：第二句柄自检只证明同一客户端的锁 API，网络盘/FUSE/同步目录既未识别也未拒绝；v1 无 `resume`/`abort`/`adopt`，失败后只能人工在协议外处理且必须先备份；v1 不保留永久历史，操作前 state 由受保护的操作证据保存（非永久），证据清理后的可恢复性不作保证、用户数据恢复依据是 D 备份；`revision` 只能在存在可验证先前值/可信锚时检出倒退，整体旧 state 重放可能不可检测（需独立锚，不在本任务）；`lock_identity` 不能识别保留标识的整盘镜像，Windows 上只是 `creation_time` 的尽力标识（size 与写时间会随写入头部变化，不能当标识）；未初始化副本与首次 init 不可区分；非 UTF-8 路径不能无损记录，因此这类存档不能初始化；`readdir` 迭代中途错误未注入（NOT_RUN）；Windows 上同长度改写保留 `creation_time`，`lock_identity` 无法发现；`publish_state` 删除 temp 失败只报告残留、不影响 state；每次更新前的复核与随后的 rename 之间仍有残余 TOCTOU；内层锁仍无界（§2.6、OD-9）；#264 根因仍 UNKNOWN；现有测试通过不构成锁安全证明。

回退：本文件为普通文档，可整体 revert；库的回退是 revert `tools/talkdnd/src/advancement_lock.rs`、其测试与 `lib.rs` 的模块注册，不触及存档、schema 或任何运行行为。
