//! The published manifest of a website guide program (#2648). A program
//! unchanged since the release pins each BACnet crate to its published
//! version; one changed since takes them from the repository's `dev` branch,
//! a git dependency, since the workspace compiles it against `dev`
//! (`scripts/release/website_samples.py --pin` and `--follow-dev`). [`as_dev`]
//! takes either form and returns the dev form, so one expected manifest covers
//! both. Include it as a module named `guide_manifest`.

use std::path::Path;
use toml::Value;
use toml::map::Map;

pub const REPOSITORY: &str = "https://github.com/jscott3201/rusty-bacnet";

fn read(path: &Path) -> Value {
    toml::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// The version the workspace gives a crate, which a release publishes: its
/// own (bacnet-time versions on its own) or the workspace's.
pub fn published_version(krate: &str) -> String {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let package = &read(&crates.join(krate).join("Cargo.toml"))["package"];
    match &package["version"] {
        Value::String(own) => own.clone(),
        _ => read(&crates.parent().unwrap().join("Cargo.toml"))["workspace"]["package"]["version"]
            .as_str()
            .unwrap()
            .to_owned(),
    }
}

/// A BACnet crate from the dev branch, as the manifests write it.
pub fn dev(krate: &str) -> String {
    format!("{krate} = {{ git = \"{REPOSITORY}\", branch = \"dev\" }}")
}

/// `manifest` with each `bacnet-*` dependency in the dev form. Fails on one in
/// neither form, on a pin to another version than the crate's published one,
/// and on a mix.
pub fn as_dev(manifest: &str) -> Result<Value, String> {
    let mut value: Value = toml::from_str(manifest).map_err(|e| e.to_string())?;
    let deps = value
        .get_mut("dependencies")
        .and_then(Value::as_table_mut)
        .ok_or("no [dependencies]")?;
    let (mut pinned, mut on_dev) = (0, 0);
    for (name, dep) in deps
        .iter_mut()
        .filter(|(name, _)| name.starts_with("bacnet-"))
    {
        let pin = format!("={}", published_version(name));
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

/// Each `bacnet-*` line of `manifest` mapped by `line`.
fn lines(manifest: &str, line: impl Fn(&str, &str) -> String) -> String {
    manifest
        .split_inclusive('\n')
        .map(|text| match text.split_once(" = ") {
            Some((krate, rest)) if krate.starts_with("bacnet-") => {
                let newline = if rest.ends_with('\n') { "\n" } else { "" };
                format!("{krate} = {}{newline}", line(krate, rest.trim_end()))
            }
            _ => text.to_owned(),
        })
        .collect()
}

/// `manifest` as the release script pins it: each crate at its published version.
pub fn pinned(manifest: &str) -> String {
    let dev = format!("{{ git = \"{REPOSITORY}\", branch = \"dev\"");
    lines(manifest, |krate, value| {
        let version = published_version(krate);
        match value.strip_prefix(&dev) {
            Some(" }") => format!("\"={version}\""),
            Some(rest) => format!("{{ version = \"={version}\"{rest}"),
            None => value.to_owned(),
        }
    })
}

/// `manifest` as the release script takes it back to the dev branch.
pub fn on_dev(manifest: &str) -> String {
    let dev = format!("{{ git = \"{REPOSITORY}\", branch = \"dev\"");
    lines(manifest, |krate, value| {
        let version = published_version(krate);
        if value == format!("\"={version}\"") {
            format!("{dev} }}")
        } else if let Some(rest) = value.strip_prefix(&format!("{{ version = \"={version}\"")) {
            format!("{dev}{rest}")
        } else {
            value.to_owned()
        }
    })
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
    let crates: Vec<_> = dev.lines().filter(|line| line.contains(&source)).collect();
    let first = crates[0].split_once(" = ").unwrap().0;
    let pin = format!("\"={}\"", published_version(first));
    let pinned_first = pinned(crates[0]);
    let mut invalid = vec![
        release.replacen(&pin, "\"=0.1.0\"", 1),
        release.replacen(&pin, &pin.replace('=', ""), 1),
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
    if crates.len() > 1 {
        invalid.push(dev.replacen(crates[0], &pinned_first, 1));
    }
    for invalid in invalid {
        assert!(contract(&invalid).is_err(), "accepted drift: {invalid}");
    }
}
