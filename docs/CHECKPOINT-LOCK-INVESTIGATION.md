# Checkpoint 锁异常事实调查

本报告只涉及合成 fixture；不修改恢复、锁或测试。调查基线 `46a2eda04c4bb3f35d9ab0d47c90587a34993653`。**首次失败的原因未知；后续通过不证明锁安全，也不证明真实存档不受影响。**

## 首次失败和证据

[#261 原调查记录](https://github.com/ctrlshiftaltdel/TalkDND/pull/261#issuecomment-6780981168) 报告完整 `talkdnd` 测试退出 101。首次完整日志仍可取得，其 SHA-256 为 `548ee32a8a8b54bb4c38445be9ae36e505259d76a4cc1b9555d6390c7f9ccbec`；本次重新计算匹配。日志中 checkpoint 套件 29 通过、1 失败，失败点为 `tests/checkpoint.rs:1145:10`，`after.try_lock().expect(...)` 输出 `a released integrity lock must be acquirable again: "WouldBlock"`。此前“持有期间其他句柄必须被拒绝”的断言未报错。首次运行期间的进程、描述符、锁所有者及调度时序未记录，不能事后从日志恢复。

原记录另报目标测试 20 次、完整 checkpoint 10 次及原始 main 对照均通过；这是历史报告，不是当前 head 的新运行。原记录指出另一个 Cargo 进程等待构建锁，不证明两个测试进程同时运行。#261 三平台 CI 通过也不解释首次失败。

## 测试和实现路径：静态事实

`tools/talkdnd/tests/checkpoint.rs` 的 `fixture("rollback-integrity-lock")` 使用临时目录、测试标签和进程 ID 构造合成存档；创建 snapshot 后注入一次提交 rename 失败。`restore_injecting_after_backup` 的钩子在备份之后打开并锁定完整性锁文件，把持锁 `File` 保存到 mutex；恢复失败后测试要求回滚成功。它另打开 probe，验证 try-lock 为 `WouldBlock`，随后丢弃 holder 和 probe，再打开 after 并非阻塞地取锁。失败发生在这最后一步，而不是回滚或第一个互斥检查。

`checkpoint.rs::remove_extras` 跳过 `.talkdnd.integrity.lock`，避免回滚期间删除锁文件并让不同 inode 各自持锁。完整性锁是 `combat-sim/src/integrity.rs` 的文件锁，不同于 checkpoint 的 `.talkdnd.lock` 文件创建式锁。完整 checkpoint 套件的 CLI 测试通过 `Command` 启动子进程；测试默认可并发。目标 fixture 标签独有，但其他两个测试复用 `stage-fail` 标签，这一独立测试隔离风险不等于已证明影响目标测试。静态阅读不能确定首次失败时是否有子进程持有该描述符。

## 本次运行

环境：Linux x86_64，内核 `6.12.115+deb13.1-amd64`，rustc `1.99.1 (58cf8f9c3 2026-10-13)`，cargo `1.99.1 (5f94df478 2026-08-27)`；独立 worktree，默认测试线程数。逐次串行执行，每个 worktree 同时只有一个测试进程。

| 命令 | 次数 | 每次退出码和结果 |
|---|---:|---|
| `cargo test --manifest-path tools/talkdnd/Cargo.toml --test checkpoint rollback_keeps_the_integrity_lock_a_concurrent_writer_holds` | 1 | 0；1 通过、29 过滤 |
| `cargo test --manifest-path tools/talkdnd/Cargo.toml --test checkpoint` | 1 | 0；30 通过 |

本次未复现，故没有失败现场可采集。本次运行次数不足以量化概率，也不覆盖 Windows/macOS 或真实存档。测试使用其自身合成 fixture，不使用玩家数据。

## 隔离假设实验

以下是本机临时合成文件和 Rust 标准库文件锁的机制实验，不是产品复现；命令和代码见下方。两个实验分别构建、执行一次，编译器和程序均退出 0。第二次编译产生两个 FFI 类型警告；以下列出实际签名，结果只作为本环境观察。

1. **重复句柄**：同一进程 `try_clone` 持锁句柄、丢弃原句柄，另开句柄尝试取得锁：仍 `WouldBlock`；丢弃 clone 后成功。输出 `duplicate_retains_lock=true`、`duplicate_closed_reacquire=true`。说明保留副本可产生相同表象，不证明测试存在泄漏。
2. **独立子进程**：子进程独立打开并持锁，经管道确认后父进程尝试：`WouldBlock`；子进程退出 0 后成功。输出 `independent_child_blocks=true`、`child_exit_reacquire=true`。说明独立竞争可产生同类现象；没有首次失败时的锁持有者证据。
3. **fork 期间继承**：父进程持锁后 fork，子进程保持继承的描述符并在管道上等待；父进程丢弃自己的句柄后仍 `WouldBlock`，释放子进程并 wait 后成功。输出 `fork_child_retains_fd=true parent_closed=true`、`child_wait_status=0 reacquire_after_child_exit=true`。这是有意延长的 fork 窗口实验，未验证 `Command` 实际使用的创建机制、exec 后继承、首次运行子进程时序或跨平台行为，不能声称首次故障由 fork 导致。

综合来看，句柄副本、继承窗口和独立写者都是可形成 `WouldBlock` 的机制；历史日志无法区分。并发 fixture 路径碰撞需另看具体标签和进程；目标标签与其他测试不同，不能用其他测试共享标签解释目标锁失败。也不能排除其他内核、运行时或测试交错原因。若真实恢复与并发完整性写入失去互斥或产生误导性恢复结果，可能损坏存档；本实验没有证明发生了这种产品缺陷，也没有排除它。后续应在**合成数据**上捕获失败当时锁 inode、进程树、各进程打开描述符及其生命周期，并比较默认并行与单线程；收集时不得公开私有路径或数据。任何修复须另立任务、通过安全决策与独立审查。

### 机制实验重现材料

本次命令依次为 `rustc /tmp/opencode/checkpoint-lock-synthetic.rs -o /tmp/opencode/checkpoint-lock-synthetic`、`/tmp/opencode/checkpoint-lock-synthetic run /tmp/opencode/checkpoint-lock-synthetic-file`、`rustc /tmp/opencode/checkpoint-lock-fork.rs -o /tmp/opencode/checkpoint-lock-fork`、`/tmp/opencode/checkpoint-lock-fork`；四个退出码均为 0。下列代码分别保存为上述源文件；仅 Linux fork 实验依赖 libc ABI。

```rust
use std::{env,fs::OpenOptions,process::{Command,Stdio},io::{BufRead,BufReader,Write}};
fn open(p:&str)->std::fs::File { OpenOptions::new().create(true).truncate(false).write(true).open(p).unwrap() }
fn blocked(f:&std::fs::File)->bool { matches!(f.try_lock(),Err(std::fs::TryLockError::WouldBlock)) }
fn main(){let a:Vec<String>=env::args().collect(); let p=&a[2]; if a[1]=="child" {let f=open(p); f.lock().unwrap(); println!("locked"); std::io::stdout().flush().unwrap(); let mut s=String::new(); std::io::stdin().read_line(&mut s).unwrap(); return}
 let f=open(p); f.lock().unwrap(); let dup=f.try_clone().unwrap(); drop(f); let q=open(p); println!("duplicate_retains_lock={}",blocked(&q)); drop(dup); println!("duplicate_closed_reacquire={}",q.try_lock().is_ok()); drop(q);
 let mut c=Command::new(env::current_exe().unwrap()).args(["child",p]).stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap(); let mut line=String::new(); BufReader::new(c.stdout.take().unwrap()).read_line(&mut line).unwrap(); assert_eq!(line,"locked\n"); let q=open(p); println!("independent_child_blocks={}",blocked(&q)); writeln!(c.stdin.take().unwrap(),"release").unwrap(); println!("child_exit={}",c.wait().unwrap()); println!("child_exit_reacquire={}",q.try_lock().is_ok()); }
```

```rust
use std::{fs::OpenOptions,os::fd::AsRawFd};
unsafe extern "C" { fn fork()->i32; fn pipe(fds:*mut i32)->i32; fn read(fd:i32,buf:*mut u8,n:usize)->isize; fn write(fd:i32,buf:*const u8,n:usize)->isize; fn close(fd:i32)->i32; fn waitpid(pid:i32,status:*mut i32,opt:i32)->i32; fn _exit(code:i32)->!; }
fn main(){let p="/tmp/opencode/checkpoint-lock-fork-file"; let open=||OpenOptions::new().create(true).truncate(false).write(true).open(p).unwrap(); let f=open(); f.lock().unwrap(); let mut fd=[0;2]; unsafe{assert_eq!(pipe(fd.as_mut_ptr()),0); let pid=fork(); assert!(pid>=0); if pid==0 {close(fd[1]); let mut b=0; read(fd[0],&mut b,1); _exit(0)} close(fd[0]); drop(f); let q=open(); println!("fork_child_retains_fd={} parent_closed=true",matches!(q.try_lock(),Err(std::fs::TryLockError::WouldBlock))); let b=1u8; assert_eq!(write(fd[1],&b,1),1); close(fd[1]); let mut status=0; assert_eq!(waitpid(pid,&mut status,0),pid); println!("child_wait_status={status} reacquire_after_child_exit={}",q.try_lock().is_ok()); println!("holder_raw_fd_before_fork={}",q.as_raw_fd()); }}
```
