//! Contract for the Rust first-read guide's program (#1989): the page shows
//! the file the `guide-first-read` bin compiles, with a manifest that follows
//! `dev` or pins the release (#2648).
#![forbid(unsafe_code)]

#[path = "support/guide_manifest.rs"]
mod guide_manifest;

const MANIFEST: &str = include_str!("../../../website/examples/rust/first-read/Cargo.toml");
const SOURCE: &str = include_str!("../../../website/examples/rust/first-read/src/main.rs");
const GUIDE: &str = include_str!("../../../website/src/content/docs/start/rust.mdx");
const BIN: &str = include_str!("../src/bin/guide-first-read.rs");

fn manifest_contract(source: &str) -> Result<(), String> {
    let mut actual = guide_manifest::as_dev(source)?;
    let expected: toml::Value = toml::from_str(
        r#"
[package]
name = "first-read"
version = "0.1.0"
edition = "2024"
rust-version = "1.95"
publish = false
[workspace]
[dependencies]
bacnet-client = { git = "https://github.com/jscott3201/rusty-bacnet", branch = "dev" }
bacnet-encoding = { git = "https://github.com/jscott3201/rusty-bacnet", branch = "dev" }
bacnet-types = { git = "https://github.com/jscott3201/rusty-bacnet", branch = "dev" }
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
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
fn manifest_is_complete_and_follows_dev_or_the_release() {
    guide_manifest::check_forms(MANIFEST, manifest_contract);
    let dev = guide_manifest::on_dev(MANIFEST);
    for invalid in [
        dev.replace(&format!("{}\n", guide_manifest::dev("bacnet-encoding")), ""),
        dev.replace("rt-multi-thread", "rt"),
        format!("{dev}unexpected = \"1\"\n"),
    ] {
        assert!(
            manifest_contract(&invalid).is_err(),
            "accepted drift: {invalid}"
        );
    }
    // Feature order and comments are not drift.
    manifest_contract(&format!(
        "# A saveable manifest.\n{}",
        dev.replace(
            "[\"macros\", \"rt-multi-thread\"]",
            "[\"rt-multi-thread\", \"macros\"]"
        )
    ))
    .unwrap();
}

#[test]
fn the_bin_compiles_the_source_the_guide_imports() {
    assert!(
        BIN.contains("include!(\"../../../../website/examples/rust/first-read/src/main.rs\");")
    );
    for line in [
        "import firstReadSource from '../../../../examples/rust/first-read/src/main.rs?raw';",
        "import firstReadManifest from '../../../../examples/rust/first-read/Cargo.toml?raw';",
        "<Code code={firstReadSource} lang=\"rust\" title=\"src/main.rs\" />",
        "<Code code={firstReadManifest} lang=\"toml\" title=\"Cargo.toml\" />",
        "<SampleNote sample=\"first-read\" />",
    ] {
        assert_eq!(GUIDE.matches(line).count(), 1, "{line}");
    }
    assert!(SOURCE.ends_with("}\n"));
    // `include!` takes items only: the program has no inner attributes.
    assert!(!SOURCE.contains("#!["));
    assert!(GUIDE.contains("minimum Rust version of **1.95**"));
}
