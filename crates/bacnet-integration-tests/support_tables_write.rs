//! Rewrites the website's generated conformance pages from the code (#1496):
//! the capability tables, `website/src/data/support.json`, and the sample
//! PICS, `website/src/content/docs/project/sample-pics.md`.
//! `cargo run -p bacnet-integration-tests --example support-tables`.
#![forbid(unsafe_code)]
#![allow(clippy::print_stdout, clippy::print_stderr)] // a command-line tool reports what it did

#[path = "sample_pics.rs"]
mod sample_pics;
#[path = "support_tables.rs"]
mod support_tables;

type Generator = fn(&std::path::Path) -> Result<String, String>;

fn main() {
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let outputs: [(&str, Generator); 2] = [
        (support_tables::OUTPUT, support_tables::generate),
        (sample_pics::OUTPUT, sample_pics::generate),
    ];
    for (output, generate) in outputs {
        let text = generate(&repo).unwrap_or_else(|error| {
            eprintln!("support-tables: {error}");
            std::process::exit(1);
        });
        let path = repo.join(output);
        std::fs::write(&path, text).unwrap_or_else(|error| {
            eprintln!("support-tables: write {}: {error}", path.display());
            std::process::exit(1);
        });
        println!("wrote {output}");
    }
}
