//! The published manifest of a website guide program (#2648). Between
//! releases its BACnet crates come from the repository's `dev` branch, a git
//! dependency, since the workspace compiles the program against `dev`; a
//! release pins them to itself (`scripts/release/website_samples.py --pin`).
//! [`as_dev`] takes either form and returns the dev form, so one expected
//! manifest covers both. Include it as a module named `guide_manifest`.

use toml::Value;
use toml::map::Map;

pub const REPOSITORY: &str = "https://github.com/jscott3201/rusty-bacnet";
const SITE: &str = include_str!("../../../../website/src/lib/site.mjs");

/// The release the website describes, from `website/src/lib/site.mjs`.
pub fn site_release() -> &'static str {
    SITE.lines()
        .find_map(|line| {
            line.strip_prefix("export const release = '")?
                .strip_suffix("';")
        })
        .expect("site.mjs exports the release")
}

/// A BACnet crate from the dev branch, as the manifests write it.
pub fn dev(krate: &str) -> String {
    format!("{krate} = {{ git = \"{REPOSITORY}\", branch = \"dev\" }}")
}

/// `manifest` with each `bacnet-*` dependency in the dev form. Fails on one in
/// neither form, on a pin to another release than the site's, and on a mix.
pub fn as_dev(manifest: &str) -> Result<Value, String> {
    let mut value: Value = toml::from_str(manifest).map_err(|e| e.to_string())?;
    let pin = format!("={}", site_release());
    let deps = value
        .get_mut("dependencies")
        .and_then(Value::as_table_mut)
        .ok_or("no [dependencies]")?;
    let (mut pinned, mut on_dev) = (0, 0);
    for (name, dep) in deps
        .iter_mut()
        .filter(|(name, _)| name.starts_with("bacnet-"))
    {
        let mut table = match dep {
            Value::String(version) => {
                Map::from_iter([("version".into(), Value::from(version.as_str()))])
            }
            Value::Table(table) => table.clone(),
            other => return Err(format!("{name}: {other}")),
        };
        let field = |key| table.get(key).and_then(Value::as_str).map(str::to_owned);
        match (field("version"), field("git"), field("branch")) {
            (Some(version), None, None) if version == pin => {
                table.remove("version");
                table.insert("git".into(), REPOSITORY.into());
                table.insert("branch".into(), "dev".into());
                pinned += 1;
            }
            (None, Some(git), Some(branch)) if git == REPOSITORY && branch == "dev" => on_dev += 1,
            _ => {
                return Err(format!(
                    "{name} is neither on the dev branch nor at {pin}: {dep}"
                ));
            }
        }
        *dep = Value::Table(table);
    }
    match (pinned, on_dev) {
        (0, 0) => Err("no BACnet crate".into()),
        (0, _) | (_, 0) => Ok(value),
        _ => Err(format!("{pinned} crates pinned and {on_dev} on dev")),
    }
}

/// `manifest` as the release script pins it to the site's release.
pub fn pinned(manifest: &str) -> String {
    let release = site_release();
    manifest
        .replace(
            &format!("{{ git = \"{REPOSITORY}\", branch = \"dev\" }}"),
            &format!("\"={release}\""),
        )
        .replace(
            &format!("{{ git = \"{REPOSITORY}\", branch = \"dev\", "),
            &format!("{{ version = \"={release}\", "),
        )
}

/// `manifest` as the release script takes it back to the dev branch.
pub fn on_dev(manifest: &str) -> String {
    let release = site_release();
    manifest
        .replace(
            &format!("{{ version = \"={release}\", "),
            &format!("{{ git = \"{REPOSITORY}\", branch = \"dev\", "),
        )
        .replace(
            &format!("\"={release}\""),
            &format!("{{ git = \"{REPOSITORY}\", branch = \"dev\" }}"),
        )
}

/// The committed manifest, in whichever form, passes `contract`; so do its dev
/// and pinned forms, and every way of leaving those two forms is refused.
/// `contract` compares a manifest, through [`as_dev`], with the expected one.
pub fn check_forms(manifest: &str, contract: impl Fn(&str) -> Result<(), String>) {
    let dev = on_dev(manifest);
    let release = pinned(&dev);
    assert_eq!(on_dev(&release), dev, "the two forms map onto each other");
    assert_ne!(release, dev);
    for form in [manifest, &dev, &release] {
        contract(form).unwrap();
    }
    let source = format!("{{ git = \"{REPOSITORY}\", branch = \"dev\"");
    let mut invalid = vec![
        release.replace(&format!("\"={}\"", site_release()), "\"=0.1.0\""),
        release.replace(
            &format!("\"={}\"", site_release()),
            &format!("\"{}\"", site_release()),
        ),
        dev.replacen(
            &source,
            &format!("{{ git = \"{REPOSITORY}\", branch = \"main\""),
            1,
        ),
        dev.replacen(
            &source,
            "{ git = \"https://example.invalid/fork\", branch = \"dev\"",
            1,
        ),
        dev.replacen(&source, &format!("{source}, path = \"../local\""), 1),
        dev.replacen(&source, &format!("{source}, rev = \"0123456\""), 1),
        dev.replacen(&source, &format!("{source}, default-features = false"), 1),
        format!("{dev}\n[patch.crates-io]\nbacnet-types = {{ path = \"../local\" }}\n"),
        dev.replace("[workspace]\n", ""),
        dev.replace("rust-version = \"1.95\"", "rust-version = \"1.93\""),
    ];
    // One crate pinned, the rest on dev.
    let crates: Vec<_> = dev.lines().filter(|line| line.contains(&source)).collect();
    if crates.len() > 1 {
        invalid.push(dev.replacen(crates[0], &pinned(crates[0]), 1));
    }
    for invalid in invalid {
        assert!(contract(&invalid).is_err(), "accepted drift: {invalid}");
    }
}
