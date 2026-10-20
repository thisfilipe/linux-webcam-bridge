use std::fs;
use std::path::{Path, PathBuf};

pub const EXPECTED_LABEL: &str = "Jieli 720p30 V4L2";
pub const PRODUCT_NODE_NAME: &str = "jieli_720p30";
pub const SYSFS_VIDEO: &str = "/sys/class/video4linux";
pub const DEV_ROOT: &str = "/dev";
pub const VIRTUAL_VIDEO_ROOT: &str = "/sys/devices/virtual/video4linux";

/// The live publisher only feeds the loopback for the product node name and
/// when the user has not created the disable flag.
pub fn enabled(node_name: &str, home: &Path) -> bool {
    node_name == PRODUCT_NODE_NAME && !home.join(".config/jieli-camera/disable-v4l2").exists()
}

/// Find the v4l2loopback node with the live label rules.
///
/// The entry must be a virtual video device and readable/writable by the
/// current process. The roots are parameters so tests never touch the real
/// `/dev` or `/sys`.
pub fn discover_device(sysfs_root: &Path, dev_root: &Path, virtual_root: &Path) -> Option<PathBuf> {
    let entries = fs::read_dir(sysfs_root).ok()?;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with("video") {
            continue;
        }
        let directory = entry.path();
        let Some(label) = fs::read_to_string(directory.join("name")).ok() else {
            continue;
        };
        let label = label.trim();
        let label_matches = label == EXPECTED_LABEL || (name == "video10" && label == "Jieli");
        if !label_matches {
            continue;
        }

        let Ok(resolved) = fs::canonicalize(&directory) else {
            continue;
        };
        if !resolved.starts_with(virtual_root) {
            continue;
        }

        let device = dev_root.join(&name);
        if is_read_write(&device) {
            return Some(device);
        }
    }
    None
}

pub fn is_read_write(path: &Path) -> bool {
    rustix::fs::access(
        path,
        rustix::fs::Access::READ_OK | rustix::fs::Access::WRITE_OK,
    )
    .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device_entry(sysfs: &Path, name: &str, label: &str) -> PathBuf {
        let directory = sysfs.join(name);
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("name"), format!("{label}\n")).unwrap();
        directory
    }

    #[test]
    fn enabled_only_for_product_node_without_flag() {
        let directory = tempfile::tempdir().unwrap();
        let home = directory.path();
        assert!(enabled(PRODUCT_NODE_NAME, home));
        assert!(!enabled("jieli_test", home));

        let flag = home.join(".config/jieli-camera/disable-v4l2");
        fs::create_dir_all(flag.parent().unwrap()).unwrap();
        fs::write(&flag, b"").unwrap();
        assert!(!enabled(PRODUCT_NODE_NAME, home));
    }

    #[test]
    fn matches_video10_jieli_fallback() {
        let directory = tempfile::tempdir().unwrap();
        let sysfs = directory.path().join("sys");
        let dev = directory.path().join("dev");
        fs::create_dir_all(&sysfs).unwrap();
        fs::create_dir_all(&dev).unwrap();
        let virtual_root = fs::canonicalize(&sysfs).unwrap();

        device_entry(&sysfs, "video9", "Jieli");
        fs::write(dev.join("video9"), b"").unwrap();
        assert!(discover_device(&sysfs, &dev, &virtual_root).is_none());

        device_entry(&sysfs, "video10", "Jieli");
        assert!(discover_device(&sysfs, &dev, &virtual_root).is_none());
        fs::write(dev.join("video10"), b"").unwrap();
        assert_eq!(
            discover_device(&sysfs, &dev, &virtual_root),
            Some(dev.join("video10"))
        );
    }

    #[test]
    fn matches_the_expected_label() {
        let directory = tempfile::tempdir().unwrap();
        let sysfs = directory.path().join("sys");
        let dev = directory.path().join("dev");
        fs::create_dir_all(&sysfs).unwrap();
        fs::create_dir_all(&dev).unwrap();
        let virtual_root = fs::canonicalize(&sysfs).unwrap();

        device_entry(&sysfs, "video0", EXPECTED_LABEL);
        fs::write(dev.join("video0"), b"").unwrap();
        assert_eq!(
            discover_device(&sysfs, &dev, &virtual_root),
            Some(dev.join("video0"))
        );
    }

    #[test]
    fn ignores_non_virtual_or_other_labels() {
        let directory = tempfile::tempdir().unwrap();
        let sysfs = directory.path().join("sys");
        let dev = directory.path().join("dev");
        fs::create_dir_all(&sysfs).unwrap();
        fs::create_dir_all(&dev).unwrap();

        device_entry(&sysfs, "video0", "Other camera");
        device_entry(&sysfs, "video1", EXPECTED_LABEL);
        fs::write(dev.join("video1"), b"").unwrap();

        let virtual_root = directory.path().join("not-virtual");
        assert!(discover_device(&sysfs, &dev, &virtual_root).is_none());
    }

    #[test]
    fn access_requires_an_existing_read_write_file() {
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("device");
        assert!(!is_read_write(&file));
        fs::write(&file, b"").unwrap();
        assert!(is_read_write(&file));
    }
}
