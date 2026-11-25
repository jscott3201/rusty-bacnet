//! Contract for the router guide's Rust program (#1989): the page shows the
//! file the integration bin compiles, with a manifest that follows `dev` or
//! pins the release (#2648).
#![forbid(unsafe_code)]

#[path = "support/guide_manifest.rs"]
mod guide_manifest;

const MANIFEST: &str = include_str!("../../../website/examples/rust/bip-router/Cargo.toml");
const SOURCE: &str = include_str!("../../../website/examples/rust/bip-router/src/main.rs");
const GUIDE: &str = include_str!("../../../website/src/content/docs/integrate/router.mdx");

fn manifest_contract(source: &str) -> Result<(), String> {
    let actual = guide_manifest::as_dev(source)?;
    let mut expected = String::from(
        r#"
[package]
name = "bip-router"
version = "0.1.0"
edition = "2024"
rust-version = "1.95"
publish = false
[workspace]
[dependencies]
tokio = { version = "1", features = ["macros", "rt-multi-thread", "time"] }
"#,
    );
    for krate in [
        "bacnet-client",
        "bacnet-encoding",
        "bacnet-network",
        "bacnet-objects",
        "bacnet-server",
        "bacnet-services",
        "bacnet-transport",
        "bacnet-types",
    ] {
        expected.push_str(&guide_manifest::dev(krate));
        expected.push('\n');
    }
    let expected: toml::Value = toml::from_str(&expected).unwrap();
    if actual != expected {
        return Err(format!("standalone manifest drifted: {actual}"));
    }
    Ok(())
}

#[test]
fn manifest_is_complete_and_follows_dev_or_the_release() {
    guide_manifest::check_forms(MANIFEST, manifest_contract);
    let dev = guide_manifest::on_dev(MANIFEST);
    for invalid in [
        dev.replace(&format!("{}\n", guide_manifest::dev("bacnet-types")), ""),
        dev.replace(", \"time\"", ""),
        format!("{dev}bytes = \"1\"\n"),
    ] {
        assert!(
            manifest_contract(&invalid).is_err(),
            "accepted drift: {invalid}"
        );
    }
    // The page's note says what the manifest builds against (website/src/lib/samples.mjs).
    assert_eq!(
        GUIDE
            .matches("<SampleNote sample=\"bip-router\" />")
            .count(),
        1
    );
}

#[test]
fn bin_compiles_the_source_the_guide_imports() {
    let integration: toml::Value = toml::from_str(include_str!("../Cargo.toml")).unwrap();
    let bins = integration["bin"].as_array().unwrap();
    let sample: Vec<_> = bins
        .iter()
        .filter(|bin| bin["name"].as_str() == Some("guide-bip-router"))
        .collect();
    assert_eq!(sample.len(), 1);
    assert_eq!(
        sample[0]["path"].as_str(),
        Some("../../website/examples/rust/bip-router/src/main.rs")
    );
    for line in [
        "import routerSource from '../../../../examples/rust/bip-router/src/main.rs?raw';",
        "import routerManifest from '../../../../examples/rust/bip-router/Cargo.toml?raw';",
        "<Code code={routerSource} lang=\"rust\" title=\"src/main.rs\" />",
        "<Code code={routerManifest} lang=\"toml\" title=\"Cargo.toml\" />",
    ] {
        assert_eq!(GUIDE.matches(line).count(), 1, "{line}");
    }
    assert!(SOURCE.ends_with('\n'));
    // The router is stopped on every path, and its result is checked.
    assert!(SOURCE.contains("let stopped = router.stop().await;"));
    assert!(SOURCE.contains("stopped?;"));
}
