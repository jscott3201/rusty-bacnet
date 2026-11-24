//! `--format` and `--json` (#2778): which formats each command prints, and
//! the refusal of one it doesn't.
//!
//! `--format` parses to a [`Format`], so an unknown value is clap's usage
//! error, which lists the possible values. Which of them a command takes
//! comes from [`printer`], one arm a command and no catch-all, so a new
//! command doesn't compile until it says. A [`Printer`] names the type the
//! command's output matches on: [`OutputFormat`] for most, the inventory's
//! [`Layout`], or none. The formats a command takes are that type's
//! variants (listed by its derived [`ValueEnum`], each mapped to its
//! [`Format`] by a match, [`Printed::format`]), so they are exactly the arms
//! its output has. [`check`] refuses any other, and `dispatch` runs it
//! ([`refuse`]) before the write guard, a runtime, a transport or a request.
//! Plain `bacnet` with either flag is refused while parsing, where it would
//! open the terminal UI ([`plain_refusal`]).

use std::io::IsTerminal;
use std::process::ExitCode;

use clap::ValueEnum;

use crate::args::{Cli, Command, ExportFormat, McpAction};
use crate::commands::inventory::Layout;
use crate::output::OutputFormat;

/// A `--format` value: each format some command prints.
// No doc comments on the variants: clap would list them in `--help` and
// point `-h` there, for nothing the names don't say.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum Format {
    // A table, or lines, for people.
    Table,
    Json,
    // One row an object: the inventory's alone.
    Csv,
}

/// `format` as `--format` spells it (a test holds it to clap's names).
fn name(format: Format) -> &'static str {
    match format {
        Format::Table => "table",
        Format::Json => "json",
        Format::Csv => "csv",
    }
}

/// A type a command's output matches on: its variants are the formats the
/// command prints.
trait Printed: ValueEnum + Copy {
    /// The `--format` value this variant prints.
    fn format(self) -> Format;
}

impl Printed for OutputFormat {
    fn format(self) -> Format {
        match self {
            Self::Table => Format::Table,
            Self::Json => Format::Json,
        }
    }
}

impl Printed for Layout {
    fn format(self) -> Format {
        match self {
            Self::Table => Format::Table,
            Self::Json => Format::Json,
            Self::Csv => Format::Csv,
        }
    }
}

/// The type a command's output matches on, which says the formats it
/// prints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Printer {
    /// An [`OutputFormat`]: a table or JSON.
    Output,
    /// The inventory's [`Layout`]: a table, JSON or CSV.
    Inventory,
    /// None: the command takes no output format, and this says what it
    /// does instead.
    Own(&'static str),
}

impl Printer {
    /// The formats this printer has an arm for, in its type's order.
    pub(crate) fn formats(self) -> Vec<Format> {
        match self {
            Self::Output => variants::<OutputFormat>(),
            Self::Inventory => variants::<Layout>(),
            Self::Own(_) => Vec::new(),
        }
    }
}

/// What `command` prints in.
pub(crate) fn printer(command: &Command) -> Printer {
    match command {
        Command::Inventory { .. } => Printer::Inventory,
        // The nested commands by name too, so a new one says what it prints.
        Command::Export {
            format: ExportFormat::S223 { .. },
        } => Printer::Own("it writes Turtle"),
        Command::Tui { .. } => Printer::Own("it opens the terminal UI"),
        Command::Mcp {
            action:
                McpAction::Serve { .. } | McpAction::Connect { .. } | McpAction::Authorize { .. },
        } => Printer::Own("it serves MCP or sets it up"),
        Command::ScCheck { .. }
        | Command::Discover { .. }
        | Command::Find { .. }
        | Command::Read { .. }
        | Command::Readm { .. }
        | Command::Write { .. }
        | Command::Writem { .. }
        | Command::Subscribe { .. }
        | Command::Control { .. }
        | Command::Reinit { .. }
        | Command::Alarms { .. }
        | Command::AlarmSummary { .. }
        | Command::EnrollmentSummary { .. }
        | Command::FileRead { .. }
        | Command::FileWrite { .. }
        | Command::MstpPorts
        | Command::Bdt { .. }
        | Command::Fdt { .. }
        | Command::Register { .. }
        | Command::Unregister { .. }
        | Command::WhoisRouter { .. }
        | Command::RoutingTable { .. }
        | Command::AckAlarm { .. }
        | Command::ReadRange { .. }
        | Command::CreateObject { .. }
        | Command::DeleteObject { .. }
        | Command::TimeSync { .. }
        | Command::Capture { .. } => Printer::Output,
    }
}

/// `F`'s variants as the `--format` values they print.
fn variants<F: Printed>() -> Vec<Format> {
    F::value_variants()
        .iter()
        .map(|value| value.format())
        .collect()
}

