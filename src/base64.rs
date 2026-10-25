//! Standard base64 (with `=` padding), decoding only: what the private sway sends in the tick events of pointer pictures.

pub fn decode(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let mut bits = 0u32;
    let mut count = 0;
    let mut padding = 0;
    for b in text.bytes() {
        let v = match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => {
                padding += 1;
                continue;
            }
            _ => return None,
        };
        if padding > 0 {
            return None; // data after the padding
        }
        bits = bits << 6 | u32::from(v);
        count += 1;
        if count == 4 {
            out.extend_from_slice(&[(bits >> 16) as u8, (bits >> 8) as u8, bits as u8]);
            bits = 0;
            count = 0;
        }
    }
    match (count, padding) {
        (0, 0) => {}
        (2, 2) => out.push((bits >> 4) as u8),
        (3, 1) => out.extend_from_slice(&[(bits >> 10) as u8, (bits >> 2) as u8]),
        _ => return None,
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::decode;

    #[test]
    fn decodes_with_and_without_padding_bytes() {
        assert_eq!(decode("").unwrap(), b"");
        assert_eq!(decode("TQ==").unwrap(), b"M");
        assert_eq!(decode("TWE=").unwrap(), b"Ma");
        assert_eq!(decode("TWFu").unwrap(), b"Man");
        assert_eq!(decode("AAECAwQF").unwrap(), [0, 1, 2, 3, 4, 5]);
    }

    #[test]
    fn refuses_bad_text() {
        assert!(decode("T").is_none());
        assert!(decode("TQ=x").is_none());
        assert!(decode("T!==").is_none());
        assert!(decode("TQ==TQ==").is_none());
    }
}
