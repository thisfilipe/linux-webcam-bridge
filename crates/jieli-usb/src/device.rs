use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{HelperError, PID, VID, io_message};

pub const SYSFS_ROOT: &str = "/sys/bus/usb/devices";
pub const DEV_ROOT: &str = "/dev/bus/usb";
pub const USB_MAJOR: u32 = 189;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    pub name: String,
    pub bus: u8,
    pub address: u8,
    pub path: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    Character,
    Regular,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TargetStat {
    pub kind: FileKind,
    pub major: u32,
    pub minor: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LockStat {
    pub uid: u32,
    pub kind: FileKind,
    pub nlink: u64,
    pub mode: u32,
}

pub fn file_kind_from_raw_mode(raw_mode: u32) -> FileKind {
    let file_type = raw_mode & libc::S_IFMT;
    if file_type == libc::S_IFCHR {
        FileKind::Character
    } else if file_type == libc::S_IFREG {
        FileKind::Regular
    } else {
        FileKind::Other
    }
}

pub fn parse_hex_u16(text: &str) -> Option<u16> {
    u16::from_str_radix(text.trim(), 16).ok()
}

pub fn parse_bus(text: &str) -> Option<u8> {
    text.trim()
        .parse::<u16>()
        .ok()
        .filter(|value| (1..=255).contains(value))
        .map(|value| value as u8)
}

pub fn parse_address(text: &str) -> Option<u8> {
    text.trim()
        .parse::<u16>()
        .ok()
        .filter(|value| (1..=127).contains(value))
        .map(|value| value as u8)
}

fn read_trimmed(path: &Path) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .map(|text| text.trim().to_string())
}

pub fn discover_in(root: &Path) -> Result<Device, HelperError> {
    let entries = fs::read_dir(root).map_err(|error| {
        HelperError::Os(format!("{}: '{}'", io_message(&error), root.display()))
    })?;

    let mut matches = Vec::new();
    for entry in entries.flatten() {
        let directory = entry.path();
        let Some(vendor) = read_trimmed(&directory.join("idVendor")) else {
            continue;
        };
        let Some(product) = read_trimmed(&directory.join("idProduct")) else {
            continue;
        };
        if parse_hex_u16(&vendor) != Some(VID) || parse_hex_u16(&product) != Some(PID) {
            continue;
        }
        let Some(bus) = read_trimmed(&directory.join("busnum")).and_then(|text| parse_bus(&text))
        else {
            continue;
        };
        let Some(address) =
            read_trimmed(&directory.join("devnum")).and_then(|text| parse_address(&text))
        else {
            continue;
        };
        matches.push(Device {
            name: entry.file_name().to_string_lossy().into_owned(),
            bus,
            address,
            path: PathBuf::from(format!("{DEV_ROOT}/{bus:03}/{address:03}")),
        });
    }

    if matches.len() != 1 {
        return Err(HelperError::DeviceCount(matches.len()));
    }
    Ok(matches.remove(0))
}

pub fn discover() -> Result<Device, HelperError> {
    discover_in(Path::new(SYSFS_ROOT))
}

pub fn validate_target(device: &Device, stat: TargetStat) -> Result<(), HelperError> {
    if stat.kind != FileKind::Character || stat.major != USB_MAJOR {
        return Err(HelperError::NotCharacterDevice);
    }
    let expected_minor = (device.bus as u32 - 1) * 128 + device.address as u32 - 1;
    if stat.minor != expected_minor {
        return Err(HelperError::AddressChanged);
    }
    Ok(())
}

pub fn validate_descriptor(descriptor: &[u8]) -> Result<(), HelperError> {
    if descriptor.len() != 18 || descriptor[..2] != [0x12, 0x01] {
        return Err(HelperError::InvalidDescriptor);
    }
    let vendor = u16::from_le_bytes([descriptor[8], descriptor[9]]);
    let product = u16::from_le_bytes([descriptor[10], descriptor[11]]);
    if (vendor, product) != (VID, PID) {
        return Err(HelperError::WrongDevice);
    }
    Ok(())
}

