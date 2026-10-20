use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

/// Stable by-id node used by the live supervisor.
pub const DEVICE_PATH: &str = "/dev/v4l/by-id/usb-Jieli_Technology_USB_PHY_2.0-video-index0";

/// Identity used to detect reenumeration behind the stable by-id symlink.
///
/// The resolved path, inode and device number change when the camera node is
/// recreated after a USB reset.
pub fn identity(path: &Path) -> Option<String> {
    let metadata = fs::metadata(path).ok()?;
    let resolved = fs::canonicalize(path).ok()?;
    Some(format!(
        "{}:{}:{}",
        resolved.display(),
        metadata.ino(),
        metadata.rdev()
    ))
}

pub fn present(path: &Path) -> bool {
    path.exists()
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::symlink;

    use super::*;

    #[test]
    fn identity_follows_the_by_id_symlink_and_includes_inode() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("video0");
        fs::write(&target, b"node").unwrap();
        let link = directory.path().join("by-id");
        symlink(&target, &link).unwrap();

        let identity = identity(&link).expect("identity");
        assert!(identity.starts_with(&format!(
            "{}:",
            fs::canonicalize(&target).unwrap().display()
        )));
        assert!(identity.contains(':'));
        assert!(identity.ends_with(":0"));
        assert!(present(&link));
    }

    #[test]
    fn missing_device_has_no_identity() {
        let directory = tempfile::tempdir().unwrap();
        let missing = directory.path().join("missing");
        assert_eq!(identity(&missing), None);
        assert!(!present(&missing));
    }
}
