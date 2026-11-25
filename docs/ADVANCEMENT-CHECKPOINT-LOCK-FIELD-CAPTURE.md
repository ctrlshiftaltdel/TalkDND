# ADV-P1-12：#264 失败现场采集

本报告只涉及合成 fixture 与公开测试输出；不修改产品代码或测试，不读取真实存档、书籍或秘密。调查时工作区 HEAD 为基线 `328a857f7d8a976697e600962fb2ebbb2f5f73e0`（`origin/main`，PR #314 合并）。本 PR 只在该基线之上增加文档。

**本次已复现**目标测试 `rollback_keeps_the_integrity_lock_a_concurrent_writer_holds` 在回滚成功、持有期间互斥检查通过之后，`after.try_lock()` 得到 `WouldBlock`（与 [#261 首次失败](https://github.com/ctrlshiftaltdel/TalkDND/pull/261#issuecomment-6780981168) 同一断言，当前行号 `tools/talkdnd/tests/checkpoint.rs:1249`）。已采集锁 inode、进程树与相关描述符，并比较默认并行与 `--test-threads=1`。

**这不是“#264 已解决”。** 失败瞬间的持锁者没有被捕捉到，因此不能把本次区分为“测试缺陷”或“产品锁安全影响”，该项保持 `UNKNOWN`。不得用本次后续通过、也不得用本次不完整现场反推 2026 年首次失败的根因。不得用本报告把其他块的 AC 判为通过，也不得关闭 #264。

既有报告 [CHECKPOINT-LOCK-INVESTIGATION.md](CHECKPOINT-LOCK-INVESTIGATION.md) 在更早 head（`46a2eda`）与 rustc 1.99.1 上只跑了 1+1 次且未复现，其“首次原因未知 / 后续通过不证明锁安全”的结论仍然成立，本文件不删除或改写该结论。

## 环境

| 项 | 值 |
|---|---|
| OS | Debian GNU/Linux 13 (trixie) |
| 内核 | `6.12.113+`（x86_64） |
| rustc | `1.100.0 (0ef1bc386 2026-11-09)`（rustup stable，`$HOME/.cargo/bin` 优先） |
| cargo | `1.100.0 (3acc79c0c 2026-11-05)` |
| 测试线程默认 | 本机 `nproc=8`（cargo 测试二进制默认并行度） |
| fixture 文件系统 | `std::env::temp_dir()`（本环境为 `/tmp`）位于 **ext4**（块设备 major:minor 在 `/proc/locks` 中记为 `fe:20`） |
| 源码工作区文件系统 | `/workspace` 亦为 ext4，但是另一块设备；测试不把 fixture 写到工作区 |
| 工作区 HEAD（调查时） | `328a857f7d8a976697e600962fb2ebbb2f5f73e0` |
| 时区 | Asia/Shanghai（CST，UTC+8） |

Windows / macOS 本工作区未跑，标 `NOT_RUN` / `UNKNOWN`，不得以 Linux 结果代替。未使用真实存档。

## 当前测试路径（静态事实，未改代码）

目标测试：`tools/talkdnd/tests/checkpoint.rs` 的 `rollback_keeps_the_integrity_lock_a_concurrent_writer_holds`。合成 fixture 标签为 `rollback-integrity-lock`，目录名含测试**进程** PID（同一 checkpoint 二进制内的并发测试共用该 PID，但标签不同，因此目录不同）。

测试在备份之后的钩子里打开并 `lock()` `.talkdnd.integrity.lock`，把持锁 `File` 存入 mutex；注入提交 rename 失败以走回滚。回滚成功后：

1. 另开 probe，断言 `try_lock()` 为 `WouldBlock`（持有期间互斥）；这两次失败都没有在此处崩溃，因此该检查在失败路径上通过。
2. `drop` 持有者与 probe。
3. 再打开 `after` 并 `try_lock()`，断言锁可再次取得。失败点是这里：`tests/checkpoint.rs:1249`，消息为 `a released integrity lock must be acquirable again: "WouldBlock"`。

历史首次失败记录的行号是 `tests/checkpoint.rs:1145`（当时 checkpoint 套件为 30 项）。当前 head 同一断言在第 1249 行，套件为 33 项。产品侧 `checkpoint::remove_extras` 在存档根跳过完整性锁文件；`restore_rebaseline` 会调用 `integrity::lock`，但本测试走的是备份后钩子 + 提交失败回滚，不是 rebaseline 成功路径。完整性锁 API 仍是 `std::fs::File::{lock, try_lock}`（与 `tools/combat-sim/src/integrity.rs` 相同）。本块未改这些代码。

套件中会启动子进程的测试是通过 `Command` 调用 `talkdnd` CLI 的路径（如 `cli_snapshot_list_show_restore_flow`）；`concurrent_snapshots_exactly_one_wins` 只起线程不起子进程。这些事实只说明默认并行下存在重叠窗口，不单独构成根因证明。

## 采集方法

只使用合成 fixture。未改测试或产品代码。官方运行一律为工作区内的 `cargo test --manifest-path tools/talkdnd/Cargo.toml …`，`CARGO_TARGET_DIR` 指向临时目录（用后删除）。

为在失败时尽量保住现场：把 cargo 放在伪终端下跑，监视输出中的 `test rollback_keeps_the_integrity_lock_a_concurrent_writer_holds ... FAILED`（该行在测试结束时立即出现，往往早于延后打印的 panic 正文）。一出现就对进程组 `SIGSTOP`，采集锁文件元数据（inode / dev / nlink / size，**不含内容**）、`/proc/locks` 中与该 inode 匹配的行、进程树、相关进程的 `/proc/<pid>/fd` 与 `fdinfo` 中的 `pos/flags/ino/lock` 行，然后 `SIGCONT`。并行用约 2–3 ms 间隔扫描打开了该 inode 的描述符。证据中的绝对路径在本报告里改写为 `<temp>/talkdnd-cp-<tag>-<pid>/…` 与 `<target>/…/checkpoint-<hash>`，不收录存档内容或主机名。

限制：`FAILED` 行出现时，失败测试的局部变量已经在 panic 展开中被 drop，因此这是“测试刚失败、同一二进制仍在跑其他测试”的窗口，不是 `try_lock` 返回那一指令的瞬时快照。打开某 inode 的描述符也不等于该描述符持有 advisory 锁。

## 运行次数与结果

下列次数包含正式采集循环；另注明循环开始前的计时运行（同样命令、无采集包装，结果计入观察，但不与采集现场混写）。

| 命令（工作区根目录，已设 `CARGO_TARGET_DIR`） | 次数 | 结果 |
|---|---:|---|
| `cargo test --manifest-path tools/talkdnd/Cargo.toml --test checkpoint rollback_keeps_the_integrity_lock_a_concurrent_writer_holds -- --exact` | 80 | 80 通过（约 0.08 s/次） |
| 同上，另加 `--test-threads=1` | 80 | 80 通过（约 0.08 s/次） |
| `cargo test --manifest-path tools/talkdnd/Cargo.toml --test checkpoint`（默认并行） | 41 | **2 失败、39 通过**（失败见下；通过约 0.14–0.17 s/次） |
| 同上，加 `-- --test-threads=1` | 40 | 40 通过（约 0.46 s/次） |
| `cargo test --manifest-path tools/talkdnd/Cargo.toml`（默认并行，完整 talkdnd crate） | 5 | 5 通过（约 26 s/次） |
| 同上，加 `-- --test-threads=1` | 3 | 3 通过（约 74 s/次） |

循环前计时（均通过，未做现场包装）：目标测试 `--exact` 1 次；完整 checkpoint 默认并行 1 次、`--test-threads=1` 1 次；完整 crate 默认并行 1 次。若把计时并入观察，则完整 checkpoint 默认并行为 **2 失败 / 42 次**，`--test-threads=1` 为 **0 失败 / 41 次**；完整 crate 默认并行为 **0 失败 / 6 次**。这不是发生率的统计估计，只是本次实际执行记录。

两次失败都在**默认并行的完整 checkpoint 套件**中，断言与历史首次失败相同。单独跑目标测试 160 次、以及 `--test-threads=1` 的 40 次套件均未失败。完整 crate（首次失败当年的命令形态）在本次 5+3（加计时则 6+3）次中未再出现。不能把“单独跑或单线程未失败”写成根因，也不能写成已修复。

## 两次失败现场

两次失败时间均为 2026-11-25，Asia/Shanghai。panic 线程名均为目标测试本身；消息均为 `a released integrity lock must be acquirable again: "WouldBlock"`，位置 `tests/checkpoint.rs:1249:10`。套件结果均为 32 通过、1 失败。完成顺序上，两次都是目标测试报 `FAILED` 之后，仍有 `staging_abort_removes_the_directories_staging_created`、`staging_failure_leaves_the_save_tree_unchanged`、`same_second_restore_rolls_back_to_this_operation_not_the_earlier_backup` 打印 `ok`，说明默认并行下其他测试与失败窗口重叠。`cli_snapshot_list_show_restore_flow` 与 `concurrent_snapshots_exactly_one_wins` 在两次日志里都出现在 `FAILED` 行之前，只表明它们已结束打印，不能证明失败瞬间没有重叠过。

### 失败 1（12:08:41.922）

命令：`cargo test --manifest-path tools/talkdnd/Cargo.toml --test checkpoint`（默认并行）。当时监视的是后打印的 panic 正文，触发偏晚：cargo 已被 `SIGSTOP`，测试二进制 `checkpoint-*` 已是僵尸进程，描述符已不可用。`/proc/locks` 中没有与该完整性锁 inode 匹配的行。

残留合成 fixture（因断言失败而未执行测试末尾的删除）中，锁文件元数据为：inode `548828`，`dev=65056`（`fe:20`），mode `100644`，`nlink=1`，size `0`。未读取、不收录任何存档内容。该残留目录在第二次失败采集时仍在磁盘上，见下节说明。

### 失败 2（12:11:40.307）——测试二进制仍存活

命令相同（默认并行 checkpoint 套件的下一轮循环第 1 次）。在 `… FAILED` 行出现时 `SIGSTOP`：cargo PID 479900 与测试二进制 PID 479912 均处于停止态（`T`），测试进程 **4 个线程**，**没有子进程**。

当前这次失败的锁文件（标签 `rollback-integrity-lock`，目录名含 PID 479912）：inode `548965`，`dev=65056`（`fe:20`），mode `100644`，`nlink=1`，size `0`。采集瞬间：全机 `/proc/*/fd` 中**没有**仍打开该 inode 的描述符；`/proc/locks` 中**没有**匹配 `fe:20:548965` 或 inode `548965` 的行。测试进程当时打开的描述符属于**其他仍在运行的测试**的合成目录（名称标签为 `backup-reuse` 与 `stage-dirs`，以及一个 `.talkdnd.lock`），没有指向本次完整性锁文件。

失败前约 47 ms 的采样（12:11:40.242–40.289）曾看到测试进程 PID 479912 的 `fd=6` 打开本次完整性锁文件（inode `548965`）。这些采样的 `fdinfo` 摘要里没有 `lock:` 行；采样也没有同步记录到 `/proc/locks` 的匹配行。12:11:40.304 及之后的采样里，该 inode 已无打开描述符。因此可以确认“失败前后该锁文件存在、测试进程一度打开过它、采集瞬间锁与描述符均已释放”，但**不能**据此指出 `try_lock` 返回 `WouldBlock` 时的持锁者，也不能区分句柄副本、已退出的子进程、同进程其他线程，或尚未观察的其他机制。

现场还列出了 inode `548828` 的锁文件：那是失败 1 留下的、未清理的同一标签 fixture，不是 PID 479912 的第二个写者，也没有打开描述符或 `/proc/locks` 匹配行。它污染了第二次的目录枚举，但不改变 inode `548965` 的上述观察。

## 默认并行与单线程对照

| 制度 | 观察 |
|---|---|
| 默认 cargo 并行（完整 checkpoint 套件，测试二进制内多线程） | 本次 41 次中 2 次复现同一 `WouldBlock` |
| `--test-threads=1`（同一套件） | 本次 40 次未复现 |
| 只跑目标测试（`--exact`，无论是否 `--test-threads=1`） | 本次各 80 次未复现 |
| 完整 talkdnd crate 默认并行 / `--test-threads=1` | 本次 5 次与 3 次未复现 |

对照只说明：在本 Linux 环境、本 head 上，复现出现在默认并行的 checkpoint 套件，而没有出现在本次单线程套件或单独目标测试中。这**不足以**把根因写成“并行测试污染”或排除产品路径。完整 crate 本次未复现，也不解释 2026 年那一次完整 crate 失败。

## 测试缺陷与产品锁安全影响

**判定：`UNKNOWN`。**

本次没有可复核的证据指出失败瞬间谁仍持有锁，因此不能在“测试缺陷”与“产品锁安全影响”之间作选择。后续通过（包括单线程全部通过）不反推根因，也不证明锁安全。若真实恢复路径与并发完整性写入失去互斥，仍可能影响存档；本采集既没有证明发生了这种产品缺陷，也没有排除它。任何产品修复须另立任务，并遵守安全决策门，不得削弱该测试。

## 范围限制

- 仅 Linux 本环境；Windows / macOS 为 `NOT_RUN` / `UNKNOWN`。
- 仅合成 fixture，不是真实存档。
- 运行次数不是概率估计。
- 失败瞬间持锁者未捕捉到；打开描述符的采样不等于 `/proc/locks` 意义上的持锁。
- 不得将本块结论用于其他块 AC，不得表述为“#264 已解决”。

## 未做的事

未修改 `tools/`、测试、CI、crate 版本或 CHANGELOG 功能条目。未开始 `ADV-P1-6`，未新增 schema 11，未发明 sidecar 字段或路径，未启用成长提交 CLI，未改变旧完整性锁的阻塞 API。`CURRENT_SCHEMA` 保持 10。临时实验目录与目标目录在提交后删除，不进入版本库。