pub fn validate_lock(stat: LockStat) -> Result<(), HelperError> {
    if stat.uid != 0 || stat.kind != FileKind::Regular || stat.nlink != 1 || stat.mode & 0o022 != 0
    {
        return Err(HelperError::UnsafeLock);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device() -> Device {
        Device {
            name: "8-1".to_string(),
            bus: 8,
            address: 15,
            path: PathBuf::from("/dev/bus/usb/008/015"),
        }
    }

    fn descriptor(vendor: u16, product: u16) -> Vec<u8> {
        let mut data = vec![0u8; 18];
        data[..2].copy_from_slice(&[0x12, 0x01]);
        data[8..10].copy_from_slice(&vendor.to_le_bytes());
        data[10..12].copy_from_slice(&product.to_le_bytes());
        data
    }

    #[test]
    fn parses_sysfs_values_with_bounds() {
        assert_eq!(parse_hex_u16(" 2a25\n"), Some(0x2a25));
        assert_eq!(parse_hex_u16("zz"), None);
        assert_eq!(parse_bus("8"), Some(8));
        assert_eq!(parse_bus("0"), None);
        assert_eq!(parse_bus("256"), None);
        assert_eq!(parse_address("15"), Some(15));
        assert_eq!(parse_address("0"), None);
        assert_eq!(parse_address("128"), None);
    }

    #[test]
    fn validates_character_device_and_minor_address() {
        let stat = TargetStat {
            kind: FileKind::Character,
            major: USB_MAJOR,
            minor: (8 - 1) * 128 + 15 - 1,
        };
        assert_eq!(validate_target(&device(), stat), Ok(()));

        assert_eq!(
            validate_target(
                &device(),
                TargetStat {
                    kind: FileKind::Regular,
                    ..stat
                }
            ),
            Err(HelperError::NotCharacterDevice)
        );
        assert_eq!(
            validate_target(&device(), TargetStat { major: 81, ..stat }),
            Err(HelperError::NotCharacterDevice)
        );
        assert_eq!(
            validate_target(
                &device(),
                TargetStat {
                    minor: stat.minor + 1,
                    ..stat
                }
            ),
            Err(HelperError::AddressChanged)
        );
    }

    #[test]
    fn validates_usb_descriptor_identity() {
        assert_eq!(validate_descriptor(&descriptor(VID, PID)), Ok(()));
        assert_eq!(
            validate_descriptor(&descriptor(0x3277, PID)),
            Err(HelperError::WrongDevice)
        );
        assert_eq!(
            validate_descriptor(&descriptor(VID, 0x0029)),
            Err(HelperError::WrongDevice)
        );
        assert_eq!(
            validate_descriptor(&[0x12, 0x01]),
            Err(HelperError::InvalidDescriptor)
        );
        let mut wrong_prefix = descriptor(VID, PID);
        wrong_prefix[1] = 0x02;
        assert_eq!(
            validate_descriptor(&wrong_prefix),
            Err(HelperError::InvalidDescriptor)
        );
    }

    #[test]
    fn rejects_untrusted_or_writable_lock() {
        let safe = LockStat {
            uid: 0,
            kind: FileKind::Regular,
            nlink: 1,
            mode: 0o600,
        };
        assert_eq!(validate_lock(safe), Ok(()));
        assert_eq!(
            validate_lock(LockStat { uid: 1000, ..safe }),
            Err(HelperError::UnsafeLock)
        );
        assert_eq!(
            validate_lock(LockStat {
                kind: FileKind::Character,
                ..safe
            }),
            Err(HelperError::UnsafeLock)
        );
        assert_eq!(
            validate_lock(LockStat { nlink: 2, ..safe }),
            Err(HelperError::UnsafeLock)
        );
        assert_eq!(
            validate_lock(LockStat {
                mode: 0o622,
                ..safe
            }),
            Err(HelperError::UnsafeLock)
        );
    }

    #[test]
    fn discovery_requires_exactly_one_matching_device() {
        let directory = tempfile::tempdir().unwrap();
        assert_eq!(
            discover_in(directory.path()),
            Err(HelperError::DeviceCount(0))
        );

        for name in ["8-1", "8-2"] {
            let entry = directory.path().join(name);
            fs::create_dir(&entry).unwrap();
            fs::write(entry.join("idVendor"), "1224").unwrap();
            fs::write(entry.join("idProduct"), "2a25").unwrap();
            fs::write(entry.join("busnum"), "8").unwrap();
            fs::write(entry.join("devnum"), "15").unwrap();
            if name == "8-1" {
                let found = discover_in(directory.path()).unwrap();
                assert_eq!((found.bus, found.address), (8, 15));
                assert_eq!(found.path, PathBuf::from("/dev/bus/usb/008/015"));
            }
        }
        assert_eq!(
            discover_in(directory.path()),
            Err(HelperError::DeviceCount(2))
        );
    }

    #[test]
    fn discovery_ignores_other_vendors_and_malformed_entries() {
        let directory = tempfile::tempdir().unwrap();
        let other = directory.path().join("9-1");
        fs::create_dir(&other).unwrap();
        fs::write(other.join("idVendor"), "3277").unwrap();
        fs::write(other.join("idProduct"), "2a25").unwrap();
        fs::write(other.join("busnum"), "9").unwrap();
        fs::write(other.join("devnum"), "2").unwrap();

        let incomplete = directory.path().join("9-2");
        fs::create_dir(&incomplete).unwrap();
        fs::write(incomplete.join("idVendor"), "1224").unwrap();

        let out_of_range = directory.path().join("9-3");
        fs::create_dir(&out_of_range).unwrap();
        fs::write(out_of_range.join("idVendor"), "1224").unwrap();
        fs::write(out_of_range.join("idProduct"), "2a25").unwrap();
        fs::write(out_of_range.join("busnum"), "0").unwrap();
        fs::write(out_of_range.join("devnum"), "128").unwrap();

        assert_eq!(
            discover_in(directory.path()),
            Err(HelperError::DeviceCount(0))
        );
    }
}
