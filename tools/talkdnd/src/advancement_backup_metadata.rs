//! Metadata captured before a D backup reads the source tree.

use std::fs::{self, File, FileTimes, Metadata};
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[cfg(not(target_os = "linux"))]
use anyhow::bail;
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};

use crate::advancement_commit::{is_root_excluded, posix_rel, BACKUP_MANIFEST};

#[derive(Deserialize, Serialize)]
pub(crate) struct Inventory {
    attribute_inspection: AttributeInspection,
    entries: Vec<Entry>,
}

#[derive(Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum AttributeInspection {
    LinuxProcessVisible,
    DarwinAclAndXattr,
}

#[derive(Deserialize, Serialize)]
struct Entry {
    path: String,
    directory: bool,
    accessed: Timestamp,
    modified: Timestamp,
    birth_time: Option<Timestamp>,
    posix: Posix,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
struct Timestamp {
    seconds: i64,
    nanoseconds: u32,
}

impl Timestamp {
    fn from_system_time(value: SystemTime) -> Result<Self> {
        let nanos = match value.duration_since(UNIX_EPOCH) {
            Ok(duration) => i128::try_from(duration.as_nanos())?,
            Err(error) => -i128::try_from(error.duration().as_nanos())?,
        };
        Ok(Self {
            seconds: i64::try_from(nanos.div_euclid(1_000_000_000))?,
            nanoseconds: u32::try_from(nanos.rem_euclid(1_000_000_000))?,
        })
    }

    fn system_time(self) -> Result<SystemTime> {
        let seconds = Duration::from_secs(self.seconds.unsigned_abs());
        let whole = if self.seconds.is_negative() {
            UNIX_EPOCH.checked_sub(seconds)
        } else {
            UNIX_EPOCH.checked_add(seconds)
        };
        whole
            .and_then(|time| time.checked_add(Duration::from_nanos(u64::from(self.nanoseconds))))
            .context("backup timestamp is outside the supported range")
    }
}

#[derive(Deserialize, PartialEq, Eq, Serialize)]
struct Posix {
    mode: u32,
    uid: u32,
    gid: u32,
    ctime: Timestamp,
    #[serde(skip)]
    device: u64,
    #[serde(skip)]
    inode: u64,
}

impl Posix {
    #[cfg(unix)]
    fn read(metadata: &Metadata) -> Result<Self> {
        use std::os::unix::fs::MetadataExt;
        Ok(Self {
            mode: metadata.mode() & 0o7777,
            uid: metadata.uid(),
            gid: metadata.gid(),
            ctime: Timestamp {
                seconds: metadata.ctime(),
                nanoseconds: u32::try_from(metadata.ctime_nsec())?,
            },
            device: metadata.dev(),
            inode: metadata.ino(),
        })
    }

    #[cfg(not(unix))]
    fn read(_metadata: &Metadata) -> Result<Self> {
        bail!("this platform has no verified backup metadata implementation")
    }
}

impl Entry {
    fn read(root: &Path, path: &Path) -> Result<Self> {
        let metadata = fs::symlink_metadata(path)?;
        ensure!(
            metadata.is_file() || metadata.is_dir(),
            "备份元数据检查拒绝符号链接或特殊文件"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            ensure!(
                !metadata.is_file() || metadata.nlink() == 1,
                "无法安全复制硬链接"
            );
        }
        inspect_attributes(path, &metadata)?;
        let birth_time = match metadata.created() {
            Ok(value) => Some(Timestamp::from_system_time(value)?),
            Err(error) if error.kind() == std::io::ErrorKind::Unsupported => None,
            Err(error) => return Err(error.into()),
        };
        Ok(Self {
            path: posix_rel(root, path)?,
            directory: metadata.is_dir(),
            accessed: Timestamp::from_system_time(metadata.accessed()?)?,
            modified: Timestamp::from_system_time(metadata.modified()?)?,
            birth_time,
            posix: Posix::read(&metadata)?,
        })
    }

