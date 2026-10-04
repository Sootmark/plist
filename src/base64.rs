//! Base64, as XML property lists hold data: the standard alphabet, lines
//! broken and indented anywhere, padding optional.

/// The bytes `text` encodes, and what was wrong with it, if anything.
/// Whitespace is skipped silently; other characters outside the alphabet
/// are skipped and reported, as is a final character that completes no
/// byte. Decoding stops at the first `=`.
pub(crate) fn decode(text: &str) -> (Vec<u8>, Option<String>) {
    let mut bytes = Vec::with_capacity(text.len() / 4 * 3);
    let mut bits = 0u32;
    let mut pending = 0u32;
    let mut skipped = 0usize;
    let mut padded = false;
    for byte in text.bytes() {
        if byte.is_ascii_whitespace() {
            continue;
        }
        if byte == b'=' {
            padded = true;
            continue;
        }
        let Some(sextet) = sextet(byte).filter(|_| !padded) else {
            skipped += 1;
            continue;
        };
        bits = (bits << 6) | u32::from(sextet);
        pending += 6;
        if pending >= 8 {
            pending -= 8;
            bytes.push((bits >> pending) as u8);
        }
    }
    let problem = match (skipped, pending) {
        (0, 0 | 2 | 4) => None,
        (0, _) => Some("base64 ends with a character that completes no byte".to_owned()),
        (skipped, _) => Some(format!(
            "{skipped} characters outside the base64 alphabet (or after its padding) skipped"
        )),
    };
    (bytes, problem)
}

fn sextet(byte: u8) -> Option<u8> {
    match byte {
        b'A'..=b'Z' => Some(byte - b'A'),
        b'a'..=b'z' => Some(byte - b'a' + 26),
        b'0'..=b'9' => Some(byte - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_with_and_without_padding() {
        assert_eq!(decode(""), (vec![], None));
        assert_eq!(decode("TWFu"), (b"Man".to_vec(), None));
        assert_eq!(decode("TWE="), (b"Ma".to_vec(), None));
        assert_eq!(decode("TQ=="), (b"M".to_vec(), None));
        assert_eq!(decode("TWE"), (b"Ma".to_vec(), None));
        assert_eq!(decode("+/+/"), (vec![0xfb, 0xff, 0xbf], None));
    }

    #[test]
    fn whitespace_is_skipped_anywhere() {
        assert_eq!(decode("\n\tTW\r\n  Fu\n"), (b"Man".to_vec(), None));
    }

    #[test]
    fn stray_characters_are_reported() {
        let (bytes, problem) = decode("TW*Fu");
        assert_eq!(bytes, b"Man");
        assert!(problem.unwrap().starts_with("1 characters"));
        assert!(decode("TWFuT").1.is_some());
        assert!(decode("TQ==TQ").1.is_some());
    }
}
