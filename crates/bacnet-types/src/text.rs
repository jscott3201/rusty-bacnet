//! Characters that device text must not carry as they are onto a screen or
//! into a document (#2148).
//!
//! A device's name, state texts, description and other strings are set by
//! whoever can configure (or spoof) it. A control character in one can move
//! a terminal's cursor or end a table row, and a format character (a
//! bidirectional override or isolate, a zero-width one) changes how the
//! text around it reads without showing itself. Every place that shows
//! device text (the CLI and the TUI through `bacnet_ops::format`, the MCP
//! server's untrusted envelopes, the text and Markdown PICS) takes its
//! rules from this one table, so they can't drift apart.
//!
//! No allocation: a `const` table and `char` predicates, so it builds
//! without `std` or `alloc`.

/// The characters dropped from device text: every format character
/// (general category Cf, Unicode 16: the bidirectional embeddings,
/// overrides, isolates and marks, the zero-width ones, the tag characters
/// and the rest), the variation selectors, the Hangul fillers, and a few
/// others that show as nothing. Inclusive ranges.
pub const HIDDEN: &[(char, char)] = &[
    // Format characters (Cf).
    ('\u{00AD}', '\u{00AD}'),
    ('\u{0600}', '\u{0605}'),
    ('\u{061C}', '\u{061C}'),
    ('\u{06DD}', '\u{06DD}'),
    ('\u{070F}', '\u{070F}'),
    ('\u{0890}', '\u{0891}'),
    ('\u{08E2}', '\u{08E2}'),
    ('\u{180E}', '\u{180E}'),
    ('\u{200B}', '\u{200F}'),
    ('\u{202A}', '\u{202E}'),
    ('\u{2060}', '\u{2064}'),
    ('\u{2066}', '\u{206F}'),
    ('\u{FEFF}', '\u{FEFF}'),
    ('\u{FFF9}', '\u{FFFB}'),
    ('\u{110BD}', '\u{110BD}'),
    ('\u{110CD}', '\u{110CD}'),
    ('\u{13430}', '\u{1343F}'),
    ('\u{1BCA0}', '\u{1BCA3}'),
    ('\u{1D173}', '\u{1D17A}'),
    // The tag block, Cf and unassigned alike.
    ('\u{E0000}', '\u{E007F}'),
    // Variation selectors, Mongolian ones included.
    ('\u{180B}', '\u{180D}'),
    ('\u{180F}', '\u{180F}'),
    ('\u{FE00}', '\u{FE0F}'),
    ('\u{E0100}', '\u{E01EF}'),
    // Hangul fillers.
    ('\u{115F}', '\u{1160}'),
    ('\u{3164}', '\u{3164}'),
    ('\u{FFA0}', '\u{FFA0}'),
    // Others that show as nothing: the combining grapheme joiner, the
    // Khmer inherent vowels, the blank Braille pattern, and the
    // unassigned code points before the interlinear annotations.
    ('\u{034F}', '\u{034F}'),
    ('\u{17B4}', '\u{17B5}'),
    ('\u{2800}', '\u{2800}'),
    ('\u{FFF0}', '\u{FFF8}'),
];

/// Whether `ch` is in [`HIDDEN`]. The MCP server's untrusted envelopes,
/// read by a model rather than a person, drop every one of them.
pub fn is_hidden(ch: char) -> bool {
    HIDDEN.iter().any(|&(low, high)| (low..=high).contains(&ch))
}

/// Whether a screen or a printed document drops `ch`: a [`HIDDEN`]
/// character other than the zero width joiner (U+200D). The joiner only
/// fuses an emoji sequence (a family, a flag) into one picture, and can't
/// reorder or hide the text around it, so the CLI, the TUI and the PICS
/// keep it.
pub fn is_hidden_on_screen(ch: char) -> bool {
    ch != '\u{200D}' && is_hidden(ch)
}

/// Whether `ch` breaks a line or moves the cursor: a control (C0, DEL and
/// C1) or a line or paragraph separator. One-line renderings show it as a
/// space.
pub fn breaks_line(ch: char) -> bool {
    ch.is_control() || matches!(ch, '\u{2028}' | '\u{2029}')
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each range is ordered, both its ends are hidden, and the screens'
    /// set is the full set less the zero width joiner.
    #[test]
    fn ranges_hold_and_only_the_joiner_differs_on_screen() {
        for &(low, high) in HIDDEN {
            assert!(low <= high, "U+{:04X}", u32::from(low));
            for ch in [low, high] {
                assert!(is_hidden(ch), "U+{:04X}", u32::from(ch));
                assert_eq!(is_hidden_on_screen(ch), ch != '\u{200D}');
            }
        }
        assert!(is_hidden('\u{200D}'));
        assert!(!is_hidden_on_screen('\u{200D}'));
        for ch in ['a', ' ', '\u{1F468}', '\u{00AC}', '\u{00AE}', '\u{FFFC}'] {
            assert!(!is_hidden(ch), "U+{:04X}", u32::from(ch));
        }
    }

    /// Controls (C0, DEL, C1) and the line and paragraph separators break a
    /// line; ordinary and format characters don't.
    #[test]
    fn line_breaks() {
        for ch in [
            '\n', '\r', '\t', '\u{1b}', '\u{7f}', '\u{85}', '\u{9f}', '\u{2028}', '\u{2029}',
        ] {
            assert!(breaks_line(ch), "U+{:04X}", u32::from(ch));
        }
        for ch in ['a', ' ', '\u{a0}', '\u{202E}', '\u{200B}'] {
            assert!(!breaks_line(ch), "U+{:04X}", u32::from(ch));
        }
    }
}
