//! Contract for the router guide's Rust program (#1989): the page shows the
//! file the integration bin compiles, with a release-pinned manifest.
#![forbid(unsafe_code)]

const MANIFEST: &str = include_str!("../../../website/examples/rust/bip-router/Cargo.toml");
const SOURCE: &str = include_str!("../../../website/examples/rust/bip-router/src/main.rs");
const GUIDE: &str = include_str!("../../../website/src/content/docs/integrate/router.mdx");

fn manifest_contract(source: &str) -> Result<(), String> {
    let actual: toml::Value = toml::from_str(source).map_err(|e| e.to_string())?;
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
        expected.push_str(&format!("{krate} = \"=0.15.0\"\n"));
    }
    let expected: toml::Value = toml::from_str(&expected).unwrap();
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
        MANIFEST.replace("bacnet-network = \"=0.15.0\"", "bacnet-network = \"0.15\""),
        MANIFEST.replace("bacnet-types = \"=0.15.0\"\n", ""),
        MANIFEST.replace("[workspace]\n", ""),
        MANIFEST.replace("rust-version = \"1.95\"", "rust-version = \"1.93\""),
        MANIFEST.replace(", \"time\"", ""),
        format!("{MANIFEST}bytes = \"1\"\n"),
    ] {
        assert!(
            manifest_contract(&invalid).is_err(),
            "accepted drift: {invalid}"
        );
    }
    assert!(GUIDE.contains(
        "The Rust manifest pins **v{release}**, whose minimum Rust version is **1.95**."
    ));
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