    fn restore_copy(&self, path: &Path) -> Result<()> {
        #[cfg(unix)]
        use std::os::unix::fs::PermissionsExt;
        let before = Self::read(path, path)?;
        ensure!(before.directory == self.directory, "备份文件类型不匹配");
        ensure!(
            before.posix.uid == self.posix.uid && before.posix.gid == self.posix.gid,
            "无法保留备份所有者或组"
        );
        let file = open_copy(path)?;
        #[cfg(unix)]
        file.set_permissions(fs::Permissions::from_mode(self.posix.mode))?;
        let times = FileTimes::new()
            .set_accessed(self.accessed.system_time()?)
            .set_modified(self.modified.system_time()?);
        #[cfg(target_os = "macos")]
        let times = {
            use std::os::darwin::fs::FileTimesExt;
            match self.birth_time {
                Some(value) => times.set_created(value.system_time()?),
                None => times,
            }
        };
        file.set_times(times)?;
        file.sync_all()?;

        let actual = Self::read(path, path)?;
        ensure!(
            actual.posix.mode == self.posix.mode
                && actual.posix.uid == self.posix.uid
                && actual.posix.gid == self.posix.gid
                && actual.accessed == self.accessed
                && actual.modified == self.modified,
            "备份权限、所有者或时间戳校验失败"
        );
        #[cfg(target_os = "macos")]
        ensure!(actual.birth_time == self.birth_time, "备份创建时间校验失败");
        Ok(())
    }
}

impl Inventory {
    pub(crate) fn capture(root: &Path) -> Result<Self> {
        Self::capture_tree(root, false)
    }

    fn capture_tree(root: &Path, backup: bool) -> Result<Self> {
        let mut entries = Vec::new();
        let dev = Posix::read(&fs::symlink_metadata(root)?)?.device;
        collect(root, root, dev, backup, &mut entries)?;
        Ok(Self {
            attribute_inspection: if cfg!(target_os = "linux") {
                AttributeInspection::LinuxProcessVisible
            } else {
                AttributeInspection::DarwinAclAndXattr
            },
            entries,
        })
    }

    pub(crate) fn verify_backup(&self, root: &Path) -> Result<()> {
        let current = Self::capture_tree(root, true)?;
        ensure!(
            self.attribute_inspection == current.attribute_inspection
                && self.entries.len() == current.entries.len(),
            "备份元数据范围或检查平台与清单不一致"
        );
        for (expected, actual) in self.entries.iter().zip(current.entries) {
            // A copied inode has its own ctime, and ordinary reads can
            // change its atime. Compare the stable, recoverable attributes.
            ensure!(
                expected.path == actual.path
                    && expected.directory == actual.directory
                    && expected.posix.mode == actual.posix.mode
                    && expected.posix.uid == actual.posix.uid
                    && expected.posix.gid == actual.posix.gid
                    && expected.modified == actual.modified
                    && (!cfg!(target_os = "macos") || expected.birth_time == actual.birth_time),
                "备份条目元数据与清单不一致：{}",
                if expected.path.is_empty() {
                    "."
                } else {
                    &expected.path
                }
            );
        }
        Ok(())
    }

    pub(crate) fn verify_source(&self, root: &Path) -> Result<()> {
        let current = Self::capture(root)?;
        ensure!(
            self.entries.len() == current.entries.len(),
            "备份源树已变化"
        );
        for (before, after) in self.entries.iter().zip(current.entries) {
            // Reading the source may update atime. The pre-copy observation
            // is the timestamp written to and verified on the backup.
            ensure!(
                before.path == after.path
                    && before.directory == after.directory
                    && before.posix == after.posix
                    && before.modified == after.modified
                    && before.birth_time == after.birth_time,
                "备份源条目在复制期间变化：{}",
                before.path
            );
        }
        Ok(())
    }

