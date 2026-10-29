//! The guide is the source of both the executable and its dependency contract.
#![forbid(unsafe_code)]

#[path = "../sample_source.rs"]
mod sample_source;

const GUIDE: &[u8] = include_bytes!("../../../website/src/content/docs/start/rust.md");

#[test]
fn generated_files_are_the_exact_guide_fences() {
    assert_eq!(
        sample_source::extract(GUIDE, "rust").unwrap(),
        include_bytes!(concat!(env!("OUT_DIR"), "/first_read.rs"))
    );
    assert_eq!(
        sample_source::extract(GUIDE, "toml").unwrap(),
        include_bytes!(concat!(env!("OUT_DIR"), "/first_read.toml"))
    );
}

#[test]
fn extraction_preserves_crlf_blank_lines_and_final_newlines() {
    let document = b"before\r\n```rust\r\n\r\nfn main() {}\r\n\r\n```\r\nafter";
    assert_eq!(
        sample_source::extract(document, "rust").unwrap(),
        b"\r\nfn main() {}\r\n\r\n"
    );
    assert_eq!(
        sample_source::extract(b"```rust\nx\n```", "rust").unwrap(),
        b"x\n"
    );
    assert_eq!(
        sample_source::extract(b"```python\nx\n```\n```rust\ny\n```\n", "rust").unwrap(),
        b"y\n"
    );
}

#[test]
fn malformed_or_ambiguous_fences_fail_closed() {
    for (source, error) in [
        ("nothing", "missing"),
        ("```rust\n```\n", "empty"),
        ("```rust\n \t\r\n```", "empty"),
        ("```rust\nx", "unclosed"),
        ("```rust\nx\n```\n```rust\ny\n```", "duplicate"),
        ("```rust\nx\n```\n```rust\ny", "duplicate"),
        ("```python\n```rust\nx\n```\n", "missing"),
    ] {
        assert!(
            sample_source::extract(source.as_bytes(), "rust")
                .unwrap_err()
                .contains(error),
            "{source:?}"
        );
        let toml = source.replace("rust", "toml");
        assert!(
            sample_source::extract(toml.as_bytes(), "toml")
                .unwrap_err()
                .contains(error),
            "{toml:?}"
        );
    }
}

