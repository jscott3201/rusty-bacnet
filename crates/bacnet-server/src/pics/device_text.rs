//! Device strings as the text and Markdown PICS print them (#2137).
//!
//! Vendor_Name, Model_Name, Firmware_Revision and Application_Software_Version
//! come from the Device object, which an application or a custom Device can
//! fill with any text. Each prints inside one line of the document, so a value
//! must not be able to end that line or, in Markdown, its table cell, or start
//! inline markup: otherwise a value could forge rows or sections. Nor may it
//! hide or reorder its own text with a bidirectional or zero-width character.
//! `VendorInfo` keeps the values as read; only the rendering changes them.
//!
//! The rules are the CLI's and the TUI's, shared through
//! [`bacnet_types::text`] (#2148), without their length cap: line breaks
//! become spaces and the hidden characters are dropped, except the zero width
//! joiner, which only fuses an emoji sequence.

use bacnet_types::text::{breaks_line, is_hidden_on_screen};

/// The value's characters as they print on one line: line breaks as spaces,
/// [`is_hidden_on_screen`] characters gone.
fn one_line(value: &str) -> impl Iterator<Item = char> + '_ {
    value
        .chars()
        .filter(|c| !is_hidden_on_screen(*c))
        .map(|c| if breaks_line(c) { ' ' } else { c })
}

/// The value as one line of the text PICS, as [`one_line`] gives it: it
/// can't start a line of its own or hide or reorder its text.
pub(super) fn text_line(value: &str) -> String {
    one_line(value).collect()
}

/// The value as one Markdown table cell: one line, as [`text_line`] gives it,
/// with a backslash before each character that could end the cell (`|`) or
/// open inline markup: emphasis, code, links, raw HTML, strikethrough or
/// entity references. The backslash itself is escaped too.
pub(super) fn markdown_cell(value: &str) -> String {
    let mut cell = String::with_capacity(value.len());
    for c in one_line(value) {
        if matches!(
            c,
            '\\' | '|' | '`' | '*' | '_' | '[' | ']' | '<' | '>' | '~' | '&'
        ) {
            cell.push('\\');
        }
        cell.push(c);
    }
    cell
}
