use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;

use crate::fsutil;
use crate::layout::{FileSpec, Layout};
use crate::manifest;

pub fn service_active() -> Result<bool, String> {
    let status = Command::new("systemctl")
        .args([
            "--user",
            "is-active",
            "--quiet",
            "jieli-camera-bridge.service",
        ])
        .status()
        .map_err(|error| format!("cannot query the user service state: {error}"))?;
    match status.code() {
        Some(0) => Ok(true),
        Some(3 | 4) => Ok(false),
        Some(code) => Err(format!(
            "cannot establish the user service state (systemctl exit {code})"
        )),
        None => Err("systemctl terminated without an exit status".to_string()),
    }
}

pub fn plan(layout: &Layout) {
    println!("User Rust installation plan:");
    for file in layout.files() {
        println!(
            "  {} -> {} (mode {:04o})",
            file.source.display(),
            file.target.display(),
            file.mode
        );
    }
    println!("  manifest: {}", layout.manifest.display());
    println!("  backups: {}", layout.backups.display());
    println!("The bridge is stopped separately; this tool never starts or stops services.");
}

pub fn install(layout: &Layout, replace: bool, check_service: bool) -> Result<(), String> {
    if check_service && service_active()? {
        return Err("stop jieli-camera-bridge.service before installing files".to_string());
    }
    manifest::read(&layout.manifest)?;

    let mut staged = Vec::new();
    for file in layout.files() {
        let data = read_source(file)?;
        let hash = fsutil::digest(&data);
        validate_target(file, &hash, replace)?;
        staged.push((file, data, hash));
    }

    let backup = if any_existing(layout) {
        let directory = fsutil::backup_directory(&layout.backups)?;
        for file in layout.files() {
            if file.target.exists() {
                fsutil::archive(&file.target, &layout.home, &directory)?;
            }
        }
        if layout.manifest.exists() {
            fsutil::archive(&layout.manifest, &layout.home, &directory)?;
        }
        Some(directory)
    } else {
        None
    };

    let mut hashes = BTreeMap::new();
    for (file, data, hash) in staged {
        fsutil::atomic_write(&file.target, &data, file.mode)?;
        hashes.insert(file.target.to_string_lossy().into_owned(), hash);
    }
    manifest::write(&layout.manifest, &hashes)?;
    if let Some(directory) = backup {
        println!(
            "Installed Rust user files; previous copies archived at {}",
            directory.display()
        );
    } else {
        println!("Installed Rust user files.");
    }
    Ok(())
}

pub fn remove(layout: &Layout, check_service: bool) -> Result<(), String> {
    if check_service && service_active()? {
        return Err("stop jieli-camera-bridge.service before removing files".to_string());
    }
    let Some(hashes) = manifest::read(&layout.manifest)? else {
        return Err(format!(
            "no installation manifest at {}; inspect the installation manually",
            layout.manifest.display()
        ));
    };
    if hashes
        .keys()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>()
        != layout.target_set()
    {
        return Err("installation manifest paths do not match this layout".to_string());
    }
    for file in layout.files() {
        match fs::symlink_metadata(&file.target) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(format!("refusing symlink: {}", file.target.display()));
            }
            Ok(metadata) if !metadata.is_file() => {
                return Err(format!(
                    "installed path is not a regular file: {}",
                    file.target.display()
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(format!("cannot inspect {}: {error}", file.target.display()));
            }
        }
        let expected = hashes
            .get(&file.target.to_string_lossy().into_owned())
            .ok_or_else(|| format!("manifest has no entry for {}", file.target.display()))?;
        if fsutil::digest_file(&file.target)? != *expected {
            return Err(format!(
                "modified file preserved: {}",
                file.target.display()
            ));
        }
    }

    let backup = fsutil::backup_directory(&layout.backups)?;
    for file in layout.files() {
        if file.target.exists() {
            fsutil::archive(&file.target, &layout.home, &backup)?;
            fs::remove_file(&file.target)
                .map_err(|error| format!("cannot remove {}: {error}", file.target.display()))?;
        }
    }
    fsutil::archive(&layout.manifest, &layout.home, &backup)?;
    fs::remove_file(&layout.manifest)
        .map_err(|error| format!("cannot remove {}: {error}", layout.manifest.display()))?;
    println!("Removed Rust user files; archived at {}", backup.display());
    Ok(())
}

fn read_source(file: &FileSpec) -> Result<Vec<u8>, String> {
    fsutil::regular_file(&file.source)?;
    let metadata = fs::metadata(&file.source)
        .map_err(|error| format!("cannot inspect {}: {error}", file.source.display()))?;
    if file.mode & 0o111 != 0 && metadata.permissions().mode() & 0o111 == 0 {
        return Err(format!(
            "source is not executable: {}",
            file.source.display()
        ));
    }
    fs::read(&file.source)
        .map_err(|error| format!("cannot read {}: {error}", file.source.display()))
}

