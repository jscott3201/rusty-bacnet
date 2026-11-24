//! `formats`' tests (#2778): the formats each command takes, as read from
//! what each prints; the refusal's text; and what `--json` and `--format`
//! pick.

use clap::{CommandFactory, FromArgMatches};

use super::*;

const TABLE_JSON: &[&str] = &["table", "json"];

/// Every command, by a command line that parses, and the formats it prints
/// (read from its output code, not its help). A command added without a
/// row here fails [`every_command_takes_the_formats_it_prints`]; one added
/// without an arm in [`printer`] doesn't compile.
const COMMANDS: &[(&[&str], &[&str])] = &[
    (&["sc-check", "--sc-cert", "node.pem"], TABLE_JSON),
    (&["tui"], &[]),
    (&["mcp", "serve", "--stdio", "--passive"], &[]),
    (&["discover"], TABLE_JSON),
    (&["find", "AHU-1"], TABLE_JSON),
    (&["read", "10.0.0.1", "ai:1", "pv"], TABLE_JSON),
    (&["export", "223-experimental", "10.0.0.1"], &[]),
    (&["inventory", "10.0.0.1"], &["table", "json", "csv"]),
    (&["readm", "10.0.0.1", "ai:1", "pv"], TABLE_JSON),
    (&["write", "10.0.0.1", "av:1", "pv", "1"], TABLE_JSON),
    (&["writem", "10.0.0.1", "av:1", "pv=1"], TABLE_JSON),
    (&["subscribe", "10.0.0.1", "ai:1"], TABLE_JSON),
    (&["control", "10.0.0.1", "enable"], TABLE_JSON),
    (&["reinit", "10.0.0.1", "warmstart"], TABLE_JSON),
    (&["alarms", "10.0.0.1"], TABLE_JSON),
    (&["alarm-summary", "10.0.0.1"], TABLE_JSON),
    (&["enrollment-summary", "10.0.0.1"], TABLE_JSON),
    (&["file-read", "10.0.0.1", "1"], TABLE_JSON),
    (&["file-write", "10.0.0.1", "1", "in.bin"], TABLE_JSON),
    (&["mstp-ports"], TABLE_JSON),
    (&["bdt", "10.0.0.1"], TABLE_JSON),
    (&["fdt", "10.0.0.1"], TABLE_JSON),
    (&["register", "10.0.0.1"], TABLE_JSON),
    (&["unregister", "10.0.0.1"], TABLE_JSON),
    (&["whois-router"], TABLE_JSON),
    (&["routing-table", "10.0.0.1"], TABLE_JSON),
    (
        &[
            "ack-alarm",
            "10.0.0.1",
            "ai:1",
            "--state",
            "1",
            "--timestamp",
            "sequence:1",
            "--ack-time",
            "sequence:2",
        ],
        TABLE_JSON,
    ),
    (&["read-range", "10.0.0.1", "tl:1"], TABLE_JSON),
    (&["create-object", "10.0.0.1", "av:100"], TABLE_JSON),
    (&["delete-object", "10.0.0.1", "av:100"], TABLE_JSON),
    (&["time-sync", "10.0.0.1"], TABLE_JSON),
    (&["capture"], TABLE_JSON),
];

/// Parse `bacnet` and `args` as `main` does, on its larger stack.
fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
    let args: Vec<String> = std::iter::once("bacnet")
        .chain(args.iter().copied())
        .map(str::to_owned)
        .collect();
    crate::parse_on_large_stack(move || {
        let mut matches = Cli::command().try_get_matches_from(args)?;
        let command_name = matches.subcommand_name().map(str::to_owned);
        let mut cli = Cli::from_arg_matches_mut(&mut matches)?;
        cli.command_name = command_name;
        Ok(cli)
    })
}

/// `args` with `extra` after them.
fn with<'a>(args: &[&'a str], extra: &[&'a str]) -> Vec<&'a str> {
    args.iter().chain(extra).copied().collect()
}

fn names(formats: Vec<Format>) -> Vec<&'static str> {
    formats.into_iter().map(name).collect()
}