fn dependencies(source: &[u8]) -> Result<(), String> {
    let mut parsed: toml::Value =
        toml::from_str(std::str::from_utf8(source).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    let expected: toml::Value = toml::from_str(
        r#"
[dependencies]
bacnet-client = "=0.15.0"
bacnet-types = "=0.15.0"
bacnet-encoding = "=0.15.0"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
"#,
    )
    .unwrap();
    if let Some(features) = parsed
        .get_mut("dependencies")
        .and_then(|v| v.get_mut("tokio"))
        .and_then(|v| v.get_mut("features"))
        .and_then(toml::Value::as_array_mut)
    {
        features.sort_by(|a, b| a.as_str().cmp(&b.as_str()));
    }
    if parsed != expected {
        return Err(format!("release dependency contract drifted: {parsed}"));
    }
    Ok(())
}

#[test]
fn dependency_contract_matches_the_documented_release() {
    dependencies(sample_source::extract(GUIDE, "toml").unwrap()).unwrap();
    let prose = std::str::from_utf8(GUIDE).unwrap();
    assert!(prose.contains("These instructions target **v0.15.0**"));
    assert!(prose.contains("minimum Rust version of **1.95**"));
}

#[test]
fn dependency_drift_is_detected_semantically() {
    let source = std::str::from_utf8(sample_source::extract(GUIDE, "toml").unwrap()).unwrap();
    for bad in [
        source.replace("=0.15.0", "0.15"),
        source.replace("rt-multi-thread", "rt"),
        format!("{source}unexpected = \"1\"\n"),
        source.replace("bacnet-encoding = \"=0.15.0\"\n", ""),
    ] {
        assert!(dependencies(bad.as_bytes()).is_err(), "{bad}");
    }
    dependencies(
        source
            .replace(
                "[\"macros\", \"rt-multi-thread\"]",
                "[\"rt-multi-thread\", \"macros\"]",
            )
            .as_bytes(),
    )
    .unwrap();
    dependencies(format!("# Formatting and comments are not semantic drift.\n{source}").as_bytes())
        .unwrap();
}

// These tests define the intentionally narrow guide profile. They are not a
// claim that the extractor is a general Markdown/MDX renderer.
fn outer_fences_keep_nested_samples_literal(language: &str) {
    for (opening, misleading_close, closing) in [
        ("~~~text", "```", "~~~"),
        ("````text", "```", "````"),
        ("~~~~text", "~~~", "~~~~"),
        ("`````text", "~~~~~", "`````"),
    ] {
        let inert =
            format!("{opening}\n{misleading_close}\n```{language}\nnested\n```\n{closing}\n");
        assert!(
            sample_source::extract(inert.as_bytes(), language).is_err(),
            "inert {language}: {inert}"
        );
        let outside = format!("{inert}```{language}\nactual\n```\n");
        assert_eq!(
            sample_source::extract(outside.as_bytes(), language).unwrap(),
            b"actual\n",
            "{outside}"
        );
    }
}

fn target_identity_is_separate_from_opener_whitespace(language: &str) {
    for opener in [
        format!("```{language} "),
        format!("```{language}\t"),
        format!("``` {language}\t "),
        format!("~~~{}", language.to_ascii_uppercase()),
    ] {
        let delimiter = if opener.starts_with('~') {
            "~~~"
        } else {
            "```"
        };
        let first = format!("{opener}\nexact\r\n\r\n{delimiter}\n");
        assert_eq!(
            sample_source::extract(first.as_bytes(), language).unwrap(),
            b"exact\r\n\r\n",
            "{first}"
        );
        let duplicate = format!("```{language}\nfirst\n```\n{first}");
        assert!(
            sample_source::extract(duplicate.as_bytes(), language).is_err(),
            "{duplicate}"
        );
        let unclosed = format!("```{language}\nfirst\n```\n{opener}\nsecond");
        assert!(
            sample_source::extract(unclosed.as_bytes(), language).is_err(),
            "{unclosed}"
        );
    }
}

fn target_metadata_is_rejected_instead_of_ignored(language: &str) {
    for suffix in [" title=other", "\t{1}", " {title=other}", " linenums"] {
        let variant = format!("```{language}{suffix}\nsecond\n```\n");
        assert!(
            sample_source::extract(variant.as_bytes(), language).is_err(),
            "{variant}"
        );
        for ending in ["\n```\n", ""] {
            let duplicate =
                format!("```{language}\nfirst\n```\n```{language}{suffix}\nsecond{ending}");
            assert!(
                sample_source::extract(duplicate.as_bytes(), language).is_err(),
                "{duplicate}"
            );
        }
    }
}

fn only_matching_sufficient_closers_end_a_fence(language: &str) {
    for (opening, short, wrong, closing) in [
        ("````", "```", "~~~~", "`````"),
        ("~~~~", "~~~", "````", "~~~~~"),
    ] {
        let content = format!("first\r\n{short}\r\n{wrong}\r\nlast\r\n");
        let source = format!("{opening}{language}\r\n{content}   {closing}\t \r\n");
        assert_eq!(
            sample_source::extract(source.as_bytes(), language).unwrap(),
            content.as_bytes(),
            "{source}"
        );
        let unclosed = format!("{opening}{language}\nfirst\n{short}\n{wrong}\n");
        assert!(
            sample_source::extract(unclosed.as_bytes(), language).is_err(),
            "{unclosed}"
        );
    }
}

fn unsupported_containers_cannot_hide_or_supply_a_sample(language: &str) {
    let valid = format!("```{language}\nactual\n```\n");
    for wrapped in [
        format!("> ```{language}\n> hidden\n> ```\n"),
        format!("- example\n\n  ```{language}\n  hidden\n  ```\n"),
        format!("1. ```{language}\n   hidden\n   ```\n"),
        format!("<div>\n```{language}\nhidden\n```\n</div>\n"),
        format!("prose <!--\n```{language}\nhidden\n```\n-->\n"),
        format!("[reference]:\n```{language}\nhidden\n```\n"),
        format!(":::note\n```{language}\nhidden\n```\n:::\n"),
        format!(" ```{language}\nhidden\n ```\n"),
        format!("  ```{language}\nhidden\n  ```\n"),
        format!("   ```{language}\nhidden\n   ```\n"),
        format!("    ```{language}\n    hidden\n    ```\n"),
        format!("\t```{language}\n\thidden\n\t```\n"),
    ] {
        for source in [
            wrapped.clone(),
            format!("{valid}{wrapped}"),
            format!("{wrapped}{valid}"),
        ] {
            assert!(
                sample_source::extract(source.as_bytes(), language).is_err(),
                "unsupported {language} profile accepted: {source}"
            );
        }
    }
}

fn front_matter_is_opaque_and_encoded_labels_are_rejected(language: &str) {
    let front = format!("---\nexample: \"\n```{language}\nhidden\n```\n\"\n---\n");
    assert!(sample_source::extract(front.as_bytes(), language).is_err());
    let source = format!("{front}```{language}\nactual\n```\n");
    assert_eq!(
        sample_source::extract(source.as_bytes(), language).unwrap(),
        b"actual\n"
    );
    let unclosed = format!("---\ntitle: example\n```{language}\nhidden\n```\n");
    assert!(sample_source::extract(unclosed.as_bytes(), language).is_err());
    for token in [
        format!("{language}&#32;"),
        format!("{language}\\ "),
        format!("{{.{language}}}"),
        format!("{language}`"),
    ] {
        let source = format!("```{language}\nfirst\n```\n```{token}\nsecond\n```\n");
        assert!(
            sample_source::extract(source.as_bytes(), language).is_err(),
            "unsupported info: {source}"
        );
    }
}

macro_rules! profile_tests {
    ($rust:ident, $toml:ident, $check:ident) => {
        #[test]
        fn $rust() {
            $check("rust");
        }
        #[test]
        fn $toml() {
            $check("toml");
        }
    };
}

profile_tests!(
    rust_outer_fence_context,
    toml_outer_fence_context,
    outer_fences_keep_nested_samples_literal
);
profile_tests!(
    rust_info_whitespace,
    toml_info_whitespace,
    target_identity_is_separate_from_opener_whitespace
);
profile_tests!(
    rust_metadata_refusal,
    toml_metadata_refusal,
    target_metadata_is_rejected_instead_of_ignored
);
profile_tests!(
    rust_closing_delimiters,
    toml_closing_delimiters,
    only_matching_sufficient_closers_end_a_fence
);
profile_tests!(
    rust_container_refusal,
    toml_container_refusal,
    unsupported_containers_cannot_hide_or_supply_a_sample
);
profile_tests!(
    rust_front_matter_and_info_refusal,
    toml_front_matter_and_info_refusal,
    front_matter_is_opaque_and_encoded_labels_are_rejected
);
