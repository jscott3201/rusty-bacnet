//! Workspace source guard for the endpoint-private ownership adapter.
//!
//! This unpublished integration-test crate reads sources at test runtime and
//! requires the workspace source tree. Published crates gain no dependency on
//! sibling source files. Endpoint source_reporter_tests still cover behavioral
//! forwarding; method presence alone cannot prove delegation.

use std::collections::BTreeSet;
use std::path::Path;

use syn::{ImplItem, Item, TraitItem, Type};

fn parse_source(path: &Path) -> syn::File {
    let source = std::fs::read_to_string(path).unwrap_or_else(|error| {
        panic!(
            "workspace source required for completeness guard: {}: {error}",
            path.display()
        )
    });
    syn::parse_file(&source)
        .unwrap_or_else(|error| panic!("cannot parse {}: {error}", path.display()))
}

#[test]
fn source_reporter_explicitly_handles_every_bacnet_object_method() {
    let package = Path::new(env!("CARGO_MANIFEST_DIR"));
    let objects = parse_source(&package.join("../bacnet-objects/src/traits.rs"));
    let adapter = parse_source(&package.join("../bacnet-endpoint/src/source_reporter.rs"));

    let traits: Vec<_> = objects
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Trait(item) if item.ident == "BACnetObject" => Some(item),
            _ => None,
        })
        .collect();
    assert_eq!(traits.len(), 1, "expected exactly one BACnetObject trait");
    let implementations: Vec<_> = adapter
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Impl(item)
                if item.trait_.as_ref().is_some_and(|(negation, path, _)| {
                    negation.is_none() && path.is_ident("BACnetObject")
                }) && matches!(item.self_ty.as_ref(), Type::Path(ty)
                    if ty.qself.is_none() && ty.path.is_ident("SourceReporter")) =>
            {
                Some(item)
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        implementations.len(),
        1,
        "expected exactly one impl BACnetObject for SourceReporter"
    );

    let required: BTreeSet<_> = traits[0]
        .items
        .iter()
        .filter_map(|item| match item {
            TraitItem::Fn(method) => Some(method.sig.ident.to_string()),
            TraitItem::Macro(_) | TraitItem::Verbatim(_) => {
                panic!("BACnetObject has unexpanded syntax; cannot prove method completeness")
            }
            _ => None,
        })
        .collect();
    let explicit: BTreeSet<_> = implementations[0]
        .items
        .iter()
        .filter_map(|item| match item {
            ImplItem::Fn(method) => Some(method.sig.ident.to_string()),
            ImplItem::Macro(_) | ImplItem::Verbatim(_) => {
                panic!("SourceReporter has unexpanded syntax; cannot prove method completeness")
            }
            _ => None,
        })
        .collect();

    // Include defaulted methods: inheriting a default can silently discard a
    // custom wrapped object's override. Rust checks each implementation signature.
    assert_eq!(
        required,
        explicit,
        "SourceReporter must explicitly handle all BACnetObject methods; missing: {:?}; extra: {:?}",
        required.difference(&explicit).collect::<Vec<_>>(),
        explicit.difference(&required).collect::<Vec<_>>()
    );
}
