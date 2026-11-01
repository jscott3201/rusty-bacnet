//! The seed inputs in `seeds/<target>/`: real saved files, built here.
//!
//! Each format's sample from `restore.rs` is saved through its backend in a
//! scratch directory, and its files are laid out as the target's input.
//! [`generated_seeds_are_saved`] holds the files in `seeds/` to what this
//! module builds: `SEEDS_WRITE=1 cargo test` writes them again. A minimized
//! failing input saved under `seeds/<target>/` is replayed like any other
//! seed.

use std::fs;
use std::path::{Path, PathBuf};

use crate::restore::{Format, Outcome, Scratch, read_files};
use crate::{Target, seal};

const TARGETS: [(&str, Target); 3] = [
    ("log_restore", Target::Log),
    ("audit_restore", Target::Audit),
    ("object_file_restore", Target::ObjectFile),
];

struct Seed {
    target: &'static str,
    name: String,
    octets: Vec<u8>,
}

fn directory(target: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("seeds")
        .join(target)
}

/// The files `format`'s sample saves.
fn sample(format: &Format) -> Vec<Option<Vec<u8>>> {
    let scratch = Scratch::new("objects-fuzz-seeds");
    let base = scratch.base("state");
    (format.sample)(&base);
    read_files(&format.files(&base))
}

/// A two-slot input: the flags octet, slot 0's length and both slots.
fn slots(selector: u8, flags: u8, first: &[u8], second: &[u8]) -> Vec<u8> {
    let mut octets = vec![selector, flags];
    octets.extend_from_slice(&(first.len() as u16).to_be_bytes());
    octets.extend_from_slice(first);
    octets.extend_from_slice(second);
    octets
}

/// The Audit Log sample as schema version `version` (1 or 2), sealed. The
/// layout up to the records is the same in every version; version 1 ends
/// there, without the completed receipts, which in the sample take 19
/// octets (a count, then one receipt with a 3-octet key).
fn audit_version(slot: &[u8], version: u16) -> Vec<u8> {
    let mut slot = slot.to_vec();
    slot[8..10].copy_from_slice(&version.to_be_bytes());
    if version == 1 {
        let at = slot.len() - 4 - 19;
        slot.drain(at..slot.len() - 4);
    }
    seal(&mut slot);
    slot
}

fn generated() -> Vec<Seed> {
    let mut seeds = Vec::new();
    for (target, kind) in TARGETS {
        for (selector, format) in kind.formats().into_iter().enumerate() {
            let selector = u8::try_from(selector).expect("a selector octet");
            let mut seed = |suffix: &str, octets| {
                seeds.push(Seed {
                    target,
                    name: format!("{}{suffix}", format.name),
                    octets,
                });
            };
            let files = sample(format);
            if !format.two_slot {
                let mut octets = vec![selector];
                octets.extend_from_slice(files[0].as_ref().expect("a saved file"));
                seed("", octets);
                continue;
            }
            // The sample's generation is odd, so it lands in slot 1.
            let slot = files[1].as_ref().expect("a saved slot");
            seed("", slots(selector, 0b1010, &[], slot));
            seed("-unsealed", slots(selector, 0b0010, &[], slot));
            seed("-both-slots", slots(selector, 0b1111, slot, slot));
            if format.name == "audit_log" {
                for version in [1, 2] {
                    let slot = audit_version(slot, version);
                    seed(&format!("-v{version}"), slots(selector, 0b1010, &[], &slot));
                }
            }
        }
    }
    seeds
}

/// The saved files hold exactly the generated seeds (with `SEEDS_WRITE`
/// set, writes them first), and each one restores: it loads and builds.
#[test]
fn generated_seeds_are_saved() {
    let write = std::env::var_os("SEEDS_WRITE").is_some();
    for seed in generated() {
        let path = directory(seed.target).join(&seed.name);
        if write {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, &seed.octets).unwrap();
        }
        let saved = fs::read(&path).unwrap_or_else(|error| {
            panic!(
                "{}: {error}; SEEDS_WRITE=1 cargo test writes it",
                path.display()
            )
        });
        assert_eq!(saved, seed.octets, "{} is stale", path.display());
        let (_, kind) = TARGETS
            .into_iter()
            .find(|(target, _)| *target == seed.target)
            .unwrap();
        assert_eq!(
            crate::check(kind, &seed.octets),
            Outcome {
                loaded: true,
                built: true
            },
            "{} doesn't restore",
            seed.name
        );
    }
}

/// Every seed file, generated or not, runs its target's check clean, and
/// every format has one.
#[test]
fn every_seed_passes() {
    for (target, kind) in TARGETS {
        let mut seeded = vec![false; kind.formats().len()];
        for entry in fs::read_dir(directory(target)).expect("the seeds directory") {
            let octets = fs::read(entry.expect("a directory entry").path()).unwrap();
            crate::check(kind, &octets);
            if let Some(&selector) = octets.first() {
                let count = seeded.len();
                seeded[usize::from(selector) % count] = true;
            }
        }
        assert!(
            seeded.iter().all(|&seeded| seeded),
            "{target}: a format has no seed"
        );
    }
}
