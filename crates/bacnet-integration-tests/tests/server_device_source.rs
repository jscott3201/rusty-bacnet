//! Contract for the server guide's Rust program (#1989): the page shows the
//! file the integration bin compiles, with a release-pinned manifest.
#![forbid(unsafe_code)]

const MANIFEST: &str = include_str!("../../../website/examples/rust/server-device/Cargo.toml");
const SOURCE: &str = include_str!("../../../website/examples/rust/server-device/src/main.rs");
const GUIDE: &str = include_str!("../../../website/src/content/docs/integrate/server.mdx");

fn manifest_contract(source: &str) -> Result<(), String> {
    let actual: toml::Value = toml::from_str(source).map_err(|e| e.to_string())?;
    let expected: toml::Value = toml::from_str(
        r#"
[package]
name = "server-device"
version = "0.1.0"
edition = "2024"
rust-version = "1.95"
publish = false
[workspace]
[dependencies]
bacnet-objects = "=0.15.0"
bacnet-server = "=0.15.0"
bacnet-transport = "=0.15.0"
bacnet-types = "=0.15.0"
tokio = { version = "1", features = ["macros", "rt-multi-thread", "io-std", "io-util", "time"] }
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
        MANIFEST.replace("bacnet-server = \"=0.15.0\"", "bacnet-server = \"0.15\""),
        MANIFEST.replace("bacnet-types = \"=0.15.0\"\n", ""),
        MANIFEST.replace(", \"time\"", ""),
        MANIFEST.replace("[workspace]\n", ""),
        MANIFEST.replace("rust-version = \"1.95\"", "rust-version = \"1.93\""),
        format!("{MANIFEST}bacnet-client = \"=0.15.0\"\n"),
    ] {
        assert!(
            manifest_contract(&invalid).is_err(),
            "accepted drift: {invalid}"
        );
    }
    assert!(
        GUIDE
            .contains("These files target **v{release}**, whose minimum Rust version is **1.95**.")
    );
}

#[test]
fn bin_compiles_the_source_the_guide_imports() {
    let integration: toml::Value = toml::from_str(include_str!("../Cargo.toml")).unwrap();
    let bins = integration["bin"].as_array().unwrap();
    let sample: Vec<_> = bins
        .iter()
        .filter(|bin| bin["name"].as_str() == Some("guide-server-device"))
        .collect();
    assert_eq!(sample.len(), 1);
    assert_eq!(
        sample[0]["path"].as_str(),
        Some("../../website/examples/rust/server-device/src/main.rs")
    );
    assert!(sample[0].get("required-features").is_none());
    for line in [
        "import serverSource from '../../../../examples/rust/server-device/src/main.rs?raw';",
        "import serverManifest from '../../../../examples/rust/server-device/Cargo.toml?raw';",
        "<Code code={serverSource} lang=\"rust\" title=\"src/main.rs\" />",
        "<Code code={serverManifest} lang=\"toml\" title=\"Cargo.toml\" />",
    ] {
        assert_eq!(GUIDE.matches(line).count(), 1, "{line}");
    }
    assert!(SOURCE.ends_with('\n'));
    // The page explains that the application supplies the input through the
    // server and reads the commanded output back.
    assert!(SOURCE.contains(".set_present_value_local(&temperature,"));
    assert!(SOURCE.contains(".read_local(&fan, PropertyIdentifier::PRESENT_VALUE, None)"));
    assert!(SOURCE.contains("server.stop().await?;"));
}
