use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};

pub fn atomic_write(path: &Path, data: &[u8], mode: u32) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("destination has no parent: {}", path.display()))?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)
        .map_err(|error| format!("cannot stage {}: {error}", path.display()))?;
    temporary
        .write_all(data)
        .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
    temporary
        .as_file()
        .set_permissions(fs::Permissions::from_mode(mode))
        .map_err(|error| format!("cannot set mode on {}: {error}", path.display()))?;
    temporary
        .persist(path)
        .map_err(|error| format!("cannot publish {}: {error}", path.display()))?;
    Ok(())
}

pub fn digest_file(path: &Path) -> Result<String, String> {
    let data =
        fs::read(path).map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    Ok(digest(&data))
}

pub fn digest(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}

pub fn regular_file(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("cannot inspect {}: {error}", path.display()))?;
    if metadata.file_type().is_symlink() {
        return Err(format!("refusing symlink: {}", path.display()));
    }
    if !metadata.is_file() {
        return Err(format!("not a regular file: {}", path.display()));
    }
    Ok(())
}

pub fn archive(path: &Path, home: &Path, backup: &Path) -> Result<(), String> {
    let relative = path
        .strip_prefix(home)
        .map(Path::to_path_buf)
        .unwrap_or_else(|_| {
            let absolute = path.to_string_lossy();
            PathBuf::from("external").join(absolute.trim_start_matches('/'))
        });
    let destination = backup.join(relative);
    let parent = destination
        .parent()
        .ok_or_else(|| format!("backup path has no parent: {}", destination.display()))?;
    fs::create_dir_all(parent).map_err(|error| {
        format!(
            "cannot create backup directory {}: {error}",
            parent.display()
        )
    })?;
    fs::copy(path, &destination).map_err(|error| {
        format!(
            "cannot archive {} to {}: {error}",
            path.display(),
            destination.display()
        )
    })?;
    Ok(())
}

pub fn backup_directory(base: &Path) -> Result<PathBuf, String> {
    fs::create_dir_all(base)
        .map_err(|error| format!("cannot create backup base {}: {error}", base.display()))?;
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("system clock is before the Unix epoch: {error}"))?
        .as_secs();
    for suffix in 0..1000 {
        let name = if suffix == 0 {
            seconds.to_string()
        } else {
            format!("{seconds}-{suffix}")
        };
        let directory = base.join(name);
        match fs::create_dir(&directory) {
            Ok(()) => return Ok(directory),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(format!(
                    "cannot create backup directory {}: {error}",
                    directory.display()
                ));
            }
        }
    }
    Err(format!(
        "cannot find a free backup directory under {}",
        base.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digest_matches_known_sha256() {
        assert_eq!(
            digest(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn atomic_write_sets_mode_and_replaces() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("nested/file");
        atomic_write(&path, b"first", 0o600).unwrap();
        atomic_write(&path, b"second", 0o644).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"second");
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o644
        );
    }

    #[test]
    fn symlinks_are_refused() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("target");
        fs::write(&target, b"x").unwrap();
        let link = directory.path().join("link");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(regular_file(&link).is_err());
    }

    #[test]
    fn archives_relative_to_home_and_external_paths() {
        let directory = tempfile::tempdir().unwrap();
        let home = directory.path().join("home");
        let backup = directory.path().join("backup");
        let path = home.join("a/b");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"content").unwrap();
        archive(&path, &home, &backup).unwrap();
        assert_eq!(fs::read(backup.join("a/b")).unwrap(), b"content");
    }
}