/// [`name`] spells each value as clap parses it, and each output type's
/// variant prints the `--format` value of its own name.
#[test]
fn names_are_clap_s_and_each_variant_prints_its_own() {
    fn spelled<F: ValueEnum>(value: &F) -> String {
        value.to_possible_value().unwrap().get_name().to_owned()
    }
    for format in Format::value_variants() {
        assert_eq!(name(*format), spelled(format));
    }
    for format in OutputFormat::value_variants() {
        assert_eq!(name(format.format()), spelled(format));
    }
    for layout in Layout::value_variants() {
        assert_eq!(name(layout.format()), spelled(layout));
    }
}

/// The refusal [`check`] gives `args`, as clap prints it.
fn refusal(args: &[&str]) -> String {
    let cli = parse(args).unwrap_or_else(|error| panic!("{args:?}: {error}"));
    let error = check(&cli).expect_err("refused");
    assert_eq!(error.kind(), clap::error::ErrorKind::ArgumentConflict);
    assert_eq!(error.exit_code(), 2, "a usage error");
    error.to_string()
}

/// Each command declares the formats it prints, and takes exactly those:
/// `--format` with each, and `--json` where it prints JSON, pass; any
/// other is refused, naming the ones it prints.
#[test]
fn every_command_takes_the_formats_it_prints() {
    let commands: Vec<String> = crate::parse_on_large_stack(|| {
        Cli::command()
            .get_subcommands()
            .map(|command| command.get_name().to_owned())
            .collect()
    });
    for command in &commands {
        let rows = COMMANDS.iter().filter(|(args, _)| args[0] == command);
        assert_eq!(rows.count(), 1, "bacnet {command} has one row");
    }
    assert_eq!(COMMANDS.len(), commands.len(), "a row a command");
    for &(args, expected) in COMMANDS {
        let cli = parse(args).unwrap_or_else(|error| panic!("{args:?}: {error}"));
        let declared = names(printer(cli.command.as_ref().unwrap()).formats());
        assert_eq!(declared, expected, "{args:?}");
        for format in Format::value_variants() {
            let given = name(*format);
            let cli = parse(&with(args, &["--format", given])).unwrap();
            let takes = expected.contains(&given);
            assert_eq!(check(&cli).is_ok(), takes, "{args:?} --format {given}");
        }
        let cli = parse(&with(args, &["--json"])).unwrap();
        assert_eq!(
            check(&cli).is_ok(),
            expected.contains(&"json"),
            "{args:?} --json"
        );
        // Neither flag is the default output, which every command takes.
        assert!(check(&parse(args).unwrap()).is_ok(), "{args:?}");
    }
}

/// The formats some command prints are the `--format` values, each one
/// printed by some command: none is offered that nothing prints.
#[test]
fn every_format_value_is_printed_by_some_command() {
    let mut printed: Vec<Format> = [Printer::Output, Printer::Inventory]
        .into_iter()
        .flat_map(Printer::formats)
        .collect();
    printed.sort_by_key(|format| name(*format));
    printed.dedup();
    let mut values = Format::value_variants().to_vec();
    values.sort_by_key(|format| name(*format));
    assert_eq!(printed, values);
}

/// The refusal names the command (its full name, also for an alias), the
/// flag and the formats it prints, or that it takes none.
#[test]
fn a_refusal_names_the_command_and_its_formats() {
    let read = "error: bacnet read: --format csv is not available; \
                this command supports table and json\n";
    for args in [
        &["read", "10.0.0.1", "ai:1", "pv", "--format", "csv"][..],
        &["--format", "csv", "rp", "10.0.0.1", "ai:1", "pv"],
        // `--format` is checked even though `--json` outranks it.
        &[
            "--json", "read", "10.0.0.1", "ai:1", "pv", "--format", "csv",
        ],
    ] {
        assert_eq!(refusal(args), read, "{args:?}");
    }
    assert_eq!(
        refusal(&["--json", "export", "223-experimental", "10.0.0.1"]),
        "error: bacnet export: --json is not available; \
         this command takes no output format (it writes Turtle)\n"
    );
    assert_eq!(
        refusal(&["tui", "--format", "table"]),
        "error: bacnet tui: --format table is not available; \
         this command takes no output format (it opens the terminal UI)\n"
    );
    assert_eq!(
        refusal(&["mcp", "serve", "--stdio", "--passive", "--json"]),
        "error: bacnet mcp: --json is not available; \
         this command takes no output format (it serves MCP or sets it up)\n"
    );
    assert_eq!(
        refusal(&["sc-check", "--sc-cert", "node.pem", "--format", "csv"]),
        "error: bacnet sc-check: --format csv is not available; \
         this command supports table and json\n"
    );
}

