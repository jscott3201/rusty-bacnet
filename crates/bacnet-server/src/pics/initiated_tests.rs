//! [`PicsGenerator::INITIATED_SERVICES`] matches the requests the server
//! builds for the wire.
//!
//! The test reads the crate's non-test source, finds every outbound request
//! PDU it constructs (a `ConfirmedRequest { .. }` or `UnconfirmedRequest
//! { .. }` literal, under those names or their `..Pdu` aliases), and compares
//! their services with the list, both ways: a new outbound path can't skip
//! the list, and the list can't name a service the server never sends.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use bacnet_types::enums::{ConfirmedServiceChoice, ServiceSupported, UnconfirmedServiceChoice};

use super::*;

/// The struct names a request literal may use: the encoding crate's names
/// and the aliases this crate imports them under.
const REQUEST_TYPES: &[&str] = &[
    "ConfirmedRequest",
    "UnconfirmedRequest",
    "ConfirmedRequestPdu",
    "UnconfirmedRequestPdu",
];

/// Request literals whose service isn't a `..ServiceChoice::X` literal:
/// (file, the `service_choice` expression, the services it can be, why).
/// Any other non-literal service fails the test.
const NON_LITERAL: &[(&str, &str, &[ServiceSupported], &str)] = &[
    (
        "remote_writes.rs",
        "service_choice",
        &[
            ServiceSupported::WRITE_PROPERTY,
            ServiceSupported::READ_PROPERTY,
        ],
        "a remote write, or the read a Channel makes first",
    ),
    (
        "segmented_receive.rs",
        "first.service_choice",
        &[],
        "reassembles a request received in segments, so initiates nothing",
    ),
];

/// Drops each `#[cfg(test)] mod name { .. }` block, so inline tests don't
/// count. Everything else stays, including `#[cfg(test)]` on single items.
/// A copy lives in `bacnet-client`'s `client/sent_services_tests.rs`; keep
/// the two alike.
pub(crate) fn strip_test_modules(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let mut rest = source;
    while let Some(at) = rest.find("#[cfg(test)]") {
        let after = &rest[at + "#[cfg(test)]".len()..];
        let trimmed = after.trim_start();
        let is_inline_module = trimmed.starts_with("mod ")
            && trimmed
                .find(['{', ';'])
                .is_some_and(|i| trimmed.as_bytes()[i] == b'{');
        if !is_inline_module {
            out.push_str(&rest[..at + "#[cfg(test)]".len()]);
            rest = after;
            continue;
        }
        out.push_str(&rest[..at]);
        let open = after.find('{').expect("checked above");
        rest = &after[open + 1 + braced(&after[open + 1..]).len() + 1..];
    }
    out.push_str(rest);
    out
}

/// The text after an opening brace up to its matching closing brace, or
/// all of it if unmatched.
fn braced(text: &str) -> &str {
    let mut depth = 1usize;
    for (index, ch) in text.char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return &text[..index];
                }
            }
            _ => {}
        }
    }
    text
}

/// The crate's non-test source files, keyed by their path under `src/`.
/// Left out: test-named files and directories ([`test_named`]), every file
/// a `#[cfg(test)] mod name;` declaration loads, and every file a test file
/// declares as a module (by name or `#[path]`). Only the path under `src/`
/// is read, so a checkout whose own path says "test" changes nothing
/// (#2224).
fn sources() -> Vec<(PathBuf, String)> {
    sources_under(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"))
}

/// [`sources`] for the source tree at `root`.
fn sources_under(root: &Path) -> Vec<(PathBuf, String)> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<(PathBuf, String)>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(root, &path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let source = std::fs::read_to_string(&path).unwrap();
                out.push((path.strip_prefix(root).unwrap().to_path_buf(), source));
            }
        }
    }
    let mut all = Vec::new();
    walk(root, root, &mut all);
    let mut tests: BTreeSet<PathBuf> = all
        .iter()
        .map(|(path, _)| path.clone())
        .filter(|path| test_named(path))
        .collect();
    // A test file's modules are test files too, so repeat until none is new.
    loop {
        let before = tests.len();
        for (path, source) in &all {
            let in_test = tests.contains(path);
            for (cfg_test, child) in declared_modules(path, source) {
                if in_test || cfg_test {
                    tests.insert(child);
                }
            }
        }
        if tests.len() == before {
            break;
        }
    }
    all.into_iter()
        .filter(|(path, _)| !tests.contains(path))
        .collect()
}

