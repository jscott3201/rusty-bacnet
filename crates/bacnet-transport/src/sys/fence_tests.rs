//! The fence around `sys` (#1923): no Rust source of this crate outside
//! `src/sys/` may hold unsafe code, nor allow the `unsafe_code` lint. The
//! lint alone can't hold the line, since a new `#[allow(unsafe_code)]`
//! anywhere silences it; this test fails on one.

use std::path::{Path, PathBuf};

/// Every `.rs` file under `dir`, skipping `skip` and build output.
fn rust_files(dir: &Path, skip: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path == skip || path.file_name().is_some_and(|name| name == "target") {
            continue;
        }
        if path.is_dir() {
            rust_files(&path, skip, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

/// Whether `code` (one line, comments removed) holds the keyword `unsafe`
/// as a word of its own, or names the `unsafe_code` lint anywhere but
/// straight after `deny(` or `forbid(`. Strings count as code, so this fails
/// closed on them.
fn holds_unsafe(code: &str) -> bool {
    let word = |c: char| c.is_alphanumeric() || c == '_';
    let keyword = code.match_indices("unsafe").any(|(at, found)| {
        let before = code[..at].chars().next_back();
        let after = code[at + found.len()..].chars().next();
        !before.is_some_and(word) && !after.is_some_and(word)
    });
    let lint = code.match_indices("unsafe_code").any(|(at, _)| {
        let before = &code[..at];
        !before.ends_with("deny(") && !before.ends_with("forbid(")
    });
    keyword || lint
}

/// The line's code, without a `//` comment (doc comments included). A `//`
/// inside a string literal is code; `in_string` carries an open string from
/// one line to the next.
fn code_of<'a>(line: &'a str, in_string: &mut bool) -> &'a str {
    let chars: Vec<(usize, char)> = line.char_indices().collect();
    let char_at = |i: usize| chars.get(i).map(|&(_, c)| c);
    let mut i = 0;
    while let Some(&(at, c)) = chars.get(i) {
        let next = char_at(i + 1);
        if *in_string {
            match c {
                '\\' => i += 1,
                '"' => *in_string = false,
                _ => {}
            }
        } else {
            match c {
                '/' if next == Some('/') => return &line[..at],
                '"' => *in_string = true,
                // An escaped char literal ('\\', '\''): skip to its end.
                '\'' if next == Some('\\') => {
                    i += 3;
                    while char_at(i).is_some_and(|c| c != '\'') {
                        i += 1;
                    }
                }
                // A plain char literal ('"'), not a lifetime.
                '\'' if char_at(i + 2) == Some('\'') => i += 2,
                _ => {}
            }
        }
        i += 1;
    }
    line
}

/// The lines of `text` that hold unsafe code, numbered from 1.
fn offending_lines(text: &str) -> Vec<(usize, &str)> {
    let mut in_string = false;
    text.lines()
        .enumerate()
        .filter(|(_, line)| holds_unsafe(code_of(line, &mut in_string)))
        .map(|(number, line)| (number + 1, line))
        .collect()
}

#[test]
fn unsafe_stays_in_sys() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let sys = root.join("src").join("sys");
    let mut files = Vec::new();
    rust_files(root, &sys, &mut files);
    assert!(files.len() > 50, "found only {} files", files.len());
    let offences: Vec<String> = files
        .iter()
        .flat_map(|file| {
            let text = std::fs::read_to_string(file).unwrap();
            offending_lines(&text)
                .into_iter()
                .map(|(number, line)| format!("{}:{number}: {}", file.display(), line.trim()))
                .collect::<Vec<_>>()
        })
        .collect();
    assert!(
        offences.is_empty(),
        "unsafe code belongs in src/sys/ (#1923):\n{}",
        offences.join("\n")
    );
}

#[test]
fn the_fence_sees_unsafe_and_lets_prose_by() {
    for caught in [
        "let x = unsafe { f() };",
        "unsafe fn f() {}",
        "unsafe impl Send for T {}",
        "unsafe extern \"C\" {",
        "#[unsafe(no_mangle)]",
        "#[allow(unsafe_code)]",
        "#![allow(unsafe_code)]",
        "#[cfg_attr(test, allow(unsafe_code))]",
        "#[expect(unsafe_code)]",
        "#[allow(unsafe_code)] #[deny(missing_docs)]",
        "let u = \"http://x\"; #[allow(unsafe_code)]",
        "let c = '\"'; #[allow(unsafe_code)]",
        "let c = '\\''; #[allow(unsafe_code)]",
        "let s = \"unsafe { }\";",
    ] {
        assert_eq!(offending_lines(caught).len(), 1, "{caught}");
    }
    // A string over two lines hides no `//` from the second.
    assert_eq!(
        offending_lines("let s = \"a\n// b\"; #[allow(unsafe_code)]"),
        [(2, "// b\"; #[allow(unsafe_code)]")]
    );
    for passed in [
        "#![forbid(unsafe_code)]",
        "#![deny(unsafe_code)]",
        "/// Unsafe code lives in sys; no unsafe here.",
        "let x = 1; // unsafe { } in a comment",
        "#[deny(unsafe_op_in_unsafe_fn)]",
        "let unsafe_count = 0;",
        "fn f<'a>(x: &'a str) {} // unsafe",
        "let s = \"a \\\" b\"; // unsafe",
    ] {
        assert!(offending_lines(passed).is_empty(), "{passed}");
    }
}
