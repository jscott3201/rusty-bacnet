//! Device-supplied text, labelled as data (design §7.5).
//!
//! Anyone who can configure a device, or spoof one, sets its names and
//! descriptions; B/IP has no authentication. So every string from the wire
//! comes back as an envelope, never spliced into prose:
//! `{"untrusted": "...", "source": "device 1201 analog-output:3 description",
//! "truncated": false}`. The text inside is cleaned first: control
//! characters (C0 and C1) and line and paragraph separators become spaces,
//! format characters (bidirectional overrides, zero-width and tag
//! characters), variation selectors and Hangul fillers are dropped, URLs
//! are defanged (`hxxp://`), and the result is capped.

use bacnet_types::text::breaks_line;
use serde_json::{Value, json};

/// A device-supplied string as an envelope: cleaned, capped at `cap`
/// characters, and labelled with where it came from.
pub(crate) fn envelope(text: &str, source: impl Into<String>, cap: usize) -> Value {
    let (text, truncated) = clean(text, cap);
    json!({ "untrusted": text, "source": source.into(), "truncated": truncated })
}

/// An envelope for text already cleaned and capped by [`clean`].
pub(crate) fn envelope_cleaned(text: &str, truncated: bool, source: impl Into<String>) -> Value {
    json!({ "untrusted": text, "source": source.into(), "truncated": truncated })
}

/// `text` cleaned and capped; true when it was cut.
pub(crate) fn clean(text: &str, cap: usize) -> (String, bool) {
    let mut cleaned: String = text
        .chars()
        .filter(|ch| !hidden(*ch))
        .map(|ch| if breaks_line(ch) { ' ' } else { ch })
        .collect();
    cleaned = defang(&cleaned);
    match cleaned.char_indices().nth(cap) {
        Some((cut, _)) => {
            cleaned.truncate(cut);
            (cleaned, true)
        }
        None => (cleaned, false),
    }
}

/// The table of [`hidden`] characters, for the tests.
#[cfg(test)]
pub(crate) use bacnet_types::text::HIDDEN;

/// Whether `ch` is dropped: the whole shared set of format characters and
/// others that show as nothing ([`bacnet_types::text::HIDDEN`]), the
/// zero width joiner included, which the CLI, the TUI and the PICS keep.
fn hidden(ch: char) -> bool {
    bacnet_types::text::is_hidden(ch)
}

/// URLs made inert: `http://` and `https://` as `hxxp://` and `hxxps://`,
/// and any other `scheme://` as `scheme[:]//`, so no host renders them as
/// links.
fn defang(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find("://") {
        let (before, after) = rest.split_at(at);
        let scheme_start = before
            .char_indices()
            .rev()
            .find(|(_, ch)| !(ch.is_ascii_alphanumeric() || matches!(ch, '+' | '-' | '.')))
            .map_or(0, |(i, ch)| i + ch.len_utf8());
        let (lead, scheme) = before.split_at(scheme_start);
        out.push_str(lead);
        match scheme.to_ascii_lowercase().as_str() {
            "http" | "https" => {
                // Keep the scheme's own case after the h: HTTP becomes HXXP.
                let x = if scheme.as_bytes()[1].is_ascii_uppercase() {
                    "XX"
                } else {
                    "xx"
                };
                out.push_str(&scheme[..1]);
                out.push_str(x);
                out.push_str(&scheme[3..]);
                out.push_str("://");
            }
            _ => {
                out.push_str(scheme);
                out.push_str("[:]//");
            }
        }
        rest = &after[3..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controls_bidi_and_urls_are_neutralised() {
        let text = "Supply\u{0007} Fan\u{009B}\u{202E}gnp.exe\u{202C} see https://evil.example/x \
                    and ftp://h/f or HTTP://A";
        let (cleaned, truncated) = clean(text, 256);
        assert!(!truncated);
        assert_eq!(
            cleaned,
            "Supply  Fan gnp.exe see hxxps://evil.example/x and ftp[:]//h/f or HXXP://A"
        );
        assert!(!cleaned.chars().any(|ch| ch.is_control() || hidden(ch)));
    }

    /// Every range of the table, at both ends, is dropped; the separators
    /// become spaces; the characters around each range stay.
    #[test]
    fn every_hidden_range_is_dropped() {
        for &(low, high) in HIDDEN {
            for ch in [low, high] {
                let text = format!("a{ch}b");
                assert_eq!(clean(&text, 256).0, "ab", "U+{:04X}", u32::from(ch));
            }
        }
        for named in [
            '\u{E0041}',
            '\u{FE0F}',
            '\u{E0100}',
            '\u{115F}',
            '\u{1160}',
            '\u{3164}',
            '\u{FFA0}',
        ] {
            assert!(hidden(named), "U+{:04X}", u32::from(named));
        }
        assert_eq!(clean("a\u{2028}b\u{2029}c", 256).0, "a b c");
        for kept in ['A', 'é', '\u{4E2D}', '\u{1F600}', '\u{FE10}', '\u{3165}'] {
            assert!(!hidden(kept), "U+{:04X}", u32::from(kept));
        }
    }

    #[test]
    fn long_text_is_capped_by_characters() {
        let text = "é".repeat(300);
        let (cleaned, truncated) = clean(&text, 256);
        assert!(truncated);
        assert_eq!(cleaned.chars().count(), 256);
        let value = envelope("ok", "device 1 name", 256);
        assert_eq!(
            value,
            json!({ "untrusted": "ok", "source": "device 1 name", "truncated": false })
        );
    }

    #[test]
    fn a_bare_separator_is_left_alone() {
        assert_eq!(defang("://x"), "[:]//x");
        assert_eq!(defang("é://x éhttp://y"), "é[:]//x éhxxp://y");
        assert_eq!(defang("no url here"), "no url here");
    }
}
