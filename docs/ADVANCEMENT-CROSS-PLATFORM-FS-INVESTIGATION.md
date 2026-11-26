# 跨平台/网络文件系统调查（ADV-P1-13）

第 1–8 节保留 #313 的 Linux 调查快照。第 9 节补充 #328 的 Windows NTFS 实测与写入准入结论。

本报告只涉及合成临时实验目录，以及对已批准文档与产品代码的边界指针。不修改产品代码、测试或 CI；不开始 `ADV-P1-6`；不新增 schema 11 或选择 sidecar，也不发明 sidecar 字段或路径；不启用成长提交或恢复 CLI 入口；不改变旧完整性锁的阻塞 API；`CURRENT_SCHEMA` 保持 10。#264 结论不变，本报告不把 #264 写成已解决。

调查与仓库 head 绑定：`762313bc4d1498d3565974354076f31e5ea5b28f`（`origin/main`，[PR #312](https://github.com/ctrlshiftaltdel/TalkDND/pull/312) / `ADV-P1-5` 合并）。本报告不得被用来把其它块的验收标准判为通过，也不得替代 `ADV-P1-11` 的端到端与故障注入证据。不得据此扩展支持的平台或文件系统范围，不得承诺跨主机安全。通过或整洁的书面记录不是真实存档的安全证明，也不是开始 `ADV-P1-6` 的充分条件。

风格参照 [CHECKPOINT-LOCK-INVESTIGATION.md](CHECKPOINT-LOCK-INVESTIGATION.md)（事实、环境、已跑与未跑）；主题不同，不沿用其锁结论。已批准产品边界见 [ADVANCEMENT-LOCK-DESIGN.md](ADVANCEMENT-LOCK-DESIGN.md) 与 [ADVANCEMENT-SAFETY-AUDIT.md](ADVANCEMENT-SAFETY-AUDIT.md)，本报告只记录指针，不改写那些边界。

## 1. 环境、已运行与未运行

### 1.1 Linux 实测环境（绑定本 head）

实验于 2026-11-25 11:34（Asia/Shanghai，CST / UTC+8）在独立 scratch 目录 `/workspace/tmp/adv-p1-13-fs-exp` 执行一次；该目录位于与 `/workspace` 同一块本地盘上，用后已删除。未读取真实存档、书籍或秘密。

| 项 | 实测值 |
|---|---|
| 仓库 head | `762313bc4d1498d3565974354076f31e5ea5b28f` |
| OS | Debian GNU/Linux 13 (trixie) |
| 内核 | `Linux grok-bot-vm-418838437 6.12.113+ #1 SMP PREEMPT_DYNAMIC Mon Nov 23 16:05:51 UTC 2026 x86_64 GNU/Linux` |
| 实验文件系统 | `/workspace` → `/dev/vdb[/workspace]`，`ext4`，挂载选项 `rw,relatime`；`statfs` 报告 `Type: ext2/ext3`，`Namelen: 255`，块大小 4096 |
| 对照本地卷（仅用于跨设备硬链接） | `/tmp` 落在 `/` → `/dev/vdc`，同为本地 `ext4`（`rw,relatime`），设备号不同（`stat`：`/workspace` `dev=65040`，`/tmp` `dev=65056`） |
| 用户 | `uid=1000(box)` `gid=1000(box)`，无特权 |
| rustc | `1.100.0 (0ef1bc386 2026-11-09)`（`$HOME/.cargo/bin`） |
| Python | 3.13.5 |

未能读取 `/dev/vdb` 超级块（无权限），因此不能从超级块确认 `CASEFOLD` 等特性位；大小写结论只来自该挂载上的目录项实测。本结果只约束上述内核 + 该 `ext4` 挂载，不外推到 XFS、Btrfs、tmpfs、overlay、启用 casefold 的 ext4 或其他 Linux 文件系统（那些均为 `NOT_RUN`）。

### 1.2 已运行

- Linux、本工作区该 `ext4` 挂载：目录枚举、大小写、Unicode NFC/NFD、非 UTF-8 文件名、符号链接（含悬空与环）、硬链接、FIFO、Unix 套接字、备份相关元数据（权限、时间、user xattr、POSIX ACL）、Rust `std::fs` 的 `read_dir` / `symlink_metadata` / `metadata` / `fs::copy` / `fs::hard_link`、目录与文件 `sync_all`、两个本地 ext4 卷之间的硬链接。命令与输出摘要见第 7 节。
- 无特权 `mknod` 创建字符设备：已尝试，失败（见第 3.5 节）。对已存在的 `/dev/null` 做了一次参考 `lstat`（本地 `devtmpfs`），不是在实验树里创建设备，也不构成网络/FUSE 探测。

### 1.3 未运行（`NOT_RUN` / `UNKNOWN`）

- **Windows**：本工作区不能在 Windows 上执行。目录枚举、大小写、链接、特殊文件、备份元数据在 Windows 上均为 `NOT_RUN` / `UNKNOWN`。不得用 Linux 结果或“设计上应该如此”代替。既有产品边界（无目录 sync 则失败关闭）见第 2 节，那是已批准行为，不是本次 Windows 实测。
- **macOS**：同样未在对应环境实测，全部 `NOT_RUN` / `UNKNOWN`。
- **网络盘 / 同步目录 / FUSE**：项目权限**未授予**。未挂载、未接入、未探测任何未授权的网络/同步/FUSE 文件系统，也不以本地结果代替。环境的挂载表中可见 `TARGET=/cursor/stores FSTYPE=fuse.agent-store`；仅记录该名称，**未**打开、列举或探测其内容。
- **T11 / T12**（锁能力自检在网络盘/只读卷上的行为，以及跨主机/网络文件系统互斥）：`NOT_RUN` / `UNKNOWN`。
- **T5d3 / T13b**（整盘/同相对路径镜像与旧 state 重放是否可检测）：`NOT_RUN`。设计只声称“不承诺检测”；本次未做镜像或重放实验，不把该非承诺改写为已检测。
- `readdir` 迭代中途错误注入：`NOT_RUN`（与设计已声明的限制一致）。
- 掉电或真实崩溃后的目录项持久性：`NOT_RUN`。本次只观察到目录 `fsync`/`sync_all` 系统调用返回成功，不证明掉电后目录项一定持久。
- 其它 Linux 文件系统、跨主机、真实存档、特权设备节点创建：`NOT_RUN`。

## 2. 与本调查相关的既有产品边界（只引用）

以下是调查时源码与已批准文档中的既有边界，不是本块的新声明，也不在本块中扩展。

- **Windows 上目录同步不受支持，提交路径失败关闭。** `dir_sync_supported()` 为 `cfg!(unix)`；非 Unix 上 `sync_dir` 失败关闭。`init` 仍可能用硬链接发布 state（硬链接不可撤销），但返回 `StatePublishedNotDurable` 而不是 `Ok`。`AdvancementLock::check_writable` 与 `begin_operation` 均拒绝。因此 D / 发布 / 恢复路径在 Windows 上失败关闭；本平台也无法经发布路径构造 leftover `active` / `in_doubt`（`admit` 同样因目录 sync 失败关闭）。依据：`tools/advancement-lock/src/save_lock.rs` 模块注释与 `dir_sync_supported` / `check_writable`，以及 [ADVANCEMENT-LOCK-DESIGN.md](ADVANCEMENT-LOCK-DESIGN.md) §2.6「平台边界」、§6。
- **硬链接拒绝目前只在 Unix 上检查 `nlink > 1`。** D 备份路径 `tools/talkdnd/src/advancement_commit.rs` 的 `refuse_special_file` 仅 `#[cfg(unix)]` 读取 `MetadataExt::nlink()`。`migrate::copy_tree` 与 `checkpoint` 的 `copy_tree` 对 `DirEntry::file_type().is_file()` 为真的项直接 `fs::copy`，不检查 `nlink`。
- **符号链接与特殊文件：现有拷贝树失败关闭，不静默跳过。** `migrate::copy_tree`、`checkpoint` 的 `copy_tree`、以及 D 备份的 `copy_tree_at` 在条目不是普通文件或目录时报错中止（符号链接走 `is_symlink` 或 else 分支）。这与「不得跟随链接逃出存档根」的既有要求一致。
- **网络盘 / FUSE / 同步目录：不受支持且未验证，也没有强制准入门。** `save_lock.rs` 写明 “Network shares, FUSE and sync folders are unsupported and unverified. Nothing in this build detects them.” 设计 §2.6 / §2.8 与项目章程 OD-1 同一边界。第二句柄自检只证明同一客户端的锁 API。
- **整盘 / 同相对路径镜像与旧 state 重放：不承诺检测。** 设计 §2.5、§2.8 与计划中 T5d3/T13b：`lock_identity` 不能识别保留标识的整盘镜像；无可验证锚时整体替换为自洽旧 state 可能被当成合法。错误信息必须要求人工确认，不得写成必然拒绝或已经检出。

## 3. Linux 实测事实（本环境 / 本 head）

下列事实均来自第 7 节那一次运行。除非另注明，路径相对于当时的 scratch 目录。

### 3.1 目录枚举

- `find -P`、`ls -la`、Python `os.listdir` / `os.scandir`、Rust `std::fs::read_dir` 都能看到普通文件、子目录、点文件（`.hidden`）、含空格名称、UTF-8 中文名称、符号链接、硬链接的两个目录项、FIFO 与 Unix 套接字。
- GNU `ls -la` 显示 `.` 与 `..`。Python `os.listdir` 与 Rust `read_dir` **不**包含 `.` / `..`（本次 `read_dir_includes_dot_or_dotdot=false`，`read_dir_count=23`，该计数含随后加入的 `UniqueCase`，不含后来 Rust 程序写入的 `renamed-state`）。
- `DirEntry::file_type()`（不跟随）可以把符号链接标为 `is_symlink`，把 FIFO 标为 `is_fifo`，把套接字标为 `is_socket`，把硬链接项标为 `is_file`（硬链接是普通文件的第二个名字）。
- `Path::is_file()` **跟随**符号链接：对指向普通文件的 `link-to-regular` 为 `true`，对 `DirEntry::file_type()` 则是 `is_symlink`。若枚举或备份先调用 `Path::is_file()` / `fs::metadata` 而不是 `symlink_metadata` / `DirEntry::file_type()`，会把符号链接当成普通文件。
- 在一次 `os.scandir` 迭代已经取出第一项之后新建 `added-during-iter.txt`，该名称**没有**出现在该迭代器的剩余项中。这只是本环境一次观察，不是枚举稳定性保证；中途错误注入仍为 `NOT_RUN`。
- 本挂载允许非 UTF-8 文件名：以字节 `bad-\xff-name.txt` 创建成功；Rust `OsStr::to_str()` 对该目录项返回 `None`。这与设计「非 UTF-8 路径不能无损记录为 POSIX 字符串」一致，但不能把“本盘允许创建”写成产品已处理。

### 3.2 大小写与 Unicode 规范化

- 同一目录中 `Foo`（内容 `upper`）与 `foo`（内容 `lower`）同时存在，inode 不同。
- 仅存在 `UniqueCase` 时，打开 `uniquecase` 得到 `FileNotFoundError`。该挂载是**大小写敏感且保留大小写**的。
- NFC `café`（字节 `caf\xc3\xa9`，inode 2123478，内容 `nfc`）与 NFD `cafe\u0301`（字节 `cafe\xcc\x81`，inode 2123479，内容 `nfd`）是两个不同的目录项。
- 名称 `trailing-space.txt `（尾随空格）与 `trailing-dot.txt.`（尾随点）可以创建，且与去掉尾随字符的名称不是同一目录项。

### 3.3 符号链接

- GNU `stat` 默认不跟随（悬空链接 `tree/dangling -> missing-target` 可显示）；`stat -L` 跟随：对悬空链接报 `No such file or directory`，对 `loop-a`/`loop-b` 环报 `Too many levels of symbolic links`。
- Rust `fs::symlink_metadata` 与 `Path::is_symlink()` 看到链接本身；`fs::metadata` 与 `Path::is_file()` 跟随目标。环的 `metadata` 为 `os error 40`，悬空为 `os error 2`。
- `readlink`：`link-to-regular` → `regular.txt`；`dangling` → `missing-target`；绝对链接保留绝对目标路径。
- **`std::fs::copy` 跟随符号链接**：复制 `link-to-regular` 得到普通文件（`dst_via_symlink_is_symlink=false`，17 字节，新 inode）。复制悬空链接：`NotFound`。默认 `cp` 与 `shutil.copy2` 同样跟随；`cp -d` 把目标写成符号链接（`out/cp-d-symlink.txt -> regular.txt`）。
- 对 `ADV-P1-6` 的含义：若备份使用未先做 `lstat` / `file_type` 判断的 `fs::copy`，会跟随链接（可能离开存档根），并且不会把链接作为链接保存。既有 D/迁移/checkpoint 拷贝树先看 `file_type` 再拒绝符号链接，与本环境可区分“链接 vs 目标”的事实相符。本次未证明产品路径在所有调用点都使用不跟随的枚举。

### 3.4 硬链接

- `hard-a` 与 `hard-b` 是同一 inode（2123493），`nlink=2`，`file_type` 为普通文件，不是符号链接。
- `std::fs::copy` / 默认 `cp` / `shutil.copy2` 都把硬链接拆成独立副本：目标为新 inode、`nlink=1`。`fs::copy` 还不保留 mtime。`shutil.copy2` 保留了 mtime，但仍是 `nlink=1`。
- `cp -a hard-a hard-b` 到新目录时，目标侧共享一个**新** inode 且 `nlink=2`（源 inode 2123493，目标 inode 2123500）。这是 GNU `cp -a` 的链接保持行为，不是 `fs::copy` 的行为。
- `std::fs::hard_link` 在本 `ext4` 上成功；目标已存在时返回 `AlreadyExists`（`os error 17`）。这与首次发布 state「用硬链接、目标已存在即失败、绝不覆盖」的既有实现一致，但本实验只证明该系统调用在本盘上的语义，不是发布协议的新证明。
- 从 `/tmp`（`/dev/vdc`）硬链接到 `/workspace`（`/dev/vdb`）失败：`Invalid cross-device link`（`EXDEV`）。两个卷都是本地 ext4，不是网络盘实验。

对 `ADV-P1-6` 的含义：在本 Linux 环境可以用 `nlink > 1` 发现硬链接。`fs::copy` 不能安全地「按链接关系」复制，只会写成独立文件。D 备份已有的 Unix `nlink > 1` 失败关闭与这一事实相符。未在 Unix 上做同等检查的拷贝路径（既有 `migrate` / `checkpoint` 的 `copy_tree`）若遇到硬链接，按本次观察会静默拆链；本报告不修改那些路径，只记录事实。Windows 上是否暴露可比的链接计数仍为 `UNKNOWN`（现有拒绝本来就只在 Unix）。

### 3.5 特殊文件

- FIFO（`mkfifo`）与 Unix 套接字均可在实验树中创建；`lstat` / Rust `FileTypeExt` 分别报告 `is_fifo`、`is_socket`。二者的 `Path::is_file()` 均为 `false`，因此既有「`is_file` 否则失败关闭」的拷贝树在本环境会拒绝它们。
- 无特权 `mknod tree/fake-null c 1 3` 失败：`Operation not permitted`，节点未创建。参考 `lstat /dev/null`：`typ=character special file mode=666 nlink=1`（本地 `devtmpfs`）。未在实验树中观察到块设备或字符设备节点。
- 默认 `cp` 复制 FIFO：2 秒超时，退出码 124，目标未创建（打开 FIFO 读会阻塞）。`cp -a` 复制 FIFO 成功，目标仍是 FIFO。`cp` 复制套接字失败：`cannot open 'tree/unix.sock' for reading: No such device or address`，退出码 1。

对 `ADV-P1-6` 的含义：本环境可以在不跟随的前提下把 FIFO/套接字/设备类型与普通文件区分开。失败关闭（拒绝特殊文件，而不是默认 `cp` 那种可能阻塞或静默失败）与既有拷贝树策略相符，也与 `ADV-P1-6` AC-1「特殊文件无法安全复制时停止且目标未改」的实现前提相符——但仅就本 Linux `ext4` 而言。

### 3.6 备份元数据

源文件 `tree/regular.txt` 在实验中被设为模式 `0640`，user xattr `user.talkdnd-test=adv-p1-13`，并用 `setfacl -m u:box:r` 写入 POSIX ACL（`getfacl` 可见 `user:box:r--`，`listxattr` 同时看到 `user.talkdnd-test` 与 `system.posix_acl_access`）。

| 复制方式 | 模式 0640 | user xattr | POSIX ACL | mtime | 新 inode |
|---|---|---|---|---|---|
| `std::fs::copy` | 保留 | **不保留**（目标 `xattr=[]`） | **不保留**（getfacl 无 `user:box`） | **不保留** | 是 |
| 默认 `cp` | 保留 | **不保留** | **不保留** | **不保留** | 是 |
| `cp --preserve=all` | 保留 | 保留（值相同） | 保留 | 保留 | 是 |

Rust 标准库没有 xattr API；`fs::copy` 在本环境不是「完整元数据复制」。若源上存在 user xattr 或 POSIX ACL，仅用 `fs::copy` 得到的备份在本环境是元数据不完整的。`ADV-P1-6` AC-1 要求「元数据无法安全复制时停止且目标未改」：在本环境，这意味着实现不得把裸 `fs::copy` 的成功当成完整备份；若选择不实现已验证的保留路径，遇到无法完整复制的元数据时应失败关闭。本次**没有**把 `cp --preserve=all` 接入产品，也不把它写成已经验收的迁移拷贝实现。

目录与文件同步（本环境，系统调用成功，**不是**掉电证明）：

- `File::open("tree").sync_all()` → `ok`
- `dir.sync_data()` → `ok`
- 普通文件 `sync_all` → `ok`
- 写临时文件、`sync_all`、`rename`、再对父目录 `sync_all` → `ok`
- Python `os.fsync(dir_fd)`（`O_RDONLY|O_DIRECTORY`）→ `ok`

这与「Unix 上 `dir_sync_supported()` 为真，产品会尝试目录 sync」一致，只证明本挂载接受该系统调用。掉电后目录项是否持久仍为 `NOT_RUN`。

本环境 Rust `Metadata::created()` 与 GNU `stat` 的 `Birth` 字段有值；本次未验证该出生时间在何种替换下保持不变，也不把它当成跨平台身份。所有权未做 `chown` 实验（无特权）。

## 4. Windows 与 macOS（#313 快照，`NOT_RUN` / `UNKNOWN`）

#313 的调查环境仅有 Linux，未取得对应 Windows 或 macOS 的实测。下列条目为 #313 当时的 `NOT_RUN` / `UNKNOWN`。后续 #328 的 Windows 实测与仍未验证的项目见[第 9 节](#9-328-补充调查)。

- 目录枚举（含短名、大小写折叠、尾随点/空格、备用数据流或资源叉等）
- 大小写敏感性（常见 NTFS / APFS 默认与 Linux 不同，但未测）
- 符号链接、目录联接、硬链接、以及创建链接所需的权限或特权
- 特殊文件（Windows 上的设备名、macOS 别名等）
- 备份元数据（ADS、资源叉、Finder 信息、xattr、ACL、时间戳隧道）
- 目录 `fsync` 是否存在及其错误语义（产品已把非 Unix 的目录 sync 实现为失败关闭，见第 2 节；那是编译期能力门，不是本次对 NTFS/APFS 的实测）

既有产品在 Windows 上因无目录 sync 而对 D/发布/恢复/admit 失败关闭，这一边界仍然有效。它**不**填补上述未测语义，也**不**使 Windows 上的迁移备份实现被本报告放行。macOS 虽属 Unix（产品会认为目录 sync 可用），但本工作区没有 macOS 实测，不能声称枚举/大小写/链接/元数据已支持。

## 5. 网络盘 / 同步目录 / FUSE，以及 T11/T12、T5d3/T13b

- **T11 / T12**：需要项目权限下的网络挂载、只读卷或跨主机实验。本次**未获授权**，未运行，状态保持 `NOT_RUN` / `UNKNOWN`。本地第二句柄自检与本地 `flock` 观察（本次甚至没有重做锁实验）都不能证明跨客户端互斥。设计已把跨主机互斥标为 `UNKNOWN`，未证明前按「不支持」处理；本报告维持该非承诺。
- **T5d3 / T13b**：需要整盘/快照镜像与旧 state 重放实验（设计矩阵：人工/特权）。本次未做这类实验，状态保持 `NOT_RUN`。设计只声称「可能无法检测、不承诺检测、需人工确认」；本报告重申该非承诺，**不**写成已检测或必然拒绝。
- 挂载表中出现的 `fuse.agent-store`（`/cursor/stores`）不是获准实验对象，未探测。

## 6. 对 `ADV-P1-6` 的门结论

**结论：部分支持且需限定范围。不是全面支持。未测项不放宽。**

支持的部分（仅限第 1.1 节绑定的 Linux `ext4` 环境与本 head）：

- 可以在不跟随的前提下区分普通文件、目录、符号链接、硬链接（`nlink > 1`）、FIFO 与套接字。
- 目录 `sync_all` 系统调用被该挂载接受。
- 枚举是大小写敏感的；NFC/NFD 与非 UTF-8 名称都可能作为不同目录项存在，实现若不能无损表示路径，应失败关闭而不是改写名称。
- 这些事实**支持**一种在本环境失败关闭的迁移/备份实现，即：不跟随符号链接；拒绝特殊文件；拒绝 `nlink > 1`；不把裸 `fs::copy` 的成功当作 xattr/ACL/时间戳已完整复制；无法安全复制文件或元数据时停止且不改目标。这与 `ADV-P1-6` AC-1 的失败关闭要求一致，但只是实现前提，不是已交付的迁移。

限定与阻塞（`UNKNOWN` 按阻塞对应实现处理）：

- 不得开始面向 Windows / macOS / 网络盘 / 同步目录 / FUSE / 未测 Linux 文件系统的迁移实现，也不得把本报告当作那些平台「已支持」。
- 在事实缺失处，`ADV-P1-6` 必须保持未实现，或失败关闭并明确报告范围不足；不得为了放行而把未跑项写成通过。
- Windows 上既有「无目录 sync 则 admit/D/发布/恢复失败关闭」的边界继续适用，它限制的是提交协议能力，不是一份已测的 Windows 备份语义说明书。
- 不承诺跨主机安全，不扩展支持范围，不替代 `ADV-P1-11`，不把其它块的 AC 判为通过，不修改 #264 结论。
- 因此本门**不能**单独批准开始通用的 `ADV-P1-6` 写入实现。计划中 `ADV-P1-6` 的 AC-8（结论为 `UNKNOWN` 或与前提冲突时停在预览/设计，不开始写迁移）仍然适用：未测平台与未测文件系统上的迁移写入被阻塞；即使只讨论本 Linux 本地 `ext4`，也还需所有者已授权的身份方案等其它前置，且本报告不是迁移实现本身。

## 7. 实验命令与输出摘要

实验在工作树外的 scratch 目录 `/workspace/tmp/adv-p1-13-fs-exp` 进行，用后删除。下面是实际执行过的命令类别与忠实摘要；不是另一次补跑。

环境（节选）：

```
date: 2026-11-25T11:34:09+08:00
uname: Linux grok-bot-vm-418838437 6.12.113+ #1 SMP PREEMPT_DYNAMIC Mon Nov 23 16:05:51 UTC 2026 x86_64 GNU/Linux
os: Debian GNU/Linux 13 (trixie)
rustc: rustc 1.100.0 (0ef1bc386 2026-11-09)
python: Python 3.13.5
/workspace -> /dev/vdb[/workspace] ext4 rw,relatime
/tmp 落在 / -> /dev/vdc ext4 rw,relatime
```

构造（节选）：在 `tree/` 下创建普通文件（`chmod 640`）、`Foo`/`foo`、点文件、中文名、NFC/NFD、尾随空格与尾随点、子目录、相对/绝对/悬空/环状符号链接、硬链接对、`mkfifo`、Unix 套接字；`os.setxattr(..., "user.talkdnd-test", b"adv-p1-13")`；`setfacl -m u:box:r`。随后：

```
mknod tree/fake-null c 1 3
# mknod: tree/fake-null: Operation not permitted

stat -c 'path=%n typ=%F mode=%a nlink=%h dev=%d ino=%i' /dev/null
# path=/dev/null typ=character special file mode=666 nlink=1 dev=6 ino=4
```

大小写与规范化（节选）：

```
Foo content: upper
foo content: lower
same inode? False
open uniquecase: FileNotFoundError (case-sensitive)
nfc bytes b'caf\xc3\xa9'  nfd bytes b'cafe\xcc\x81'
nfc ino 2123478 nfd ino 2123479
nfc content nfc nfd content nfd
```

链接与特殊文件（GNU `stat` 默认不跟随；下方 `stat -L` 跟随。硬链接对 `ino=2123493 nlink=2`）：

```
tree/link-to-regular : 默认 typ=symbolic link；stat -L typ=regular file（目标 inode 2123473）
tree/dangling        : 默认 typ=symbolic link；stat -L : No such file or directory
tree/loop-a          : 默认 typ=symbolic link；stat -L : Too many levels of symbolic links
tree/named-pipe      : typ=fifo
tree/unix.sock       : typ=socket
```

复制语义（节选）：

```
cp tree/link-to-regular out/cp-default-symlink.txt
# 目标为普通文件，17 字节
cp -d tree/link-to-regular out/cp-d-symlink.txt
# lrwxrwxrwx ... out/cp-d-symlink.txt -> regular.txt
cp -a tree/hard-a tree/hard-b out/cpa/
# 目标共享新 inode、nlink=2（与源 inode 不同）
timeout 2 cp tree/named-pipe out/cp-fifo ; echo exit=$?
# exit=124，目标不存在
cp -a tree/named-pipe out/cp-a-fifo
# exit=0，目标仍是 fifo
cp tree/unix.sock out/cp-sock
# cp: cannot open 'tree/unix.sock' for reading: No such device or address  exit=1
```

元数据（节选）：

```
src xattr ['user.talkdnd-test', 'system.posix_acl_access']
cp dest xattr [] ; mtime_equal False ；ACL 中无 user:box
cp --preserve=all xattr 与 ACL 保留，mtime_equal True
ln /tmp/... /workspace/tmp/... : Invalid cross-device link  exit=1
python os.fsync(dir_fd)=ok
```

Rust（`rustc rust_fs.rs -o rust_fs` 退出 0，一次未使用函数警告；`./rust_fs` 退出 0）节选：

```
DirEntry file_type: link-to-regular typ=[is_symlink] 但 Path::is_file=true
named-pipe typ=[is_fifo] Path::is_file=false
unix.sock typ=[is_socket] Path::is_file=false
hard-b typ=[is_file] nlink=2
fs::copy(symlink) -> 普通文件 is_symlink=false
fs::copy(dangling) -> NotFound
fs::copy(hard-b) -> copy_hard_same_ino=false src_nlink=2 dst_nlink=1
fs::hard_link 成功；再次 hard_link 同一目标 -> AlreadyExists (os error 17)
dir_sync_all=ok  file_sync_all=ok  rename_then_parent_dir_sync_all=ok
fs::copy 目标 xattr=[] ，不保留 ACL，mtime 为新值，模式 0640 保留
```

非 UTF-8 名称（同一次实验目录上的追加观察，随后一并删除）：

```
created_non_utf8=True
non_utf8_dirent b'bad-\xff-name.txt'
Rust OsStr::to_str()=None
```

## 8. 本报告不是什么

- 不是 `ADV-P1-6` 的实现，也不是迁移预览或 sidecar 设计的完成。
- 不是三平台或网络文件系统的支持声明。
- 不是 `ADV-P1-11` 的端到端或故障注入证据，也不能把其它块的 AC 标为通过。
- 不是掉电、真实进程杀死、跨主机互斥或 #264 的结论更新。
- 不是真实存档上的安全证明。

## 9. #328 补充调查

2026-11-26，在 Windows x64 本机独立任务检出的合成临时目录运行。源代码基线为 `9c1896294575989fcb6de0e3af13bc3a8761e912`。系统报告 Windows 10.0.26100，PowerShell 7.6.9，.NET 10.0.14。实验目录所在卷由 `DriveInfo` 报告为 `Fixed`、`NTFS`。以下观察只适用于这个目录、此卷及所用 API。探针只访问合成数据。

### 9.1 Windows 本地 NTFS 观察

| 项目 | 合成方法与实际结果 | 证据边界 |
|---|---|---|
| 枚举 | `Get-ChildItem -Force` 与 `.NET Directory.EnumerateFileSystemEntries` 观察同一批 5 个条目。两者都看到隐藏文件、点开头名称、空格和非 ASCII 名称。枚举省略 `.` 和 `..`。 | 迭代中途 I/O 错误、并发改名、短名、尾随点和空格均为 `NOT_RUN`。 |
| 大小写与 Unicode | 用小写路径读到大写名称对应的原内容。原拼写保留，用 `FileMode.CreateNew` 再建同名小写项得到 `IOException`，HRESULT `0x80070050`。NFC 与 NFD 名称各自存在，内容可独立读取。 | 仅约束合成目录。启用区分大小写属性的 NTFS 目录为 `NOT_RUN`。 |
| 硬链接 | `New-Item -ItemType HardLink` 成功。写第二个名称后第一个名称读取到新字节，`fsutil hardlink list` 退出 0 并返回两个名称。普通 `File.Copy` 产生独立文件，随后修改源文件未改变副本。 | 当前 Windows 写入接口未因此取得安全复制硬链接的证据。 |
| 重解析点与符号链接 | 目录联接成功，枚举显示 `ReparsePoint`，`fsutil reparsepoint query` 退出 0。以联接路径读取子文件时跟随到了目标。创建文件符号链接和悬空链接均失败，文件链接重试返回原生错误 1314。 | 仅联接的识别及跟随已验证。符号链接可建时的枚举、悬空链接和环的语义仍为 `UNKNOWN`。 |
| 特殊文件 | 相对路径上的合成 `AF_UNIX` 本地套接字在绑定期间显示 `ReparsePoint` 且非容器，`fsutil reparsepoint query` 退出 0。 | 设备节点、命名管道及其他重解析标签为 `NOT_RUN`。枚举中的非容器也需要类型检查。 |
| 基础文件属性与时间戳 | 对源文件成功设置只读属性及创建、写入、访问时间，再以 `File.Copy` 复制。文件 SHA-256 相同，副本保留只读属性和最后写入时间，创建时间及最后访问时间不相同。 | 普通复制的成功不足以验证所要求的时间戳。Windows 创建时间与 POSIX 状态变更时间 `ctime` 的审计含义各自独立。 |
| ACL 与所有者 | 普通同用户副本的访问 DACL 和所有者读取一致。合成源文件的显式 ACL `Set-Acl` 返回拒绝访问，`icacls /grant` 重试退出 5，回读确认未改。 | 显式 DACL、SACL、其他所有者的复制与恢复为 `UNKNOWN`。未观察到显式 DACL 已设置的样本。 |
| ADS 与 EA/xattr | 命名数据流写入及回读成功。复制后枚举到同一流，流内容匹配。`fsutil file queryEA` 退出 0，当前工具的 `setEA` 返回参数无效并退出 1。 | ADS 的结果仅覆盖一个命名流。未建立带 NTFS EA 的正样本，EA 或 Unix xattr 的复制与恢复为 `UNKNOWN`。 |
| 复制失败与同步 | 对缺失源尝试覆盖合成目标时收到 `FileNotFoundException`，目标 SHA-256 相同。普通文件的 `FileStream.Flush(true)` 返回成功。目录以通用 `File.OpenHandle` 打开时收到拒绝访问（`0x80070005`），该分支未调用 flush。 | 只验证此复制失败的文件终态及指定 API 的返回。空间耗尽、并发替换、目录原生句柄 API、崩溃及掉电后持久性均为 `NOT_RUN`。 |

上述探针以 PowerShell 和 .NET 运行。主探针进程退出 0，分别记录预期拒绝的分项异常与原生退出码。补跑验证了文件符号链接错误码、显式 DACL 未改变、原生 EA 工具限制及相对路径套接字。典型输出如下。

```text
enumeration|0|count=5 names_found=True force_hidden=True dot=True spaces=True unicode=True dots=False
case_second_create|1|IOException|0x80070050
hardlink|0|type=HardLink source_changed=True fsutil_exit=0 fsutil_entry_count=2 copy_independent=True
junction|0|type=Junction reparse=True fsutil_exit=0 open_followed=True
file_link=1 native_error=1314
af_unix_bound=0 reparse=True container=False fsutil_exit=0
icacls_exit=5
explicit_acl_differs=False
copy_metadata|0|src_creation_requested=True src_write_requested=True src_access_requested=True creation_equal=False write_equal=True access_equal=False readonly_source=True readonly_copy=True dacl_equal=True owner_equal=True named_stream_listed=True named_stream_value_equal=True content_equal=True
copy_missing_source|0|rejected=True target_bytes_equal=True
flush_regular|0|flush_to_disk_returned=true
flush_directory_generic_handle|1|UnauthorizedAccessException|0x80070005
```

### 9.2 准入与备份结论

| 范围 | 支持的结论 |
|---|---|
| #313 的本地 Linux ext4 实测挂载 | 支持设计不跟随链接的枚举、普通文件和特殊文件识别、跨设备检查、元数据核对以及目录同步调用。实际部署仍需验证可准入根目录、完整备份和失败终态。其他 ext4 配置和其他 Linux 文件系统保持未测。 |
| #328 的 Windows NTFS 合成目录 | 支持在后续诊断中区分部分链接与重解析项，并证明普通复制遗漏了部分时间戳。Windows 成长提交受现有 Unix 目录同步能力门约束，继续失败关闭。schema 11 迁移当前尚未启用。 |
| macOS 或未测本地文件系统 | 此次枚举、大小写、链接与特殊文件、权限、ACL、xattr、可恢复时间戳和目录同步均为 `NOT_RUN`。对应产品准入保持 `UNKNOWN`。 |
| 网络盘、FUSE、同步目录 | 未授权探测，全部 `NOT_RUN`。固定卷、单次同步调用、同客户端锁自检、未见同步标记均不足以判断路径没有同步。无法可靠确认时，准入方案应拒绝写入。现有强制准入仍未实现。 |

依 [OD-1 和 OD-7](ADVANCEMENT-LOCK-DESIGN.md)，#328 仅补足部分事实。准入后还须验证用户资料及既有用户备份的全部字节和必需的可恢复元数据。锁与操作证据按既有约定单独保留，源 POSIX `ctime` 供审计。空间不足、目录枚举失败、链接或特殊文件、或任何必需元数据无法复制并校验时，应在写入目标前停止。真实存档、掉电重启、跨主机互斥与 #264 的根因仍没有新增证据。

### 9.3 独立准入任务建议

建议单独立项，只交付成长写入前的准入判定。待所有者明确首批支持的部署边界，再对已知本地平台、存档根及备份目标逐项核对实际文件系统、挂载和路径组成，拒绝链接、跨设备或无法可靠分类的路径。同步工具可能在普通本地目录上运行。专用非同步目录可作为经批准的部署前提，运行时仍须拒绝已知同步位置与无法解释的路径，并公开说明检测边界。

验收可用合成路径覆盖已准入根、未获准根、重解析点或链接、未知文件系统、权限与枚举故障，并对比拒绝前后用户目标、锁、临时目录与元数据的状态。检查必须在协议建立控制文件或准备写入目标之前发生。候选发布前应复核可变化的路径和卷身份。迁移接线、版本切换和故障恢复在各自验收中单独取证。
