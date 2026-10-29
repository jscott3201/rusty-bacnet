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

/// The crate's non-test source files: test-named files and directories are
/// left out, and so are the files test modules include by `#[path]`.
fn sources() -> Vec<(PathBuf, String)> {
    fn walk(dir: &Path, out: &mut Vec<(PathBuf, String)>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push((path.clone(), std::fs::read_to_string(&path).unwrap()));
            }
        }
    }
    let mut all = Vec::new();
    walk(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut all);
    let is_test = |path: &Path| path.to_string_lossy().contains("test");
    let mut test_support = BTreeSet::new();
    for (path, source) in all.iter().filter(|(path, _)| is_test(path)) {
        for (at, _) in source.match_indices("#[path = \"") {
            let rest = &source[at + "#[path = \"".len()..];
            let name = &rest[..rest.find('"').unwrap()];
            test_support.insert(path.parent().unwrap().join(name));
        }
    }
    all.into_iter()
        .filter(|(path, _)| !is_test(path) && !test_support.contains(path))
        .collect()
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
