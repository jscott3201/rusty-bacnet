//! The website's TUI samples, read from the pages and checked by the CLI's
//! own code (#1989): the profile and `[mcp]` tables load through the profile
//! store, and the command-bar lines parse as the command bar reads them.
//! `website/tests/python/site_samples.py` lists these blocks as covered here.

use std::path::{Path, PathBuf};

use bacnet_ops::target::routed_station;
use bacnet_types::enums::Segmentation;

use crate::tui::app::command_bar::{self, Line};
use crate::tui::app::devices::DeviceTable;
use crate::tui::message::DeviceRow;
use crate::tui::profile::store::Store;

/// A page under the website's docs.
fn page(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../website/src/content/docs")
        .join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The fenced block of `page` whose first non-blank line is `first`, its
/// lines' common indentation removed, as `site_samples.py` keys it.
fn block(page: &str, first: &str) -> String {
    let mut found = Vec::new();
    let mut fence: Option<Vec<&str>> = None;
    for line in page.lines() {
        let stripped = line.trim();
        match &mut fence {
            Some(body) if stripped == "```" => {
                if body.iter().find(|l| !l.trim().is_empty()).map(|l| l.trim()) == Some(first) {
                    found.push(std::mem::take(body));
                }
                fence = None;
            }
            Some(body) => body.push(line),
            None if stripped.starts_with("```") => fence = Some(Vec::new()),
            None => {}
        }
    }
    assert_eq!(found.len(), 1, "one block starting {first:?}");
    let body = found.remove(0);
    let indent = body
        .iter()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.len() - l.trim_start().len())
        .min()
        .unwrap_or(0);
    body.iter()
        .map(|l| l.get(indent..).unwrap_or("").trim_end())
        .collect::<Vec<_>>()
        .join("\n")
}

/// `text` saved as profile `name` in a fresh store, loaded back through the
/// store's own loader: the size cap, the TOML parse that refuses unknown
/// fields, and the profile's validation.
fn load(text: &str) -> Result<crate::tui::profile::Profile, String> {
    let root = tempfile::tempdir().unwrap();
    let profiles: PathBuf = root.path().join("profiles");
    std::fs::create_dir_all(&profiles).unwrap();
    std::fs::write(profiles.join("site.toml"), text).unwrap();
    Store::new(root.path().to_path_buf()).load("site")
}

/// The profile page's example profile.
fn profile_sample() -> String {
    block(&page("tui/profiles.mdx"), "version = 1")
}

#[test]
fn the_profile_page_example_loads() {
    let profile = load(&profile_sample()).unwrap();
    assert_eq!(profile.version, 1);
    assert!(profile.bip.is_some());
}

/// Each `[mcp]` table on the agents page, added to the profile page's
/// profile, loads: its keys are fields of the section, and its values pass
/// the checks the TUI and the server make before serving.
#[test]
fn the_agents_page_mcp_tables_load_in_a_profile() {
    let agents = page("guides/ai-agents.md");
    for first in ["[mcp]", "# A plan grant: writes approved in advance"] {
        let table = block(&agents, first);
        let profile = load(&format!("{}\n\n{table}\n", profile_sample()))
            .unwrap_or_else(|e| panic!("{first}: {e}"));
        let mcp = profile.mcp.expect("an [mcp] section");
        assert_eq!(mcp.plan.is_some(), first.starts_with("# A plan"), "{first}");
    }
    // A misspelt key is refused, so a page that drifts from the section fails.
    let table = block(&agents, "[mcp]").replace("slow_networks", "slow_network");
    assert!(load(&format!("{}\n\n{table}\n", profile_sample())).is_err());
}

/// A device table with 1001 on the local network and 2001 behind network 2.
fn devices() -> DeviceTable {
    let mut table = DeviceTable::default();
    for (instance, station) in [(1001, None), (2001, Some((2u16, [5u8])))] {
        table.upsert(DeviceRow {
            instance,
            address: match station {
                Some((network, mac)) => routed_station(network, &mac).unwrap(),
                None => "10.0.1.20:47808".into(),
            },
            via: station.map(|_| "10.0.1.1:47808".into()),
            network: station.map(|(network, _)| network),
            vendor_id: 260,
            max_apdu: 1476,
            segmentation: Segmentation::NO_SEGMENTATION,
            last_seen: std::time::Instant::now(),
        });
    }
    table
}

/// Every command-bar line on the watch and writes pages parses, as the
/// bar reads it after its `:`, into the command the page describes.
#[test]
fn the_command_bar_lines_parse() {
    let devices = devices();
    let watch = page("tui/watch.mdx");
    let writes = page("tui/writes.mdx");
    let lines = [
        (block(&watch, ":watch 1001 ai:2"), "watch"),
        (
            block(
                &writes,
                ":writem 1001 av:1 present-value 72@12 av:2 present-value 68@12",
            ),
            "writem",
        ),
        (
            block(
                &writes,
                ":writem 1001 av:1 description \"Zone 2 supply\"@12",
            ),
            "writem",
        ),
        (block(&writes, ":dcc disable-initiation 30"), "dcc"),
    ];
    for (text, kind) in &lines {
        for line in text.lines() {
            let command = line.strip_prefix(':').expect("a command-bar line");
            let parsed =
                command_bar::parse(command, &devices).unwrap_or_else(|e| panic!("{line}: {e}"));
            let matches = match (*kind, &parsed) {
                ("watch", Line::Watch(_)) | ("dcc", Line::Manage(_)) => true,
                ("writem", Line::Writem(specs)) => !specs.is_empty(),
                _ => false,
            };
            assert!(matches, "{line}: {parsed:?}");
        }
    }
    // A misspelt object type is refused, so the check sees drift.
    assert!(command_bar::parse("watch 1001 zz:2", &devices).is_err());
}