/// Whether a path under `src/` follows a test-file naming convention: a
/// component named `tests` (a directory or `tests.rs`), or one whose name,
/// less `.rs`, ends `_tests` or `_test_support` or starts `test_`.
fn test_named(relative: &Path) -> bool {
    relative.components().any(|component| {
        let name = component.as_os_str().to_string_lossy();
        let stem = name.strip_suffix(".rs").unwrap_or(&name);
        stem == "tests"
            || stem.ends_with("_tests")
            || stem.ends_with("_test_support")
            || stem.starts_with("test_")
    })
}

/// The out-of-line modules `source` (at `path` under `src/`) declares, one
/// `mod name;` line each, with whether the attribute lines just above it
/// include `#[cfg(test)]`. A `#[path]` attribute names the file relative to
/// the declaring file's directory; otherwise it is `name.rs` or
/// `name/mod.rs`, beside a `lib.rs` or `mod.rs` and under a directory named
/// after any other file.
fn declared_modules(path: &Path, source: &str) -> Vec<(bool, PathBuf)> {
    let parent = path.parent().unwrap_or(Path::new(""));
    let owner = match path.file_name().and_then(|name| name.to_str()) {
        Some("lib.rs" | "mod.rs" | "main.rs") => parent.to_path_buf(),
        _ => parent.join(path.file_stem().unwrap()),
    };
    let lines: Vec<&str> = source.lines().map(str::trim).collect();
    let mut found = Vec::new();
    for (at, line) in lines.iter().enumerate() {
        let declaration = line
            .strip_prefix("pub ")
            .or_else(|| line.strip_prefix("pub(crate) "))
            .or_else(|| line.strip_prefix("pub(super) "))
            .unwrap_or(line);
        let Some(name) = declaration
            .strip_prefix("mod ")
            .and_then(|rest| rest.strip_suffix(';'))
        else {
            continue;
        };
        let attributes = lines[..at]
            .iter()
            .rev()
            .take_while(|line| line.starts_with("#["));
        let mut cfg_test = false;
        let mut file = None;
        for attribute in attributes {
            cfg_test |= *attribute == "#[cfg(test)]";
            if let Some(named) = attribute
                .strip_prefix("#[path = \"")
                .and_then(|rest| rest.strip_suffix("\"]"))
            {
                file = Some(parent.join(named));
            }
        }
        match file {
            Some(file) => found.push((cfg_test, file)),
            // Whichever of the two exists; naming the other too is harmless.
            None => {
                found.push((cfg_test, owner.join(format!("{name}.rs"))));
                found.push((cfg_test, owner.join(name).join("mod.rs")));
            }
        }
    }
    found
}

/// The service bit a `ConfirmedServiceChoice::X` or
/// `UnconfirmedServiceChoice::X` expression names.
fn bit_of(expression: &str) -> Option<u8> {
    let (kind, name) = expression.trim().split_once("ServiceChoice::")?;
    let name: String = name
        .chars()
        .take_while(|c| c.is_ascii_uppercase() || *c == '_')
        .collect();
    let bit = if kind.ends_with("Unconfirmed") {
        ServiceSupported::from_unconfirmed_choice(name.parse::<UnconfirmedServiceChoice>().ok()?)
    } else {
        ServiceSupported::from_confirmed_choice(name.parse::<ConfirmedServiceChoice>().ok()?)
    };
    bit.map(|bit| bit.to_raw())
}

/// The services of every outbound request literal in `source`.
fn outbound_in(file: &str, source: &str) -> BTreeSet<u8> {
    let mut found = BTreeSet::new();
    for name in REQUEST_TYPES {
        let pattern = format!("{name} {{");
        for (at, _) in source.match_indices(&pattern) {
            // The whole identifier: not `UnconfirmedRequest {` when looking
            // for `ConfirmedRequest {`, nor `PendingConfirmedRequest {`.
            if source[..at]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_alphanumeric() || c == '_')
            {
                continue;
            }
            let before = source[..at].trim_end();
            if before.ends_with("->") || before.ends_with("struct") || before.ends_with("for") {
                continue; // A return type or a declaration, not a literal.
            }
            let body = braced(&source[at + pattern.len()..]);
            let field = body
                .split([',', '\n'])
                .map(str::trim)
                .find(|f| f.starts_with("service_choice"))
                .unwrap_or_else(|| panic!("{file}: a request literal without service_choice"));
            let value = field
                .strip_prefix("service_choice")
                .unwrap()
                .trim_start_matches(':')
                .trim();
            // Field shorthand names a local of the same name.
            let value = if value.is_empty() {
                "service_choice"
            } else {
                value
            };
            if let Some(bit) = bit_of(value) {
                found.insert(bit);
                continue;
            }
            let Some((_, _, services, _)) = NON_LITERAL
                .iter()
                .find(|(name, expression, _, _)| file.ends_with(name) && value == *expression)
            else {
                panic!("{file}: add the request's service {value:?} to NON_LITERAL");
            };
            found.extend(services.iter().map(|service| service.to_raw()));
        }
    }
    found
}

