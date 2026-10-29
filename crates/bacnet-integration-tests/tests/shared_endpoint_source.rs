//! Contract for the canonical program compiled directly by the integration bin.
#![forbid(unsafe_code)]

const MANIFEST: &str = include_str!("../../../website/examples/rust/shared-endpoint/Cargo.toml");
const SOURCE: &str = include_str!("../../../website/examples/rust/shared-endpoint/src/main.rs");
const GUIDE: &str =
    include_str!("../../../website/src/content/docs/integrate/shared-endpoints.mdx");

fn manifest_contract(source: &str) -> Result<(), String> {
    let mut actual: toml::Value = toml::from_str(source).map_err(|e| e.to_string())?;
    let expected: toml::Value = toml::from_str(
        r#"
[package]
name = "shared-endpoint-guide"
version = "0.1.0"
edition = "2024"
rust-version = "1.95"
publish = false
[workspace]
[dependencies]
bacnet-endpoint = "=0.15.0"
tokio = { version = "1", features = ["io-std", "io-util", "macros", "rt-multi-thread"] }
"#,
    )
    .unwrap();
    if let Some(features) = actual
        .get_mut("dependencies")
        .and_then(|deps| deps.get_mut("tokio"))
        .and_then(|tokio| tokio.get_mut("features"))
        .and_then(toml::Value::as_array_mut)
    {
        features.sort_by(|left, right| left.as_str().cmp(&right.as_str()));
    }
    if actual != expected {
        return Err(format!("standalone release manifest drifted: {actual}"));
    }
    Ok(())
}

#[test]
fn canonical_manifest_is_complete_and_release_pinned() {
    manifest_contract(MANIFEST).unwrap();
    assert!(
        include_str!("../../../website/src/lib/site.mjs")
            .contains("export const release = '0.15.0';")
    );
    assert!(GUIDE.contains("These files target **v{release}**"));
    assert!(GUIDE.contains("minimum Rust version is **1.95**"));
    assert!(GUIDE.contains("Rust 1.99.0 and a development test harness"));
}

#[test]
fn dependency_versions_features_and_overrides_are_checked_semantically() {
    for invalid in [
        MANIFEST.replace("=0.15.0", "0.15"),
        MANIFEST.replace("=0.15.0", "=0.16.0"),
        MANIFEST.replace("bacnet-endpoint = \"=0.15.0\"\n", ""),
        MANIFEST.replace(
            "\"=0.15.0\"",
            "{ version = \"=0.15.0\", path = \"../local\" }",
        ),
        MANIFEST.replace(
            "\"=0.15.0\"",
            "{ git = \"https://example.invalid/endpoint\" }",
        ),
        MANIFEST.replace(
            "\"=0.15.0\"",
            "{ version = \"=0.15.0\", default-features = false }",
        ),
        MANIFEST.replace(
            "\"=0.15.0\"",
            "{ version = \"=0.15.0\", features = [\"sc-tls\"] }",
        ),
        MANIFEST.replace(
            "version = \"1\"",
            "version = \"1\", default-features = false",
        ),
        MANIFEST.replace("\"rt-multi-thread\"", "\"rt\""),
        MANIFEST.replace("\"io-std\", ", ""),
        MANIFEST.replace(", \"io-util\"", ""),
        MANIFEST.replace("\"macros\", ", ""),
        MANIFEST.replace("\"io-util\"", "\"io-util\", \"full\""),
        format!("{MANIFEST}unexpected = \"1\"\n"),
        format!("{MANIFEST}\n[patch.crates-io]\nbacnet-endpoint = {{ path = \"../local\" }}\n"),
        format!("{MANIFEST}\n[target.'cfg(unix)'.dependencies]\nunexpected = \"1\"\n"),
        MANIFEST.replace("[workspace]\n", ""),
        MANIFEST.replace("edition = \"2024\"", "edition = \"2021\""),
        MANIFEST.replace("name = \"shared-endpoint-guide\"\n", ""),
    ] {
        assert!(
            manifest_contract(&invalid).is_err(),
            "accepted drift: {invalid}"
        );
    }
}

#[test]
fn comments_whitespace_and_feature_dependency_order_are_not_drift() {
    manifest_contract(&format!("# A saveable manifest.\n\n{MANIFEST}")).unwrap();
    manifest_contract(&MANIFEST.replace(
        "[\"macros\", \"rt-multi-thread\", \"io-std\", \"io-util\"]",
        "[\"io-util\", \"io-std\", \"rt-multi-thread\", \"macros\"]",
    ))
    .unwrap();
    let lines: Vec<_> = MANIFEST.lines().collect();
    let mut reversed = lines[..lines.len() - 2].join("\n");
    for line in lines[lines.len() - 2..].iter().rev() {
        reversed.push('\n');
        reversed.push_str(line);
    }
    manifest_contract(&reversed).unwrap();
}

#[test]
fn normal_bin_compiles_the_same_canonical_source_imported_by_mdx() {
    let integration: toml::Value = toml::from_str(include_str!("../Cargo.toml")).unwrap();
    let bins = integration.get("bin").unwrap().as_array().unwrap();
    let sample: Vec<_> = bins
        .iter()
        .filter(|bin| bin["name"].as_str() == Some("guide-shared-endpoint"))
        .collect();
    assert_eq!(sample.len(), 1);
    assert_eq!(
        sample[0]["path"].as_str(),
        Some("../../website/examples/rust/shared-endpoint/src/main.rs")
    );
    assert_eq!(GUIDE.matches("import endpointSource from '../../../../examples/rust/shared-endpoint/src/main.rs?raw';").count(), 1);
    assert_eq!(
        GUIDE
            .matches("<Code code={endpointSource} lang=\"rust\" title=\"src/main.rs\" />")
            .count(),
        1
    );
    assert_eq!(GUIDE.matches("import endpointManifest from '../../../../examples/rust/shared-endpoint/Cargo.toml?raw';").count(), 1);
    assert_eq!(
        GUIDE
            .matches("<Code code={endpointManifest} lang=\"toml\" title=\"Cargo.toml\" />")
            .count(),
        1
    );
    assert!(!SOURCE.is_empty());
    assert!(SOURCE.ends_with('\n'));
}
