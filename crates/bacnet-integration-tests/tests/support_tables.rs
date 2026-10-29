//! The committed capability tables and sample PICS match a fresh run of
//! their generators (#1496).
#![forbid(unsafe_code)]

#[path = "../sample_pics.rs"]
mod sample_pics;
#[path = "../support_tables.rs"]
mod support_tables;

use std::path::{Path, PathBuf};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Each generated file, with its generator.
type Generator = fn(&Path) -> Result<String, String>;
const GENERATED: [(&str, Generator); 2] = [
    (support_tables::OUTPUT, support_tables::generate),
    (sample_pics::OUTPUT, sample_pics::generate),
];

fn committed(path: &str) -> String {
    std::fs::read_to_string(repo().join(path))
        .unwrap_or_default()
        .replace("\r\n", "\n")
}

#[test]
fn the_committed_support_tables_and_sample_pics_are_fresh() {
    for (output, generate) in GENERATED {
        let fresh = generate(&repo()).unwrap();
        assert!(
            committed(output) == fresh,
            "{output} is stale: run `cargo run -p bacnet-integration-tests --example support-tables` and commit the result"
        );
    }
}

#[test]
fn the_generated_files_never_claim_conformance() {
    for (output, _) in GENERATED {
        let text = committed(output).to_lowercase();
        assert!(!text.is_empty(), "{output} is missing");
        for word in support_tables::FORBIDDEN_WORDS {
            assert!(!text.contains(word), "{output} says {word:?}");
        }
        assert!(!text.contains("bibb"), "{output} names a BIBB");
    }
}

#[test]
fn the_sample_pics_carries_its_banner_and_names_its_configuration() {
    let sample = committed(sample_pics::OUTPUT);
    let reference = sample_pics::REFERENCE;
    // The title and the banner name the configuration, and the banner comes
    // before the generator's output, inside the file the raw export copies.
    assert!(sample.contains(&format!("title: \"Sample PICS: the {reference}\"")));
    let banner = sample
        .find(&format!(
            ":::caution[Generated sample for the {reference} only]"
        ))
        .expect("the banner");
    let pics = sample
        .find("\n## BACnet Protocol Implementation Conformance Statement (PICS)\n")
        .expect("the generator's title, one level down");
    assert!(banner < pics);
    assert!(sample.contains("not a claim for any product or device"));
    // None of the generator's headings stays at the page title's level.
    assert!(!sample.lines().any(|line| line.starts_with("# ")));
    // The declared configuration: B/IP and SC, router and BBMD off.
    for line in [
        "- BACnet/IP (Annex J)",
        "- BACnet/SC (Annex AB)",
        "| Router | false |",
        "| BBMD | false |",
    ] {
        assert!(sample.contains(&format!("\n{line}\n")), "{line}");
    }
}

const STUB: &str = "x\nclass BACnetClient:\n    def __init__(self) -> None: ...\n    def read_property(\n    async def who_has_by_id(\n    def vt_open(\n    def subscribe_cov_property_multiple(\n    def read_property_from_devices(\nclass Other:\n    def write_property(\n";

#[test]
fn python_methods_match_services_by_name() {
    use bacnet_types::enums::ServiceSupported as S;
    let found = support_tables::python_services_with(STUB, &["__init__"]).unwrap();
    let expected: std::collections::BTreeSet<u8> = [
        S::READ_PROPERTY,
        S::WHO_HAS,
        S::VT_OPEN,
        S::SUBSCRIBE_COV_PROPERTY_MULTIPLE,
    ]
    .iter()
    .map(|s| s.to_raw())
    .collect();
    // Not SubscribeCOVProperty (a prefix of a longer service), and nothing
    // from another class.
    assert_eq!(found, expected);
}

#[test]
fn every_python_method_is_classified() {
    // A method that names no service and isn't listed fails generation.
    let stub = STUB.replace("class Other:", "    def who_is_listening(\nclass Other:");
    let error = support_tables::python_services_with(&stub, &["__init__"]).unwrap_err();
    assert!(error.contains("who_is_listening"), "{error}");
    // So does a listed method the stub no longer has.
    let error = support_tables::python_services_with(STUB, &["__init__", "gone"]).unwrap_err();
    assert!(error.contains("gone"), "{error}");
    // A listed method counts for no service, even when its name would match one.
    let stub = STUB.replace("class Other:", "    def who_is_replies(\nclass Other:");
    let found =
        support_tables::python_services_with(&stub, &["__init__", "who_is_replies"]).unwrap();
    let who_is = bacnet_types::enums::ServiceSupported::WHO_IS.to_raw();
    assert!(!found.contains(&who_is));
    // The real stub is fully classified.
    let real =
        std::fs::read_to_string(repo().join("crates/rusty-bacnet/rusty_bacnet.pyi")).unwrap();
    support_tables::python_services(&real).unwrap();
}

#[test]
fn the_feature_resolver_refuses_what_it_cannot_resolve() {
    let error = support_tables::transport_features(&repo(), "bacnet-cli", &["no-such-feature"])
        .unwrap_err();
    assert!(error.contains("no feature no-such-feature"), "{error}");
    let linux =
        support_tables::transport_features(&repo(), "bacnet-cli", &["sc-tls", "serial"]).unwrap();
    assert!(
        linux.contains("serial") && linux.contains("sc-tls") && linux.contains("ipv6"),
        "{linux:?}"
    );
    let default = support_tables::transport_features(&repo(), "bacnet-cli", &[]).unwrap();
    assert!(!default.contains("serial"), "{default:?}");
}
