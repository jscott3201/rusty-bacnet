//! Contract for the PICS task page's program (#1496): the page shows the file
//! the `guide-pics` bin compiles, with a manifest that pins the release or
//! follows `dev` (#2648).
#![forbid(unsafe_code)]

#[path = "support/guide_manifest.rs"]
mod guide_manifest;

const MANIFEST: &str = include_str!("../../../website/examples/rust/pics/Cargo.toml");
const SOURCE: &str = include_str!("../../../website/examples/rust/pics/src/main.rs");
const GUIDE: &str = include_str!("../../../website/src/content/docs/integrate/pics.mdx");
const BIN: &str = include_str!("../src/bin/guide-pics.rs");

fn manifest_contract(source: &str) -> Result<(), String> {
    let actual = guide_manifest::as_dev(source)?;
    let expected: toml::Value = toml::from_str(
        r#"
[package]
name = "generate-pics"
version = "0.1.0"
edition = "2024"
rust-version = "1.95"
publish = false
[workspace]
[dependencies]
bacnet-objects = { git = "https://github.com/jscott3201/rusty-bacnet", branch = "dev" }
bacnet-server = { git = "https://github.com/jscott3201/rusty-bacnet", branch = "dev" }
bacnet-types = { git = "https://github.com/jscott3201/rusty-bacnet", branch = "dev" }
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
"#,
    )
    .unwrap();
    if actual != expected {
        return Err(format!("standalone manifest drifted: {actual}"));
    }
    Ok(())
}

#[test]
fn manifest_is_complete_and_pins_the_release_or_follows_dev() {
    guide_manifest::check_forms(MANIFEST, manifest_contract);
    let dev = guide_manifest::on_dev(MANIFEST);
    for invalid in [
        dev.replace(&format!("{}\n", guide_manifest::dev("bacnet-types")), ""),
        dev.replace("\"macros\", ", ""),
        format!("{dev}{}\n", guide_manifest::dev("bacnet-client")),
    ] {
        assert!(
            manifest_contract(&invalid).is_err(),
            "accepted drift: {invalid}"
        );
    }
}

#[test]
fn the_bin_compiles_the_source_the_page_imports() {
    assert!(BIN.contains("include!(\"../../../../website/examples/rust/pics/src/main.rs\");"));
    for line in [
        "import picsSource from '../../../../examples/rust/pics/src/main.rs?raw';",
        "import picsManifest from '../../../../examples/rust/pics/Cargo.toml?raw';",
        "<Code code={picsSource} lang=\"rust\" title=\"src/main.rs\" />",
        "<Code code={picsManifest} lang=\"toml\" title=\"Cargo.toml\" />",
        "<SampleNote sample=\"pics\" />",
    ] {
        assert_eq!(GUIDE.matches(line).count(), 1, "{line}");
    }
    // `include!` takes items only: the program has no inner attributes.
    assert!(!SOURCE.contains("#!["));
    assert!(SOURCE.contains("server.generate_pics(&pics_config).await"));
}
