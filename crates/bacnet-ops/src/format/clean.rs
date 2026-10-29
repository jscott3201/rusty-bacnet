//! Device text made safe to show: names, state texts and other strings a
//! device supplies, which anyone who can configure (or spoof) it sets.
//!
//! A control character can move a terminal's cursor or break a table row,
//! and a format character (a bidirectional override or isolate, a
//! zero-width one) changes how the text around it reads without showing
//! itself: `High` followed by a right-to-left override reverses the rest
//! of the row. [`clean_text`] turns the first into spaces and drops the
//! second, for the one-shot CLI's table and text output; [`clean_name`]
//! does the same with a length cap, for names and state texts in the CLI
//! and the TUI. The rules come from [`bacnet_types::text`], which the MCP
//! server's untrusted envelopes and the PICS share.

use bacnet_types::text::{breaks_line, is_hidden_on_screen};

/// Longest name kept, in characters; a device's longer name is cut.
pub const MAX_NAME_CHARS: usize = 128;

/// A device's name (or other short text) for display: [`clean_text`] cut
/// to at most [`MAX_NAME_CHARS`] characters.
pub fn clean_name(text: &str) -> String {
    shown(text).take(MAX_NAME_CHARS).collect()
}

/// Device text for display, whole: each control character and line or
/// paragraph separator as a space, so it can't move the cursor or break a
/// row, and each character [`is_hidden_on_screen`] matches dropped, so it can't
/// reorder or hide what follows. The one-shot CLI's table and text output
/// shows every device string this way (#2144); nothing marks a changed one.
pub fn clean_text(text: &str) -> String {
    shown(text).collect()
}

/// The characters of `text` a screen shows, in order.
fn shown(text: &str) -> impl Iterator<Item = char> + '_ {
    text.chars()
        .filter(|ch| !is_hidden_on_screen(*ch))
        .map(|ch| if breaks_line(ch) { ' ' } else { ch })
}

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_types::text::HIDDEN;

    /// Controls become spaces; bidirectional overrides and isolates
    /// (U+202E, U+2066 to U+2069) and the zero-width ones are dropped, so
    /// they can't reverse or hide the rest of a row (#2122).
    #[test]
    fn controls_become_spaces_and_format_characters_go() {
        assert_eq!(clean_name("Zone\nTemp\u{1b}"), "Zone Temp ");
        assert_eq!(clean_name("a\u{2028}b\u{2029}c"), "a b c");
        assert_eq!(clean_name("High\u{202E} 1"), "High 1");
        assert_eq!(
            clean_name("\u{2066}a\u{2067}b\u{2068}c\u{2069}\u{200B}d\u{FEFF}"),
            "abcd"
        );
        // The zero width joiner keeps an emoji family one picture.
        let family = "\u{1F468}\u{200D}\u{1F469}";
        assert_eq!(clean_name(family), family);
        for &(low, high) in HIDDEN {
            for ch in [low, high].into_iter().filter(|ch| *ch != '\u{200D}') {
                assert_eq!(
                    clean_name(&format!("a{ch}b")),
                    "ab",
                    "U+{:04X}",
                    u32::from(ch)
                );
            }
        }
        // Dropped characters don't count toward the cap.
        let long = format!("{}{}", "\u{202E}".repeat(10), "é".repeat(500));
        assert_eq!(clean_name(&long), "é".repeat(MAX_NAME_CHARS));
    }

    /// [`clean_text`] cleans as [`clean_name`] does but keeps every shown
    /// character (#2144): a Description longer than a name stays whole.
    #[test]
    fn clean_text_keeps_every_shown_character() {
        let hostile = "A\u{202E}B\u{1b}[2JC\u{7}D\nE\u{2028}F\u{1F468}\u{200D}\u{1F469}";
        let cleaned = "AB [2JC D E F\u{1F468}\u{200D}\u{1F469}";
        assert_eq!(clean_text(hostile), cleaned);
        assert_eq!(clean_name(hostile), cleaned);
        let long = format!("{}{}", "\u{202E}".repeat(10), "é".repeat(500));
        assert_eq!(clean_text(&long), "é".repeat(500));
    }
}
