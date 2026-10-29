//! Strict extraction shared by the build script and source-contract tests.
//!
//! This is a guide-specific profile, not a general Markdown/MDX parser. It
//! accepts column-zero backtick/tilde fences, delimiter lengths of at least
//! three, plain ASCII language tokens, and optional first-line YAML front matter.
//! Target labels are case-insensitive and may have surrounding spaces/tabs, but
//! target metadata is rejected. Matching closers may have up to three spaces of
//! indentation and trailing spaces/tabs. Content bytes are never normalized.
//!
//! Outside fences/front matter, indentation, lists, block quotes, directives,
//! reference definitions, and HTML/autolinks are rejected. This deliberately
//! excludes containers that could hide, supply, or alter a displayed sample.
//! Other fenced blocks and front matter are opaque, including apparent samples
//! inside them. Unsupported syntax fails the build instead of being ignored.

#[derive(Clone, Copy)]
struct Fence {
    marker: u8,
    width: usize,
    wanted: bool,
    start: usize,
}

fn delimiter(line: &[u8]) -> Option<(u8, usize, &[u8])> {
    let marker = *line.first()?;
    if !matches!(marker, b'`' | b'~') {
        return None;
    }
    let width = line.iter().take_while(|byte| **byte == marker).count();
    (width >= 3).then_some((marker, width, &line[width..]))
}

fn horizontal(byte: &u8) -> bool {
    matches!(byte, b' ' | b'\t')
}

fn trim_horizontal(bytes: &[u8]) -> &[u8] {
    let start = bytes
        .iter()
        .position(|byte| !horizontal(byte))
        .unwrap_or(bytes.len());
    let end = bytes
        .iter()
        .rposition(|byte| !horizontal(byte))
        .map_or(start, |index| index + 1);
    &bytes[start..end]
}

fn closes(line: &[u8], fence: Fence) -> bool {
    let indent = line.iter().take_while(|byte| **byte == b' ').count();
    if indent > 3 {
        return false;
    }
    delimiter(&line[indent..]).is_some_and(|(marker, width, rest)| {
        marker == fence.marker && width >= fence.width && rest.iter().all(horizontal)
    })
}

fn unsupported_context(line: &[u8]) -> bool {
    if line.iter().all(horizontal) {
        return false;
    }
    let indented = line.first().is_some_and(horizontal);
    let bullet = line
        .first()
        .is_some_and(|byte| matches!(byte, b'-' | b'+' | b'*'))
        && (line.len() == 1 || horizontal(&line[1]));
    let digits = line.iter().take_while(|byte| byte.is_ascii_digit()).count();
    let ordered = digits > 0
        && line
            .get(digits)
            .is_some_and(|byte| matches!(byte, b'.' | b')'))
        && (line.len() == digits + 1 || line.get(digits + 1).is_some_and(horizontal));
    // A line beginning '[' must be an ordinary inline link in this profile.
    // Reject reference definitions and incomplete/multiline reference labels.
    let reference = line.starts_with(b"[")
        && (line.windows(2).any(|pair| pair == b"]:")
            || !line.windows(2).any(|pair| pair == b"]("));
    indented
        || bullet
        || ordered
        || reference
        || line.starts_with(b">")
        || line.starts_with(b":::")
        || line.contains(&b'<')
        || line.starts_with(b"---")
        || line.starts_with(b"+++")
}

/// Return the sole nonempty target-language fence under the profile above.
/// Missing, duplicate, empty, unclosed, or unsupported input is an error.
pub fn extract<'a>(document: &'a [u8], language: &str) -> Result<&'a [u8], String> {
    std::str::from_utf8(document).map_err(|_| "guide must be UTF-8".to_string())?;
    if document.starts_with(b"\xef\xbb\xbf")
        || document.iter().enumerate().any(|(index, byte)| {
            (*byte == b'\r' && document.get(index + 1) != Some(&b'\n'))
                || (*byte < b' ' && !matches!(byte, b'\n' | b'\r' | b'\t'))
        })
    {
        return Err("unsupported guide encoding or line ending".into());
    }
    let mut active: Option<Fence> = None;
    let mut front_matter = false;
    let mut seen = false;
    let mut offset = 0;
    let mut result = None;
    for line in document.split_inclusive(|byte| *byte == b'\n') {
        let marker = line.strip_suffix(b"\n").unwrap_or(line);
        let marker = marker.strip_suffix(b"\r").unwrap_or(marker);
        if offset == 0 && marker == b"---" {
            front_matter = true;
        } else if front_matter {
            if marker == b"---" {
                front_matter = false;
            } else if marker.starts_with(b"---") || marker.starts_with(b"...") {
                return Err("unsupported front matter delimiter".into());
            }
        } else if let Some(fence) = active {
            if closes(marker, fence) {
                if fence.wanted {
                    let content = &document[fence.start..offset];
                    if content.iter().all(u8::is_ascii_whitespace) {
                        return Err(format!("empty {language} fence"));
                    }
                    result = Some(content);
                }
                active = None;
            }
        } else {
            if unsupported_context(marker) {
                return Err("unsupported guide container or HTML context".into());
            }
            if let Some((kind, width, info)) = delimiter(marker) {
                if kind == b'`' && info.contains(&b'`') {
                    return Err("unsupported backtick in fence info".into());
                }
                let info = trim_horizontal(info);
                let token_end = info.iter().position(horizontal).unwrap_or(info.len());
                let token = &info[..token_end];
                if !token
                    .iter()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'+'))
                {
                    return Err("unsupported fence language token".into());
                }
                let wanted = token.eq_ignore_ascii_case(language.as_bytes());
                if wanted {
                    if seen {
                        return Err(format!("duplicate {language} fence"));
                    }
                    if !trim_horizontal(&info[token_end..]).is_empty() {
                        return Err(format!("unsupported {language} fence metadata"));
                    }
                    seen = true;
                }
                active = Some(Fence {
                    marker: kind,
                    width,
                    wanted,
                    start: offset + line.len(),
                });
            }
        }
        offset += line.len();
    }
    if front_matter {
        return Err("unclosed front matter".into());
    }
    if let Some(fence) = active {
        return Err(if fence.wanted {
            format!("unclosed {language} fence")
        } else {
            "unclosed unrelated fence".into()
        });
    }
    result.ok_or_else(|| format!("missing {language} fence"))
}
