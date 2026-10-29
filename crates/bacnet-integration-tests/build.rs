//! Compile the actual Rust first-read guide, without a maintained copy.

mod sample_source;

fn main() {
    let guide = "../../website/src/content/docs/start/rust.md";
    println!("cargo:rerun-if-changed={guide}");
    println!("cargo:rerun-if-changed=sample_source.rs");
    let document = std::fs::read(guide).expect("read Rust first-read guide");
    let rust = sample_source::extract(&document, "rust").expect("unique Rust sample");
    let toml = sample_source::extract(&document, "toml").expect("unique Cargo sample");
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").expect("Cargo OUT_DIR"));
    std::fs::write(out.join("first_read.rs"), rust).expect("write exact Rust sample");
    std::fs::write(out.join("first_read.toml"), toml).expect("write exact Cargo sample");
}
