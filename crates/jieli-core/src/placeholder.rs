use std::sync::OnceLock;

use crate::frame::{Frame, UV_SIZE, WIDTH, Y_SIZE};

pub const CAMERA_UNAVAILABLE: &str = "CAMERA INDISPONIVEL";
pub const RECONNECTING: &str = "RECONECTANDO";
pub const PLACEHOLDER_SHA256: &str =
    "b7d72d08e173e810d3fc72b0c8f52ac954cbb6f4dfd627e06fe1666ec8b387d2";

const GLYPHS: &[(char, [u8; 7])] = &[
    ('A', [14, 17, 17, 31, 17, 17, 17]),
    ('B', [30, 17, 17, 30, 17, 17, 30]),
    ('C', [14, 17, 16, 16, 16, 17, 14]),
    ('D', [30, 17, 17, 17, 17, 17, 30]),
    ('E', [31, 16, 16, 30, 16, 16, 31]),
    ('I', [31, 4, 4, 4, 4, 4, 31]),
    ('L', [16, 16, 16, 16, 16, 16, 31]),
    ('M', [17, 27, 21, 21, 17, 17, 17]),
    ('N', [17, 25, 25, 21, 19, 19, 17]),
    ('O', [14, 17, 17, 17, 17, 17, 14]),
    ('P', [30, 17, 17, 30, 16, 16, 16]),
    ('R', [30, 17, 17, 30, 20, 18, 17]),
    ('S', [15, 16, 16, 14, 1, 1, 30]),
    ('T', [31, 4, 4, 4, 4, 4, 4]),
    ('U', [17, 17, 17, 17, 17, 17, 14]),
    ('V', [17, 17, 17, 17, 17, 10, 4]),
];

fn glyph(character: char) -> [u8; 7] {
    GLYPHS
        .iter()
        .find(|(candidate, _)| *candidate == character)
        .map(|(_, bits)| *bits)
        .unwrap_or([0; 7])
}

/// Build the live neutral frame: Y=32, text=235, UV=128.
///
/// The raster text uses the same 5x7 glyphs, 4x scale and positions as the
/// deployed publisher. This function is deterministic and performs no I/O.
pub fn build_placeholder() -> Frame {
    let mut y = vec![32u8; Y_SIZE];

    for (text, top) in [(CAMERA_UNAVAILABLE, 312usize), (RECONNECTING, 380)] {
        let left = (WIDTH - text.len() * 24) / 2;
        for (index, character) in text.chars().enumerate() {
            let bits = glyph(character);
            for (row, row_bits) in bits.iter().enumerate() {
                for column in 0..5 {
                    if row_bits & (1 << (4 - column)) != 0 {
                        for dy in 0..4 {
                            let offset =
                                (top + row * 4 + dy) * WIDTH + left + index * 24 + column * 4;
                            y[offset..offset + 4].fill(235);
                        }
                    }
                }
            }
        }
    }

    let mut bytes = y;
    bytes.resize(Y_SIZE + 2 * UV_SIZE, 128);
    Frame::from_vec(bytes).expect("placeholder has the fixed I420 size")
}

/// Shared placeholder frame; the publisher can clone it cheaply each cycle.
pub fn placeholder() -> &'static Frame {
    static PLACEHOLDER: OnceLock<Frame> = OnceLock::new();
    PLACEHOLDER.get_or_init(build_placeholder)
}

pub fn placeholder_is_valid(frame: &Frame) -> bool {
    frame.len() == Y_SIZE + 2 * UV_SIZE
}

#[cfg(test)]
mod tests {
    use sha2::{Digest, Sha256};

    use super::*;

    fn hex(bytes: &[u8]) -> String {
        use std::fmt::Write;
        let mut out = String::with_capacity(bytes.len() * 2);
        for byte in bytes {
            write!(&mut out, "{byte:02x}").expect("write to string");
        }
        out
    }

    #[test]
    fn matches_live_placeholder_hash() {
        let frame = build_placeholder();
        let digest = Sha256::digest(frame.as_bytes());
        assert_eq!(hex(&digest), PLACEHOLDER_SHA256);
    }

    #[test]
    fn has_neutral_planes_and_raster_text() {
        let frame = build_placeholder();
        assert!(placeholder_is_valid(&frame));
        assert!(frame.y().iter().all(|pixel| *pixel == 32 || *pixel == 235));
        assert!(frame.y().contains(&235));
        assert!(frame.u().iter().all(|pixel| *pixel == 128));
        assert!(frame.v().iter().all(|pixel| *pixel == 128));
        assert_eq!(frame.len(), 1_382_400);
    }

    #[test]
    fn shared_placeholder_is_stable() {
        let first = placeholder();
        let second = placeholder();
        assert!(std::ptr::eq(first, second));
        assert_eq!(first, &build_placeholder());
    }
}