fn validate_target(file: &FileSpec, source_hash: &str, replace: bool) -> Result<(), String> {
    match fs::symlink_metadata(&file.target) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(format!("refusing symlink: {}", file.target.display()));
        }
        Ok(metadata) if !metadata.is_file() => {
            return Err(format!(
                "destination is not a regular file: {}",
                file.target.display()
            ));
        }
        Ok(_) => {
            if !replace && fsutil::digest_file(&file.target)? != source_hash {
                return Err(format!(
                    "existing different file preserved: {}; review before using --replace",
                    file.target.display()
                ));
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(format!("cannot inspect {}: {error}", file.target.display()));
        }
    }
    Ok(())
}

fn any_existing(layout: &Layout) -> bool {
    layout.manifest.exists() || layout.files().iter().any(|file| file.target.exists())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::environment::Environment;

    struct Fixture {
        directory: tempfile::TempDir,
        layout: Layout,
        home: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path().join("root");
            let home = directory.path().join("home");
            fs::create_dir_all(root.join("target/release")).unwrap();
            fs::create_dir_all(root.join("build/plugin")).unwrap();
            fs::create_dir_all(root.join("config")).unwrap();
            fs::create_dir_all(&home).unwrap();
            let environment = Environment {
                home: home.clone(),
                config: home.join(".config"),
                state: home.join(".local/state"),
                root,
            };
            let layout = Layout::new(&environment, None).unwrap();
            for file in layout.files() {
                fs::create_dir_all(file.source.parent().unwrap()).unwrap();
                fs::write(&file.source, format!("source:{}", file.target.display())).unwrap();
                if file.mode & 0o111 != 0 {
                    fs::set_permissions(&file.source, fs::Permissions::from_mode(0o755)).unwrap();
                }
            }
            Self {
                directory,
                layout,
                home,
            }
        }

        fn environment(&self) -> Environment {
            Environment {
                home: self.home.clone(),
                config: self.home.join(".config"),
                state: self.home.join(".local/state"),
                root: self.directory.path().join("root"),
            }
        }
    }

    #[test]
    fn installs_manifest_and_modes_then_removes() {
        let fixture = Fixture::new();
        install(&fixture.layout, false, false).unwrap();
        for file in fixture.layout.files() {
            assert!(file.target.is_file());
            assert_eq!(
                fs::metadata(&file.target).unwrap().permissions().mode() & 0o777,
                file.mode
            );
        }
        let hashes = manifest::read(&fixture.layout.manifest).unwrap().unwrap();
        assert_eq!(hashes.len(), 5);
        remove(&fixture.layout, false).unwrap();
        assert!(!fixture.layout.manifest.exists());
        assert!(
            fixture
                .layout
                .files()
                .iter()
                .all(|file| !file.target.exists())
        );
        assert!(fixture.layout.backups.exists());
    }

    #[test]
    fn conflicting_file_requires_replace_and_is_archived() {
        let fixture = Fixture::new();
        let target = &fixture.layout.files()[0].target;
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(target, b"local customization").unwrap();
        assert!(install(&fixture.layout, false, false).is_err());
        assert_eq!(fs::read(target).unwrap(), b"local customization");
        install(&fixture.layout, true, false).unwrap();
        assert_ne!(fs::read(target).unwrap(), b"local customization");
        assert!(
            fs::read_dir(&fixture.layout.backups)
                .unwrap()
                .next()
                .is_some()
        );
    }

    #[test]
    fn modified_installed_file_is_preserved_on_remove() {
        let fixture = Fixture::new();
        install(&fixture.layout, false, false).unwrap();
        let target = &fixture.layout.files()[1].target;
        fs::write(target, b"modified").unwrap();
        assert!(remove(&fixture.layout, false).is_err());
        assert_eq!(fs::read(target).unwrap(), b"modified");
        assert!(fixture.layout.manifest.exists());
    }

    #[test]
    fn symlink_target_is_refused() {
        let fixture = Fixture::new();
        let target = &fixture.layout.files()[0].target;
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink("/tmp", target).unwrap();
        assert!(install(&fixture.layout, true, false).is_err());
    }

    #[test]
    fn missing_source_is_refused_before_writes() {
        let fixture = Fixture::new();
        fs::remove_file(&fixture.layout.files()[0].source).unwrap();
        assert!(install(&fixture.layout, false, false).is_err());
        assert!(
            fixture
                .layout
                .files()
                .iter()
                .all(|file| !file.target.exists())
        );
    }

    #[test]
    fn manifest_mismatch_is_refused() {
        let fixture = Fixture::new();
        install(&fixture.layout, false, false).unwrap();
        let mut text = fs::read_to_string(&fixture.layout.manifest).unwrap();
        text = text.replace("jieli-bridge", "other");
        fs::write(&fixture.layout.manifest, text).unwrap();
        assert!(remove(&fixture.layout, false).is_err());
    }

    #[test]
    fn environment_paths_are_not_required_for_install_api() {
        let fixture = Fixture::new();
        let _ = fixture.environment();
        assert!(!fixture.layout.files().is_empty());
    }
}
