//! The crate's unsafe code stays in `attach::win32` (#2201), the one
//! module the workspace's `deny(unsafe_code)` is lifted for: an unsafe
//! block, function, impl or extern, or another allow, anywhere else fails
//! here, on every platform, though the lint only sees the code compiled
//! for its own. This file is left out of the scan, as it names them.

use std::path::{Path, PathBuf};

/// Every `.rs` file under `dir`.
fn rust_files(dir: &Path, found: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, found);
        } else if path.extension().is_some_and(|e| e == "rs") {
            found.push(path);
        }
    }
}

/// `line` up to a `//` comment that isn't inside a string literal.
fn code_of(line: &str) -> &str {
    let bytes = line.as_bytes();
    let mut in_string = false;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' if in_string => i += 1,
            b'"' => in_string = !in_string,
            b'/' if !in_string && bytes.get(i + 1) == Some(&b'/') => return &line[..i],
            _ => {}
        }
        i += 1;
    }
    line
}

/// Whether `code` holds unsafe code or lifts the lint.
fn holds_unsafe(code: &str) -> bool {
    [
        "unsafe {",
        "unsafe fn",
        "unsafe impl",
        "unsafe extern",
        "#[unsafe(",
        "allow(unsafe_code)",
        "expect(unsafe_code)",
    ]
    .iter()
    .any(|pattern| code.contains(pattern))
}

#[test]
fn unsafe_stays_in_win32() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let fenced = [
        src.join("attach").join("win32.rs"),
        src.join("fence_tests.rs"),
    ];
    let mut files = Vec::new();
    rust_files(&src, &mut files);
    assert!(files.len() > 30, "found only {} files", files.len());
    let offences: Vec<String> = files
        .iter()
        .filter(|file| !fenced.contains(file))
        .flat_map(|file| {
            let text = std::fs::read_to_string(file).unwrap();
            text.lines()
                .enumerate()
                .filter(|(_, line)| holds_unsafe(code_of(line)))
                .map(|(n, line)| format!("{}:{}: {}", file.display(), n + 1, line.trim()))
                .collect::<Vec<_>>()
        })
        .collect();
    assert!(
        offences.is_empty(),
        "unsafe code belongs in src/attach/win32.rs:\n{}",
        offences.join("\n")
    );
}

#[test]
fn the_fence_sees_unsafe_and_lets_prose_by() {
    for caught in [
        "let x = unsafe { f() };",
        "unsafe fn f() {}",
        "unsafe impl Send for T {}",
        "#![allow(unsafe_code)]",
        "let s = \"a // b\"; unsafe { f() }",
    ] {
        assert!(holds_unsafe(code_of(caught)), "{caught}");
    }
    for passed in [
        "#![forbid(unsafe_code)]",
        "/// No unsafe { } here.",
        "let x = 1; // unsafe { } in a comment",
    ] {
        assert!(!holds_unsafe(code_of(passed)), "{passed}");
    }
}