/// `inventory --csv-exact` needs the CSV: refused with a table, JSON
/// (`--json` outranks `--format csv`) or no format, taken with
/// `--format csv`.
#[test]
fn csv_exact_needs_the_csv() {
    let inventory = ["inventory", "10.0.0.1", "--csv-exact"];
    let needs = "error: bacnet inventory: --csv-exact needs --format csv\n";
    let without = "error: bacnet inventory: --csv-exact needs --format csv without --json\n";
    for (extra, message) in [
        (&[][..], needs),
        (&["--format", "table"], needs),
        (&["--format", "json"], needs),
        (&["--json"], without),
        (&["--format", "csv", "--json"], without),
    ] {
        assert_eq!(refusal(&with(&inventory, extra)), message, "{extra:?}");
    }
    let csv = parse(&with(&inventory, &["--format", "csv"])).unwrap();
    assert!(check(&csv).is_ok());
}

/// An unknown `--format` is clap's usage error, on either side of the
/// command, listing the possible values.
#[test]
fn an_unknown_format_is_refused_with_the_possible_values() {
    for args in [
        &["--format", "jsn", "read", "10.0.0.1", "ai:1", "pv"][..],
        &["inventory", "10.0.0.1", "--format", "jsn"],
    ] {
        let error = parse(args).map(drop).expect_err("refused");
        assert_eq!(
            error.kind(),
            clap::error::ErrorKind::InvalidValue,
            "{args:?}"
        );
        assert_eq!(error.exit_code(), 2);
        let text = error.to_string();
        assert!(
            text.contains("invalid value 'jsn' for '--format <FORMAT>'")
                && text.contains("[possible values: table, json, csv]"),
            "{text}"
        );
    }
}

/// `--json` outranks `--format`; csv is the inventory's layout, and with
/// it the notes on stderr take the default; neither flag is the default.
#[test]
fn json_and_format_pick_the_output() {
    let inventory =
        |extra: &[&'static str]| parse(&with(&["inventory", "10.0.0.1"], extra)).unwrap();
    let cli = inventory(&["--format", "csv"]);
    assert_eq!((layout(&cli), output(&cli)), (Layout::Csv, auto()));
    let cli = inventory(&["--format", "csv", "--json"]);
    assert_eq!(
        (layout(&cli), output(&cli)),
        (Layout::Json, OutputFormat::Json)
    );
    let cli = inventory(&["--format", "table"]);
    assert_eq!(
        (layout(&cli), output(&cli)),
        (Layout::Table, OutputFormat::Table)
    );
    let cli = inventory(&[]);
    let auto_layout = match auto() {
        OutputFormat::Table => Layout::Table,
        OutputFormat::Json => Layout::Json,
    };
    assert_eq!((layout(&cli), output(&cli)), (auto_layout, auto()));

    let read =
        |extra: &[&'static str]| parse(&with(&["read", "10.0.0.1", "ai:1", "pv"], extra)).unwrap();
    assert_eq!(output(&read(&["--format", "table"])), OutputFormat::Table);
    assert_eq!(output(&read(&["--format", "json"])), OutputFormat::Json);
    assert_eq!(
        output(&read(&["--format", "table", "--json"])),
        OutputFormat::Json
    );
    assert_eq!(output(&read(&[])), auto());
}
