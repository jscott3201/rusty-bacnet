//! Contract for the canonical program compiled directly by the integration
//! bin, with a manifest that follows `dev` or pins the release (#2648).
#![forbid(unsafe_code)]

#[path = "support/guide_manifest.rs"]
mod guide_manifest;

const MANIFEST: &str = include_str!("../../../website/examples/rust/shared-endpoint/Cargo.toml");
const SOURCE: &str = include_str!("../../../website/examples/rust/shared-endpoint/src/main.rs");
const GUIDE: &str =
    include_str!("../../../website/src/content/docs/integrate/shared-endpoints.mdx");

fn manifest_contract(source: &str) -> Result<(), String> {
    let mut actual = guide_manifest::as_dev(source)?;
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
bacnet-endpoint = { git = "https://github.com/jscott3201/rusty-bacnet", branch = "dev" }
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
        return Err(format!("standalone manifest drifted: {actual}"));
    }
    Ok(())
}

#[test]
fn canonical_manifest_is_complete_and_follows_dev_or_the_release() {
    guide_manifest::check_forms(MANIFEST, manifest_contract);
    // The page's note says what the manifest builds against
    // (website/src/lib/samples.mjs), and what CI and the release build.
    assert_eq!(
        GUIDE
            .matches("<SampleNote sample=\"shared-endpoint\" />")
            .count(),
        1
    );
    assert!(GUIDE.contains("CI compiles the exact source displayed here in the workspace"));
    assert!(GUIDE.contains("against the published crates"));
}

#[test]
fn dependency_versions_features_and_overrides_are_checked_semantically() {
    let dev = guide_manifest::on_dev(MANIFEST);
    let endpoint = guide_manifest::dev("bacnet-endpoint");
    let source = format!(
        "{{ git = \"{}\", branch = \"dev\"",
        guide_manifest::REPOSITORY
    );
    for invalid in [
        dev.replace(&endpoint, "bacnet-endpoint = \"0.15\""),
        dev.replace(&format!("{endpoint}\n"), ""),
        dev.replace(&endpoint, &endpoint.replace(" }", ", tag = \"v0.15.0\" }")),
        dev.replace(
            &source,
            "{ git = \"https://example.invalid/endpoint\", branch = \"dev\"",
        ),
        dev.replace(&source, &format!("{source}, features = [\"sc-tls\"]")),
        dev.replace(
            "version = \"1\"",
            "version = \"1\", default-features = false",
        ),
        dev.replace("\"rt-multi-thread\"", "\"rt\""),
        dev.replace("\"io-std\", ", ""),
        dev.replace(", \"io-util\"", ""),
        dev.replace("\"macros\", ", ""),
        dev.replace("\"io-util\"", "\"io-util\", \"full\""),
        format!("{dev}unexpected = \"1\"\n"),
        format!("{dev}\n[target.'cfg(unix)'.dependencies]\nunexpected = \"1\"\n"),
        dev.replace("edition = \"2024\"", "edition = \"2021\""),
        dev.replace("name = \"shared-endpoint-guide\"\n", ""),
    ] {
        assert!(
            manifest_contract(&invalid).is_err(),
            "accepted drift: {invalid}"
        );
    }
}

#[test]
fn comments_whitespace_and_feature_dependency_order_are_not_drift() {
    let dev = guide_manifest::on_dev(MANIFEST);
    manifest_contract(&format!("# A saveable manifest.\n\n{dev}")).unwrap();
    manifest_contract(&dev.replace(
        "[\"macros\", \"rt-multi-thread\", \"io-std\", \"io-util\"]",
        "[\"io-util\", \"io-std\", \"rt-multi-thread\", \"macros\"]",
    ))
    .unwrap();
    let lines: Vec<_> = dev.lines().collect();
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