#[test]
fn the_initiator_column_lists_every_outbound_request() {
    let listed: BTreeSet<u8> = PicsGenerator::INITIATED_SERVICES
        .iter()
        .map(|service| service.to_raw())
        .collect();
    assert_eq!(
        listed.len(),
        PicsGenerator::INITIATED_SERVICES.len(),
        "no service twice"
    );
    let mut sent = BTreeSet::new();
    for (path, source) in sources() {
        let file = path.to_string_lossy().into_owned();
        sent.extend(outbound_in(&file, &strip_test_modules(&source)));
    }
    let name = |bit: &u8| service_display_name(ServiceSupported::from_raw(*bit)).unwrap_or("?");
    let unlisted: Vec<_> = sent.difference(&listed).map(name).collect();
    let unsent: Vec<_> = listed.difference(&sent).map(name).collect();
    assert!(
        unlisted.is_empty() && unsent.is_empty(),
        "INITIATED_SERVICES must match the outbound requests: sent but not listed {unlisted:?}, listed but not sent {unsent:?}"
    );
}

#[test]
fn stripping_drops_only_inline_test_modules() {
    let source = "#[cfg(test)]\nuse a;\nfn kept() {}\n#[cfg(test)]\nmod tests;\n#[cfg(test)]\nmod inline {\n    fn f() { {} }\n}\nfn also_kept() {}\n";
    assert_eq!(
        strip_test_modules(source),
        "#[cfg(test)]\nuse a;\nfn kept() {}\n#[cfg(test)]\nmod tests;\n\nfn also_kept() {}\n"
    );
}

#[test]
fn every_initiated_service_has_a_pics_name() {
    for service in PicsGenerator::INITIATED_SERVICES {
        assert!(service_display_name(*service).is_some(), "{service:?}");
    }
}

#[test]
fn test_files_are_told_apart_by_their_path_under_src() {
    let kept: BTreeSet<PathBuf> = sources().into_iter().map(|(path, _)| path).collect();
    // Kept: ordinary sources, and one a non-test file loads by `#[path]`.
    for path in [
        "lib.rs",
        "pics/mod.rs",
        "server/cov_notifications.rs",
        "device_view_identity.rs",
    ] {
        assert!(kept.contains(Path::new(path)), "{path} is a source");
    }
    // Left out: by name, by directory, and by `#[cfg(test)] mod name;`.
    for path in [
        "pics/initiated_tests.rs",
        "server/test_transport.rs",
        "server/audit_reporter_test_support.rs",
        "handlers/tests/mod.rs",
        "server/warn_spy.rs",
        "handlers/read_property.rs",
    ] {
        assert!(!kept.contains(Path::new(path)), "{path} is test code");
    }
    assert!(!test_named(Path::new("server/latest.rs")));
    assert!(!test_named(Path::new("attestation/mod.rs")));
}

/// A source tree under a directory whose own path says "test" (#2224): only
/// the paths under it decide what is test code.
#[test]
fn a_root_whose_path_says_test_changes_nothing() {
    let root = std::env::temp_dir()
        .join(format!("pics-test-root-{}", std::process::id()))
        .join("src");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("server")).unwrap();
    for (path, source) in [
        ("lib.rs", "mod server;\n#[cfg(test)]\nmod spy;\n"),
        ("server.rs", "mod sends;\n#[cfg(test)]\nmod server_tests;\n"),
        ("server/sends.rs", ""),
        ("server_tests.rs", ""),
        ("spy.rs", "mod helper;\n"),
        ("spy/helper.rs", ""),
    ] {
        let file = root.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, source).unwrap();
    }
    let kept: BTreeSet<PathBuf> = sources_under(&root)
        .into_iter()
        .map(|(path, _)| path)
        .collect();
    std::fs::remove_dir_all(root.parent().unwrap()).unwrap();
    let expected: BTreeSet<PathBuf> = ["lib.rs", "server.rs", "server/sends.rs"]
        .into_iter()
        .map(PathBuf::from)
        .collect();
    assert_eq!(kept, expected);
}
