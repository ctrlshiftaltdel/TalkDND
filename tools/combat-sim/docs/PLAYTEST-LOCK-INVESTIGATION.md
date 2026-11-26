# macOS 游玩反馈队列的间歇锁失败

GitHub Actions 的 macOS 完整套件多次在 `a_lost_response_reconciles_across_restarts_without_reposting` 返回 `WouldBlock`。该测试用独占的合成临时目录与模拟远端，复现的是测试调用中的锁失败。持锁者与生产环境复现仍未知。[调查 #343](https://github.com/ctrlshiftaltdel/TalkDND/issues/343)

## 原始作业证据

表中按 `Test combat-sim` 步骤计数。样本有意选取已知失败、同一提交的重跑和邻近通过作业。在下面 12 个作业中，目标测试 6 次失败、6 次通过。这一计数只描述所列样本。失败步骤的退出码均为 101，成功步骤的退出码为 0。SHA 取 GitHub Actions 的 `head_sha`。

| job／触发事件 | head SHA | 目标测试和步骤结果 |
| --- | --- | --- |
| [139786894728／main push，首次](https://github.com/ctrlshiftaltdel/TalkDND/actions/runs/45784244909/job/139786894728) | `674b0027fb7a9fd35cc72862b05cb12a40467bcb` | `WouldBlock`，第 169 行，退出 101 |
| [139793565934／main push，重跑](https://github.com/ctrlshiftaltdel/TalkDND/actions/runs/45784244909/job/139793565934) | `674b0027fb7a9fd35cc72862b05cb12a40467bcb` | 通过，退出 0 |
| [139796230544／分支 push](https://github.com/ctrlshiftaltdel/TalkDND/actions/runs/45786971194/job/139796230544) | `9c689c966f9e8547c09c92f3e5a2edeebc732ed8` | `WouldBlock`，第 183 行，退出 101 |
| [139993537092／PR #342，首次](https://github.com/ctrlshiftaltdel/TalkDND/actions/runs/45844218372/job/139993537092) | `a110d12c780ef1a162cd95a8f1105146891b0fee` | `WouldBlock`，第 183 行，退出 101 |
| [140001058146／PR #342，重跑](https://github.com/ctrlshiftaltdel/TalkDND/actions/runs/45844218372/job/140001058146) | `a110d12c780ef1a162cd95a8f1105146891b0fee` | 通过，退出 0 |
| [139993509306／同 head 分支 push](https://github.com/ctrlshiftaltdel/TalkDND/actions/runs/45844210061/job/139993509306) | `a110d12c780ef1a162cd95a8f1105146891b0fee` | 通过，退出 0 |
| [140003866529／#342 合入后 main push](https://github.com/ctrlshiftaltdel/TalkDND/actions/runs/45847207425/job/140003866529) | `419e38717f9ef46f0c09d9a7685407275ff65fd6` | 通过，退出 0 |
| [140201030100／#346 分支 push](https://github.com/ctrlshiftaltdel/TalkDND/actions/runs/45905443179/job/140201030100) | `a5097ea528f8cc2c542db75087ea911ee9837909` | `WouldBlock`，第 183 行，退出 101 |
| [140201085876／PR #346，首次](https://github.com/ctrlshiftaltdel/TalkDND/actions/runs/45905451766/job/140201085876) | `a5097ea528f8cc2c542db75087ea911ee9837909` | 通过，退出 0。下一步骤的 `talkdnd-dev` `ENOENT` 是另一失败 |
| [140203338657／PR #346，重跑](https://github.com/ctrlshiftaltdel/TalkDND/actions/runs/45905451766/job/140203338657) | `a5097ea528f8cc2c542db75087ea911ee9837909` | `WouldBlock`，第 183 行，退出 101 |
| [140207055895／PR #346，后续 head 首次](https://github.com/ctrlshiftaltdel/TalkDND/actions/runs/45907307590/job/140207055895) | `eb51dfdc8af72a67b9e03fc7571943bd4f06422a` | `WouldBlock`，第 183 行，退出 101 |
| [140207097473／后续同 head 分支 push](https://github.com/ctrlshiftaltdel/TalkDND/actions/runs/45907302891/job/140207097473) | `eb51dfdc8af72a67b9e03fc7571943bd4f06422a` | 通过，退出 0 |

## 同一 macOS 作业的定向对照

两个调查提交临时追加了 CI 采样和失败回溯。每个采样作业在同一 VM、工具链和提交上先执行原有默认并行套件，然后在独立的测试进程逐次运行定向用例。第二个作业还重复运行整个默认并行的库单元测试进程。运行过的测试命令如下。[首次采样提交](https://github.com/ctrlshiftaltdel/TalkDND/commit/7f5e2241bacf42d7636f239aa2c7487fac78aa3b) · [重复对照提交](https://github.com/ctrlshiftaltdel/TalkDND/commit/f602b2ce2b448f17aa61c14cc58d744119ba1aff)

```sh
cargo test --manifest-path tools/combat-sim/Cargo.toml
cargo test --manifest-path tools/combat-sim/Cargo.toml --locked --lib playtest::tests::a_lost_response_reconciles_across_restarts_without_reposting -- --exact --test-threads=1
cargo test --manifest-path tools/combat-sim/Cargo.toml --locked --lib
```

| macOS 作业 | 系统与 Rust | 完整套件 | 独立定向用例 | 默认并行库测试 |
| --- | --- | --- | --- | --- |
| [首次作业](https://github.com/ctrlshiftaltdel/TalkDND/actions/runs/45907817609/job/140208691203) | macOS 15.8.2，Rust 1.100.0 | 1 次通过，退出 0 | 12 次通过，每次退出 0 | 未额外采样 |
| [重复对照作业](https://github.com/ctrlshiftaltdel/TalkDND/actions/runs/45908376966/job/140210518272) | macOS 15.8.2，Rust 1.100.0 | 1 次通过，退出 0 | 12 次通过，每次退出 0 | 23 次通过，第 8 次退出 101，采样步骤最终退出 1 |

第二个作业的并行库样本 8 有 78 项通过、1 项失败。失败用例在 `tests.rs:183:39` 对 `deliver` 的返回值执行 `unwrap()`，此时已读到 `Delivery::Linked(86)`。`WouldBlock` 的回溯经过 `playtest.rs:323:48` 和 `playtest.rs:501:25`，将该次失败定位到只尝试一次的 `worker.lock` 获取。[失败现场](https://github.com/ctrlshiftaltdel/TalkDND/actions/runs/45908376966/job/140210518272) · [获取锁的代码](https://github.com/ctrlshiftaltdel/TalkDND/blob/f602b2ce2b448f17aa61c14cc58d744119ba1aff/tools/combat-sim/src/playtest.rs#L499)

这次回溯记录了请求方和锁名。持锁 PID、线程、文件描述符的实际释放时点及是否与同进程并行用例重叠，仍为 `UNKNOWN`。一次默认并行失败及零次单例失败无法证明并行是必要条件，也无法判断两种方式的故障发生频率。

## 锁生命周期与影响

- 每个测试 fixture 用进程 ID 和随机后缀创建自己的临时目录，锁文件在各自队列下。目标用例用同一 fixture 顺序调用 `deliver`，以重新打开队列模拟重启。它使用内存中的 `Fake`，测试函数自身没有启动后台工作者。完整 libtest 中另外的用例会创建线程或启动进程，旧日志没有记录它们与失败时点的重叠。[fixture 和测试](../src/playtest/tests.rs)
- `deliver` 持有 `worker.lock` 直到调用退出，首次冲突立即返回。它另取 `record.lock` 读取队列快照，结束读取块后释放。更新状态时再取 `record.lock`，持到写入完成和 `Queue::update` 退出。`record.lock` 最多尝试五次，间隔 2 ms。代码的词法作用域界定了预期析构点，作业日志未记录实际关闭及持锁进程。[队列与交付实现](../src/playtest.rs)
- 第 169 行的失败发生在第一次提交结果不确定之后。第 183 行的五次失败都发生在测试已经读到 `Delivery::Linked(86)` 之后。此时 `deliver` 会先取 `worker.lock`，再取 `record.lock`，读取完毕后跳过已关联的条目。仅凭这些旧错误无法区分前两把锁。[测试断言](../src/playtest/tests.rs)
- 运行时同类冲突可能让本次后台交付提前返回。提交前会把 `Uncertain` 持久化，后续只查询该指纹，已关联条目会被跳过。此次测试失败没有观察到产品重复上报、队列丢失，或真实用户的后台交付卡住。游玩反馈是 #322 的 opt-in 路径，checkpoint 锁由 #264 跟踪。[反馈试点 #322](https://github.com/ctrlshiftaltdel/TalkDND/issues/322) · [checkpoint #264](https://github.com/ctrlshiftaltdel/TalkDND/issues/264)

## 后续取证与验收

下一次复现从已定位的 `worker.lock` 入手。用合成 fixture 在 macOS 记录该锁的设备和 inode、取得与释放时间、持锁进程与线程，以及并行测试的进程创建时点。当前没有证据证明先前调用留下了文件描述符。旧样本仍只有通用的锁拒绝信息。

定位后以单一任务验证修复。现有两项行为都要保持，即一个队列同一时刻只有一个工作者可以上报，记录路径仍可在上报期间写入。远端创建响应不明时，重开同一队列后只查询，已标记关联的条目不会重新提交。验收需比较 macOS 定向重复和默认并行全套，并用合成的真正多进程竞争确认失败诊断；运行时影响在取得真实持锁和交付证据前继续标记未知。[重启、并发和去重测试](../src/playtest/tests.rs)
