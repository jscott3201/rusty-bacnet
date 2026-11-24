//! `--format` and `--json` (#2778): which formats each command prints, and
//! the refusal of one it doesn't.
//!
//! `--format` parses to a [`Format`], so an unknown value is clap's usage
//! error, which lists the possible values. Which of them a command takes
//! comes from [`printer`], one arm a command and no catch-all, so a new
//! command doesn't compile until it says. A [`Printer`] names the type the
//! command's output matches on: [`OutputFormat`] for most, the inventory's
//! [`Layout`], or none. The formats a command takes are that type's
//! variants, read from its derived [`ValueEnum`], so they are exactly the
//! arms its output has. [`check`] refuses any other, and `dispatch` runs it
//! ([`refuse`]) before the write guard, a runtime, a transport or a request.

use std::io::IsTerminal;
use std::process::ExitCode;

use clap::ValueEnum;

use crate::args::{Cli, Command, ExportFormat, McpAction};
use crate::commands::inventory::Layout;
use crate::output::OutputFormat;

/// A `--format` value: each format some command prints.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum Format {
    /// A table, or lines, for people.
    Table,
    /// JSON.
    Json,
    /// CSV, one row an object (inventory only).
    Csv,
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

/// `F`'s variants as the `--format` values of the same names.
fn variants<F: ValueEnum>() -> Vec<Format> {
    F::value_variants().iter().map(as_format).collect()
}

/// `value` as the `--format` value of its name.
fn as_format<F: ValueEnum>(value: &F) -> Format {
    let name = value.to_possible_value().expect("no skipped variant");
    Format::from_str(name.get_name(), false).expect("every printed format is a --format value")
}

/// The variant of `F` that `format` names, if `F` has one.
fn pick<F: ValueEnum>(format: Format) -> Option<F> {
    F::value_variants()
        .iter()
        .find(|value| as_format(*value) == format)
        .cloned()
}

/// `format` as `--format` spells it.
fn name(format: Format) -> String {
    let value = format.to_possible_value().expect("no skipped variant");
    value.get_name().to_owned()
}

/// `a`, `a and b`, `a, b and c`.
fn listed(formats: &[Format]) -> String {
    let names: Vec<String> = formats.iter().copied().map(name).collect();
    match names.split_last() {
        Some((last, rest)) if !rest.is_empty() => format!("{} and {last}", rest.join(", ")),
        _ => names.concat(),
    }
}

/// Refuse a `--format`, or `--json`, that the command doesn't print: a
/// usage error naming the formats it does. `--format` is checked even
/// with `--json`, which outranks it.
pub(crate) fn check(cli: &Cli) -> Result<(), clap::Error> {
    let Some(command) = &cli.command else {
        return Ok(());
    };
    let printer = printer(command);
    let formats = printer.formats();
    let given = [
        cli.format
            .map(|format| (format, format!("--format {}", name(format)))),
        cli.json.then(|| (Format::Json, "--json".to_owned())),
    ];
    let Some((_, flag)) = given
        .into_iter()
        .flatten()
        .find(|(format, _)| !formats.contains(format))
    else {
        return Ok(());
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
    Err(clap::Error::raw(
        clap::error::ErrorKind::ArgumentConflict,
        format!("{command}: {flag} is not available; {why}\n"),
    ))
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