    pub(crate) fn restore_and_verify_copy(&self, root: &Path) -> Result<()> {
        // Children precede their parents so final directory timestamps are
        // installed after walking their contents.
        for entry in self.entries.iter().rev() {
            let path = root.join(&entry.path);
            entry.restore_copy(&path).with_context(|| {
                format!(
                    "验收备份元数据 {}",
                    if entry.path.is_empty() {
                        "."
                    } else {
                        &entry.path
                    }
                )
            })?;
        }
        Ok(())
    }
}

fn collect(
    root: &Path,
    path: &Path,
    dev: u64,
    backup: bool,
    entries: &mut Vec<Entry>,
) -> Result<()> {
    let entry =
        Entry::read(root, path).with_context(|| format!("检查备份源元数据 {}", path.display()))?;
    ensure!(
        entry.posix.device == dev,
        "备份源条目跨文件系统：{}",
        entry.path
    );
    let directory = entry.directory;
    entries.push(entry);
    if directory {
        let mut children = fs::read_dir(path)?.collect::<std::io::Result<Vec<_>>>()?;
        children.sort_by_key(fs::DirEntry::file_name);
        for child in children {
            let name = child.file_name();
            if path == root && (is_root_excluded(&name) || (backup && name == BACKUP_MANIFEST)) {
                continue;
            }
            collect(root, &child.path(), dev, backup, entries)?;
        }
    }
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn open_copy(path: &Path) -> Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    Ok(fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn open_copy(_path: &Path) -> Result<File> {
    bail!("this platform has no verified metadata writer")
}

#[cfg(target_os = "linux")]
fn inspect_attributes(path: &Path, _metadata: &Metadata) -> Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let native = std::ffi::CString::new(path.as_os_str().as_bytes())?;
    // SAFETY: the input is NUL-terminated. The null output with size zero
    // asks the kernel for the size of the visible attribute name list.
    let result = unsafe { libc::llistxattr(native.as_ptr(), std::ptr::null_mut(), 0) };
    require_empty_attribute_list(result)
}

#[cfg(target_os = "macos")]
fn inspect_attributes(path: &Path, metadata: &Metadata) -> Result<()> {
    use std::os::darwin::fs::MetadataExt;
    use std::os::fd::AsRawFd;

    // Darwin's <sys/acl.h> declares acl_t as an opaque pointer and
    // ACL_TYPE_EXTENDED as an acl_type_t enum value.
    const ACL_TYPE_EXTENDED: libc::c_uint = 0x100;
    unsafe extern "C" {
        fn acl_get_fd_np(fd: libc::c_int, acl_type: libc::c_uint) -> *mut libc::c_void;
        fn acl_free(acl: *mut libc::c_void) -> libc::c_int;
    }

    ensure!(metadata.st_flags() == 0, "无法保留 macOS 文件标志");
    let file = open_copy(path)?;
    let fd = file.as_raw_fd();
    // SAFETY: file owns fd throughout both calls. The null output with size
    // zero asks the kernel for the size of the attribute name list.
    require_empty_attribute_list(unsafe { libc::flistxattr(fd, std::ptr::null_mut(), 0, 0) })?;
    let acl = unsafe { acl_get_fd_np(fd, ACL_TYPE_EXTENDED) };
    if acl.is_null() {
        // With a live descriptor, ENOENT means the file security record has
        // no ACL. Every other error makes the backup unverifiable.
        let error = std::io::Error::last_os_error();
        ensure!(
            error.raw_os_error() == Some(libc::ENOENT),
            "无法检查 macOS ACL：{error}"
        );
        return Ok(());
    }
    // SAFETY: a non-null return is storage owned by this call.
    unsafe { acl_free(acl) };
    bail!("存在当前备份无法保留的 ACL")
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn require_empty_attribute_list(result: libc::ssize_t) -> Result<()> {
    if result < 0 {
        return Err(std::io::Error::last_os_error()).context("无法检查 ACL 或扩展属性");
    }
    ensure!(result == 0, "存在当前备份无法保留的 ACL 或扩展属性");
    Ok(())
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn inspect_attributes(_path: &Path, _metadata: &Metadata) -> Result<()> {
    bail!("this platform has no verified ACL or extended attribute inspection")
}
