use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use crate::fsutil;

pub type Hashes = BTreeMap<String, String>;

pub fn read(path: &Path) -> Result<Option<Hashes>, String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(format!("refusing symlink manifest: {}", path.display()));
        }
        Ok(metadata) if !metadata.is_file() => {
            return Err(format!(
                "manifest is not a regular file: {}",
                path.display()
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(format!(
                "cannot inspect manifest {}: {error}",
                path.display()
            ));
        }
    }
    let data =
        fs::read(path).map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let hashes: Hashes = serde_json::from_slice(&data)
        .map_err(|error| format!("invalid installation manifest {}: {error}", path.display()))?;
    Ok(Some(hashes))
}

pub fn write(path: &Path, hashes: &Hashes) -> Result<(), String> {
    let mut data = serde_json::to_vec_pretty(hashes)
        .map_err(|error| format!("cannot serialize installation manifest: {error}"))?;
    data.push(b'\n');
    fsutil::atomic_write(path, &data, 0o600)
}
