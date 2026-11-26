//! Read-only admission for the first supported advancement write environment.

use std::path::Path;

/// Roots the deployment has explicitly established as local and not synchronized.
/// Known synchronized paths may be supplied even when nested under either root.
pub struct ApprovedRoots<'a> {
    pub saves: &'a Path,
    pub backups: &'a Path,
    pub synchronized: &'a [&'a Path],
}

#[derive(Debug, PartialEq, Eq)]
pub enum Admission {
    Admitted,
    Unsupported(Reason),
    Unknown(Reason),
}

#[derive(Debug, PartialEq, Eq)]
pub enum Reason {
    Platform,
    OutsideApprovedRoots,
    Synchronized,
    LinkOrSpecialFile,
    NestedMount,
    Filesystem,
    ReadOnlyMount,
    Path,
    MountIdentity,
}

/// Inspect existing directories and the save tree without opening control files.
/// Use the existing trusted ancestor when the backup directory has not been created.
/// A successful result is an observation, so the writer must recheck before publish.
pub fn inspect(save: &Path, backup_parent: &Path, roots: &ApprovedRoots<'_>) -> Admission {
    #[cfg(target_os = "linux")]
    {
        linux::inspect(save, backup_parent, roots).unwrap_or_else(|denied| denied)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (save, backup_parent, roots);
        Admission::Unsupported(Reason::Platform)
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use super::{Admission, ApprovedRoots, Reason};
    use std::ffi::OsString;
    use std::fs::{self, Metadata};
    use std::os::unix::{ffi::OsStringExt, fs::MetadataExt};
    use std::path::{Component, Path, PathBuf};

    type Check<T> = Result<T, Admission>;

    pub(super) fn inspect(
        save: &Path,
        backup_parent: &Path,
        roots: &ApprovedRoots<'_>,
    ) -> Check<Admission> {
        let mounts = fs::read("/proc/self/mountinfo")
            .ok()
            .and_then(|data| parse_mounts(&data))
            .ok_or(Admission::Unknown(Reason::MountIdentity))?;
        inspect_mounts(save, backup_parent, roots, &mounts)
    }

    fn inspect_mounts(
        save: &Path,
        backup_parent: &Path,
        roots: &ApprovedRoots<'_>,
        mounts: &[Mount],
    ) -> Check<Admission> {
        let save = checked_directory(save)?;
        let backup = checked_directory(backup_parent)?;
        let save_root = checked_directory(roots.saves)?;
        let backup_root = checked_directory(roots.backups)?;
        if !save.starts_with(&save_root)
            || !backup.starts_with(&backup_root)
            || backup.starts_with(&save)
        {
            return Err(Admission::Unsupported(Reason::OutsideApprovedRoots));
        }
        for root in roots.synchronized {
            let root = absolute(root)?;
            let intersects = |root: &Path| {
                save.starts_with(root) || backup.starts_with(root) || root.starts_with(&save)
            };
            if intersects(&root)
                || fs::canonicalize(root).is_ok_and(|resolved| intersects(&resolved))
            {
                return Err(Admission::Unsupported(Reason::Synchronized));
            }
        }
        let save_mount = check_mount(&save, mounts)?;
        check_mount(&backup, mounts)?;
        // The backup walks the entire save. A mount underneath it crosses the
        // verified mount boundary, including same-device aliases.
        if mounts
            .iter()
            .any(|mount| mount.path != save && mount.path.starts_with(&save))
        {
            return Err(Admission::Unsupported(Reason::NestedMount));
        }

        scan_save(&save, save_mount.dev)?;
        Ok(Admission::Admitted)
    }

    fn absolute(path: &Path) -> Check<PathBuf> {
        if path.is_absolute() && !path.components().any(|part| part == Component::ParentDir) {
            Ok(path.components().collect())
        } else {
            Err(Admission::Unknown(Reason::Path))
        }
    }

    fn checked_directory(path: &Path) -> Check<PathBuf> {
        let path = absolute(path)?;
        let mut ancestor = PathBuf::new();
        for part in path.components() {
            ancestor.push(part.as_os_str());
            let metadata = metadata(&ancestor)?;
            if !metadata.is_dir() {
                return Err(Admission::Unsupported(Reason::LinkOrSpecialFile));
            }
        }
        if fs::canonicalize(&path).ok().as_ref() != Some(&path) {
            return Err(Admission::Unknown(Reason::Path));
        }
        Ok(path)
    }

    fn metadata(path: &Path) -> Check<Metadata> {
        fs::symlink_metadata(path).map_err(|_| Admission::Unknown(Reason::Path))
    }

    fn device(metadata: &Metadata) -> (u32, u32) {
        let dev = metadata.dev();
        (libc::major(dev), libc::minor(dev))
    }

    fn scan_save(save: &Path, dev: (u32, u32)) -> Check<()> {
        let mut dirs = vec![save.to_owned()];
        while let Some(dir) = dirs.pop() {
            let entries = fs::read_dir(dir).map_err(|_| Admission::Unknown(Reason::Path))?;
            for entry in entries {
                let path = entry.map_err(|_| Admission::Unknown(Reason::Path))?.path();
                let meta = metadata(&path)?;
                if device(&meta) != dev {
                    return Err(Admission::Unsupported(Reason::NestedMount));
                }
                if meta.is_dir() {
                    dirs.push(path);
                } else if !meta.is_file() || meta.nlink() != 1 {
                    return Err(Admission::Unsupported(Reason::LinkOrSpecialFile));
                }
            }
        }
        Ok(())
    }

    struct Mount {
        path: PathBuf,
        dev: (u32, u32),
        ext4: bool,
        writable: bool,
    }

    fn check_mount<'a>(path: &Path, mounts: &'a [Mount]) -> Check<&'a Mount> {
        let unknown = || Admission::Unknown(Reason::MountIdentity);
        let selected = mounts
            .iter()
            .filter(|mount| path.starts_with(&mount.path))
            .max_by_key(|mount| mount.path.components().count())
            .ok_or_else(unknown)?;
        if mounts
            .iter()
            .filter(|mount| mount.path == selected.path)
            .count()
            != 1
            || selected.dev != device(&metadata(path)?)
        {
            return Err(unknown());
        }
        if !selected.ext4 {
            return Err(Admission::Unsupported(Reason::Filesystem));
        }
        if !selected.writable {
            return Err(Admission::Unsupported(Reason::ReadOnlyMount));
        }
        Ok(selected)
    }

    fn parse_mounts(data: &[u8]) -> Option<Vec<Mount>> {
        data.split(|&byte| byte == b'\n')
            .filter(|row| !row.is_empty())
            .map(parse_mount)
            .collect()
    }

    fn parse_mount(row: &[u8]) -> Option<Mount> {
        let separator = row.windows(3).position(|window| window == b" - ")?;
        let mut left = row[..separator].split(|&byte| byte == b' ');
        let (major, minor) = std::str::from_utf8(left.nth(2)?).ok()?.split_once(':')?;
        let dev = (major.parse().ok()?, minor.parse().ok()?);
        let path = mount_path(left.nth(1)?)?;
        let mount_opts = left.next()?;
        let mut right = row[separator + 3..].split(|&byte| byte == b' ');
        let ext4 = right.next()? == b"ext4";
        let super_opts = right.nth(1)?;
        let writable = mount_opts
            .split(|&byte| byte == b',')
            .any(|option| option == b"rw")
            && super_opts
                .split(|&byte| byte == b',')
                .any(|option| option == b"rw");
        Some(Mount {
            path,
            dev,
            ext4,
            writable,
        })
    }

    fn mount_path(mut remaining: &[u8]) -> Option<PathBuf> {
        let mut decoded = Vec::new();
        while let Some((&byte, tail)) = remaining.split_first() {
            if byte == b'\\' {
                decoded.push(match tail.get(..3)? {
                    b"040" => b' ',
                    b"011" => b'\t',
                    b"012" => b'\n',
                    b"134" => b'\\',
                    _ => return None,
                });
                remaining = tail.get(3..)?;
            } else {
                decoded.push(byte);
                remaining = tail;
            }
        }
        let path = PathBuf::from(OsString::from_vec(decoded));
        path.is_absolute().then_some(path)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn mountinfo_decodes_paths_and_checks_both_writable_flags() {
            let rows = parse_mounts(
                b"41 7 8:2 / /approved\\040root rw,nosuid shared:1 - ext4 /dev/vda rw\n\
                  42 41 8:2 /sub /approved\\040root/read-only ro - ext4 /dev/vda rw\n\
                  43 41 8:2 / /approved\\040root/super-ro rw - ext4 /dev/vda ro\n\
                  44 41 0:17 / /approved\\040root/fuse rw - fuse.remote remote rw\n",
            )
            .unwrap();
            assert_eq!(rows[0].path, Path::new("/approved root"));
            assert_eq!(rows[0].dev, (8, 2));
            assert!(rows[0].ext4 && rows[0].writable);
            assert!(!rows[1].writable && !rows[2].writable);
            assert!(!rows[3].ext4);
            assert!(parse_mounts(b"broken").is_none());
        }

        #[test]
        fn tree_scan_rejects_links_and_special_entries_without_writing() {
            use std::os::unix::{fs::symlink, net::UnixListener};
            let root =
                std::env::temp_dir().join(format!("talkdnd-344-scan-{}", std::process::id()));
            fs::create_dir(&root).unwrap();
            let file = root.join("user");
            let link = root.join("link");
            let socket = root.join("socket");
            let dev = device(&metadata(&root).unwrap());
            fs::write(&file, b"save bytes").unwrap();
            assert_eq!(scan_save(&root, dev), Ok(()));
            symlink(&file, &link).unwrap();
            let refused = Err(Admission::Unsupported(Reason::LinkOrSpecialFile));
            assert_eq!(scan_save(&root, dev), refused);
            fs::remove_file(&link).unwrap();
            fs::hard_link(&file, &link).unwrap();
            assert_eq!(scan_save(&root, dev), refused);
            fs::remove_file(&link).unwrap();
            let listener = UnixListener::bind(&socket).unwrap();
            assert_eq!(scan_save(&root, dev), refused);
            assert_eq!(fs::read(&file).unwrap(), b"save bytes");
            assert_eq!(fs::read_dir(&root).unwrap().count(), 2);
            drop(listener);
            fs::remove_file(socket).unwrap();
            fs::remove_file(file).unwrap();
            fs::remove_dir(root).unwrap();
        }
    }
}
