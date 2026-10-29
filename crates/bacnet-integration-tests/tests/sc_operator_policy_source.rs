//! Contract for the security guide's policy program (#1989): the page shows
//! the file the integration bin compiles, with a release-pinned manifest.
#![forbid(unsafe_code)]

const MANIFEST: &str = include_str!("../../../website/examples/rust/sc-operator-policy/Cargo.toml");
const SOURCE: &str = include_str!("../../../website/examples/rust/sc-operator-policy/src/main.rs");
const GUIDE: &str = include_str!("../../../website/src/content/docs/integrate/security.mdx");
fn manifest_contract(source: &str) -> Result<(), String> {
    let actual: toml::Value = toml::from_str(source).map_err(|e| e.to_string())?;
    let expected: toml::Value = toml::from_str(
        r#"
[package]
name = "sc-operator-policy"
version = "0.1.0"
edition = "2024"
rust-version = "1.95"
publish = false
[workspace]
[dependencies]
bacnet-server = { version = "=0.15.0", features = ["sc-tls"] }
bacnet-transport = "=0.15.0"
bacnet-types = "=0.15.0"
"#,
    )
    .unwrap();
    if actual != expected {
        return Err(format!("standalone release manifest drifted: {actual}"));
    }
    Ok(())
}

#[test]
fn manifest_is_complete_and_release_pinned() {
    manifest_contract(MANIFEST).unwrap();
    assert!(
        include_str!("../../../website/src/lib/site.mjs")
            .contains("export const release = '0.15.0';")
    );
    for invalid in [
        MANIFEST.replace(", features = [\"sc-tls\"]", ""),
        MANIFEST.replace("bacnet-types = \"=0.15.0\"", "bacnet-types = \"0.15\""),
        MANIFEST.replace("[workspace]\n", ""),
        MANIFEST.replace("rust-version = \"1.95\"", "rust-version = \"1.93\""),
        format!("{MANIFEST}tokio = \"1\"\n"),
    ] {
        assert!(
            manifest_contract(&invalid).is_err(),
            "accepted drift: {invalid}"
        );
    }
    // The page says which release the manifest pins.
    assert!(
        GUIDE.contains("The manifest pins **v{release}**, whose minimum Rust version is **1.95**.")
    );
}

#[test]
fn bin_compiles_the_source_the_guide_imports() {
    let integration: toml::Value = toml::from_str(include_str!("../Cargo.toml")).unwrap();
    let bins = integration["bin"].as_array().unwrap();
    let sample: Vec<_> = bins
        .iter()
        .filter(|bin| bin["name"].as_str() == Some("guide-sc-operator-policy"))
        .collect();
    assert_eq!(sample.len(), 1);
    assert_eq!(
        sample[0]["path"].as_str(),
        Some("../../website/examples/rust/sc-operator-policy/src/main.rs")
    );
    assert_eq!(
        sample[0]["required-features"].as_array().unwrap(),
        &[toml::Value::from("sc-tls")]
    );
    for line in [
        "import policySource from '../../../../examples/rust/sc-operator-policy/src/main.rs?raw';",
        "import policyManifest from '../../../../examples/rust/sc-operator-policy/Cargo.toml?raw';",
        "<Code code={policySource} lang=\"rust\" title=\"src/main.rs\" />",
        "<Code code={policyManifest} lang=\"toml\" title=\"Cargo.toml\" />",
    ] {
        assert_eq!(GUIDE.matches(line).count(), 1, "{line}");
    }
    assert!(SOURCE.ends_with('\n'));
    // The rule the page explains: equality on a DNS name, nothing looser.
    assert!(SOURCE.contains("ScSubjectAltName::Dns(dns) => *dns == self.operator"));
    for looser in [
        "ends_with",
        "to_lowercase",
        "eq_ignore_ascii_case",
        "contains(",
    ] {
        assert!(!SOURCE.contains(looser), "{looser}");
    }
}
