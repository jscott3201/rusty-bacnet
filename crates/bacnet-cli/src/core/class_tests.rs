//! The shared class table ([`bacnet_ops::class`]) against this binary's
//! clap definitions. The table's own pin test lives with it.

use bacnet_ops::class::{COMMANDS, of};
use clap::CommandFactory;

/// Every clap subcommand and alias, from the derived definition.
fn clap_names() -> Vec<(String, Vec<String>)> {
    crate::parse_on_large_stack(|| {
        crate::args::Cli::command()
            .get_subcommands()
            .map(|sub| {
                (
                    sub.get_name().to_string(),
                    sub.get_all_aliases().map(str::to_string).collect(),
                )
            })
            .collect()
    })
}

/// A new subcommand can't skip classification, and an
/// alias has its command's class.
#[test]
fn every_command_and_alias_has_a_class() {
    let clap = clap_names();
    assert!(clap.len() > 20, "{clap:?}");
    for (name, aliases) in &clap {
        let class = of(name).unwrap_or_else(|| panic!("subcommand {name} has no class"));
        for alias in aliases {
            assert_eq!(of(alias), Some(class), "alias {alias} of {name}");
        }
    }
    // And the table names nothing that no longer exists.
    for (name, _) in COMMANDS {
        let known = clap
            .iter()
            .any(|(n, aliases)| n == name || aliases.iter().any(|a| a == name))
            || *name == "help";
        assert!(known, "{name} is classified but is no command");
    }
}