/// The variant of `F` that prints `format`, if `F` has one.
fn pick<F: Printed>(format: Format) -> Option<F> {
    F::value_variants()
        .iter()
        .copied()
        .find(|value| value.format() == format)
}

/// `a`, `a and b`, `a, b and c`.
fn listed(formats: &[Format]) -> String {
    let names: Vec<&str> = formats.iter().copied().map(name).collect();
    match names.split_last() {
        Some((last, rest)) if !rest.is_empty() => format!("{} and {last}", rest.join(", ")),
        _ => names.concat(),
    }
}

/// The formats the command line names, `--format` first, each with the
/// flag as given.
fn given(cli: &Cli) -> impl Iterator<Item = (Format, String)> {
    let format = cli
        .format
        .map(|format| (format, format!("--format {}", name(format))));
    let json = cli.json.then(|| (Format::Json, "--json".to_owned()));
    format.into_iter().chain(json)
}

/// A usage error with `message` (clap's status, 2).
fn conflict(message: String) -> clap::Error {
    clap::Error::raw(clap::error::ErrorKind::ArgumentConflict, message)
}

/// Refuse a `--format`, or `--json`, that the command doesn't print: a
/// usage error naming the formats it does. `--format` is checked even
/// with `--json`, which outranks it. The inventory's `--csv-exact` needs
/// the CSV, so it is refused with any other layout.
pub(crate) fn check(cli: &Cli) -> Result<(), clap::Error> {
    let Some(command) = &cli.command else {
        return Ok(());
    };
    let printer = printer(command);
    let formats = printer.formats();
    let Some((_, flag)) = given(cli).find(|(format, _)| !formats.contains(format)) else {
        return check_csv_exact(cli, command);
    };
    let command = cli
        .command_name
        .as_deref()
        .map_or_else(|| "bacnet".to_owned(), |name| format!("bacnet {name}"));
    let why = match printer {
        Printer::Own(what) => format!("this command takes no output format ({what})"),
        Printer::Output | Printer::Inventory => {
            format!("this command supports {}", listed(&formats))
        }
    };
    Err(conflict(format!(
        "{command}: {flag} is not available; {why}\n"
    )))
}

/// Refuse `inventory --csv-exact` unless the layout is the CSV, whose
/// formula guard it turns off: with a table, JSON (`--json` outranks
/// `--format csv`) or the default, it would change nothing.
fn check_csv_exact(cli: &Cli, command: &Command) -> Result<(), clap::Error> {
    let Command::Inventory {
        csv_exact: true, ..
    } = command
    else {
        return Ok(());
    };
    match layout(cli) {
        Layout::Csv => Ok(()),
        Layout::Table | Layout::Json => {
            let without = if cli.json { " without --json" } else { "" };
            Err(conflict(format!(
                "bacnet inventory: --csv-exact needs --format csv{without}\n"
            )))
        }
    }
}

/// The refusal of `--format` or `--json` on plain `bacnet` where it opens
/// the terminal UI, named as the user typed it (see `entry`, which calls
/// this before it becomes `bacnet tui`).
pub(crate) fn plain_refusal(cli: &Cli) -> Option<clap::Error> {
    let (_, flag) = given(cli).next()?;
    Some(conflict(format!(
        "bacnet: {flag} is not available; plain bacnet opens the terminal UI, \
         which takes no output format\n"
    )))
}

/// [`check`]'s refusal printed as clap prints a usage error, and the exit
/// status clap gives one (2); `None` when the command takes the format.
pub(crate) fn refuse(cli: &Cli) -> Option<ExitCode> {
    let error = check(cli).err()?;
    let _ = error.print();
    Some(u8::try_from(error.exit_code()).map_or(ExitCode::FAILURE, ExitCode::from))
}

/// The format the command line asked for, `--json` outranking `--format`,
/// or none.
fn asked(cli: &Cli) -> Option<Format> {
    if cli.json {
        Some(Format::Json)
    } else {
        cli.format
    }
}

/// What a command prints in, and its notes and refusals go out in, when the
/// command line names no format of [`OutputFormat`]'s: a table at a
/// terminal, JSON otherwise.
fn auto() -> OutputFormat {
    if std::io::stdout().is_terminal() {
        OutputFormat::Table
    } else {
        OutputFormat::Json
    }
}

/// The [`OutputFormat`] for this command line, after [`check`]. With
/// `--format csv` (the inventory's) it is the one the notes on stderr use.
pub(crate) fn output(cli: &Cli) -> OutputFormat {
    asked(cli).and_then(pick).unwrap_or_else(auto)
}

/// The inventory's [`Layout`] for this command line, after [`check`].
pub(crate) fn layout(cli: &Cli) -> Layout {
    asked(cli)
        .and_then(pick)
        .unwrap_or_else(|| match output(cli) {
            OutputFormat::Table => Layout::Table,
            OutputFormat::Json => Layout::Json,
        })
}

#[cfg(test)]
#[path = "formats_tests.rs"]
mod tests;
