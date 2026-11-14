//! Octets as text: the one hex formatter the CLI, the TUI and the MCP server
//! share. Each function only formats; nothing here reads text back (the
//! parsers sit with their callers).

/// `bytes` as lower-case hex with no separator: `0a1b`.
pub fn hex(bytes: &[u8]) -> String {
    hex_sep(bytes, "")
}

/// `bytes` as lower-case hex, the octets joined by `separator`: with `":"`,
/// `0a:1b`.
pub fn hex_sep(bytes: &[u8], separator: &str) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(bytes.len() * (2 + separator.len()));
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 {
            out.push_str(separator);
        }
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// `bytes` as upper-case hex with no separator: `0A1B`.
pub fn hex_upper(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(out, "{b:02X}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_form_is_pinned() {
        let cases: [(&[u8], &str, &str, &str); 4] = [
            (&[], "", "", ""),
            (&[0x0a], "0a", "0a", "0A"),
            (&[0x00, 0xff, 0x1b], "00ff1b", "00:ff:1b", "00FF1B"),
            (
                &[0xde, 0xad, 0xbe, 0xef],
                "deadbeef",
                "de:ad:be:ef",
                "DEADBEEF",
            ),
        ];
        for (bytes, plain, colon, upper) in cases {
            assert_eq!(hex(bytes), plain);
            assert_eq!(hex_sep(bytes, ":"), colon);
            assert_eq!(hex_sep(bytes, ""), plain);
            assert_eq!(hex_upper(bytes), upper);
        }
        assert_eq!(hex_sep(&[1, 2, 3], " "), "01 02 03");
        assert_eq!(hex_sep(&[1, 2, 3], ", "), "01, 02, 03");
    }
}
