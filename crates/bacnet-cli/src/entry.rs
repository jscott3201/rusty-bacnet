//! Plain `bacnet`, with no command (#974): it opens the terminal UI when the
//! TUI can run, and otherwise prints the usage to stderr and exits with
//! status 2, as clap does for any other missing argument.
//!
//! Opening the TUI rewrites the parsed command line into `bacnet … tui`, so
//! the global flags (transport, interface, MS/TP, profiles) take the same
//! path as they do with the subcommand, MS/TP's join confirmation included.

use std::ffi::OsStr;
use std::io::IsTerminal;

use clap::{CommandFactory, Parser};

use crate::args::Cli;

/// Environment variable that keeps plain `bacnet` out of the TUI, like
/// `--no-tui`, when set to anything but empty or `0`.
pub(crate) const NO_TUI_ENV: &str = "BACNET_NO_TUI";

/// The terminal facts the choice depends on, injectable for tests.
#[derive(Debug, Clone, Copy)]
// Without the TUI nothing reads them: there is nothing to open.
#[cfg_attr(not(feature = "tui"), allow(dead_code))]
pub(crate) struct Streams<'a> {
    /// Whether stdin is a terminal.
    pub(crate) stdin_tty: bool,
    /// Whether stdout is a terminal.
    pub(crate) stdout_tty: bool,
    /// The value of `TERM`, if set.
    pub(crate) term: Option<&'a OsStr>,
}

/// Whether plain `bacnet` opens the TUI: `Ok` if it does, else why not.
pub(crate) fn tui_allowed(
    no_tui: bool,
    no_tui_env: Option<&OsStr>,
    streams: Streams<'_>,
) -> Result<(), &'static str> {
    if !cfg!(feature = "tui") {
        return Err("this build has no terminal UI");
    }
    if no_tui {
        return Err("--no-tui was given");
    }
    if no_tui_env.is_some_and(|value| !value.is_empty() && value != "0") {
        return Err("BACNET_NO_TUI is set");
    }
    #[cfg(feature = "tui")]
    if let Some(reason) =
        crate::tui::unusable_terminal(streams.stdin_tty, streams.stdout_tty, streams.term)
    {
        return Err(reason);
    }
    #[cfg(not(feature = "tui"))]
    let _ = streams;
    Ok(())
}

/// [`tui_allowed`] for this process's streams and environment.
pub(crate) fn tui_allowed_here(no_tui: bool) -> Result<(), &'static str> {
    let term = std::env::var_os("TERM");
    let no_tui_env = std::env::var_os(NO_TUI_ENV);
    tui_allowed(
        no_tui,
        no_tui_env.as_deref(),
        Streams {
            stdin_tty: std::io::stdin().is_terminal(),
            stdout_tty: std::io::stdout().is_terminal(),
            term: term.as_deref(),
        },
    )
}

/// Give a command line without a command the `tui` subcommand at its
/// defaults, or the usage error when `allowed` says the TUI can't open.
/// A command line with a command is left as it is.
///
/// Builds clap's whole command tree: call it on the parser's thread (see
/// `main`).
pub(crate) fn resolve(cli: &mut Cli, allowed: Result<(), &'static str>) -> Result<(), clap::Error> {
    if cli.command.is_some() {
        return Ok(());
    }
    match allowed {
        Ok(()) => {
            cli.command = Cli::try_parse_from(["bacnet", "tui"])
                .expect("`bacnet tui` parses")
                .command;
            cli.command_name = Some("tui".to_owned());
            Ok(())
        }
        Err(reason) => Err(Cli::command().error(
            clap::error::ErrorKind::MissingSubcommand,
            format!(
                "give a command, such as `bacnet discover --json`; \
                 the terminal UI won't open: {reason}"
            ),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::FromArgMatches;

    const TTY: Streams<'static> = Streams {
        stdin_tty: true,
        stdout_tty: true,
        term: None,
    };

    /// Parse as `main` does, keeping the provenance the TUI reads.
    fn parse(args: &'static [&'static str]) -> Cli {
        crate::parse_on_large_stack(move || {
            let mut matches = Cli::command().try_get_matches_from(args).unwrap();
            let explicit = crate::args::capture_sources(&matches).unwrap();
            let command_name = matches.subcommand_name().map(str::to_owned);
            let mut cli = Cli::from_arg_matches_mut(&mut matches).unwrap();
            cli.explicit_args = explicit;
            cli.command_name = command_name;
            cli
        })
    }

    #[cfg(feature = "tui")]
    #[test]
    fn a_terminal_opens_the_tui_unless_opted_out() {
        assert_eq!(tui_allowed(false, None, TTY), Ok(()));
        let xterm = Streams {
            term: Some(OsStr::new("xterm-256color")),
            ..TTY
        };
        assert_eq!(tui_allowed(false, None, xterm), Ok(()));
        // An empty value or "0" doesn't opt out; anything else does.
        for value in ["", "0"] {
            assert_eq!(tui_allowed(false, Some(OsStr::new(value)), TTY), Ok(()));
        }
        for value in ["1", "yes", "true", "00"] {
            assert_eq!(
                tui_allowed(false, Some(OsStr::new(value)), TTY),
                Err("BACNET_NO_TUI is set"),
                "{value}"
            );
        }
        assert_eq!(tui_allowed(true, None, TTY), Err("--no-tui was given"));
    }

    #[cfg(feature = "tui")]
    #[test]
    fn a_pipe_or_a_dumb_terminal_gets_the_usage() {
        let piped_out = Streams {
            stdout_tty: false,
            ..TTY
        };
        let piped_in = Streams {
            stdin_tty: false,
            ..TTY
        };
        let dumb = Streams {
            term: Some(OsStr::new("dumb")),
            ..TTY
        };
        for streams in [piped_out, piped_in] {
            assert_eq!(
                tui_allowed(false, None, streams),
                Err("stdin and stdout must both be a terminal")
            );
        }
        assert_eq!(tui_allowed(false, None, dumb), Err("TERM is \"dumb\""));
    }

    #[cfg(not(feature = "tui"))]
    #[test]
    fn without_the_feature_there_is_no_tui_to_open() {
        assert_eq!(
            tui_allowed(false, None, TTY),
            Err("this build has no terminal UI")
        );
    }

    /// Plain `bacnet` and `bacnet tui` reach the TUI with the same command
    /// line, global flags and their provenance included.
    #[test]
    fn plain_bacnet_is_bacnet_tui() {
        for (plain, tui) in [
            (&["bacnet"][..], &["bacnet", "tui"][..]),
            (
                &["bacnet", "--sc", "--sc-url", "wss://hub", "-vv"],
                &["bacnet", "--sc", "--sc-url", "wss://hub", "-vv", "tui"],
            ),
            (
                &["bacnet", "-i", "10.0.0.5", "--foreign-bbmd", "10.0.0.1"],
                &[
                    "bacnet",
                    "-i",
                    "10.0.0.5",
                    "--foreign-bbmd",
                    "10.0.0.1",
                    "tui",
                ],
            ),
            (
                &["bacnet", "--mstp", "/dev/ttyUSB0", "--mstp-mac", "3"],
                &["bacnet", "--mstp", "/dev/ttyUSB0", "--mstp-mac", "3", "tui"],
            ),
        ] {
            let mut cli = parse(plain);
            let cli = crate::parse_on_large_stack(move || {
                resolve(&mut cli, Ok(())).unwrap();
                cli
            });
            assert_eq!(format!("{cli:?}"), format!("{:?}", parse(tui)), "{plain:?}");
        }
    }

    #[test]
    fn a_command_is_left_alone() {
        let mut cli = parse(&["bacnet", "--json", "discover"]);
        let before = format!("{cli:?}");
        let cli = crate::parse_on_large_stack(move || {
            resolve(&mut cli, Err("--no-tui was given")).unwrap();
            resolve(&mut cli, Ok(())).unwrap();
            cli
        });
        assert_eq!(format!("{cli:?}"), before);
    }

    #[test]
    fn no_tui_is_the_usage_error_with_status_2() {
        let mut cli = parse(&["bacnet", "--no-tui"]);
        let error = crate::parse_on_large_stack(move || {
            resolve(&mut cli, Err("--no-tui was given")).unwrap_err()
        });
        assert_eq!(error.kind(), clap::error::ErrorKind::MissingSubcommand);
        assert_eq!(error.exit_code(), 2);
        assert!(error.use_stderr());
        let text = error.render().to_string();
        assert!(
            text.contains("Usage: bacnet") && text.contains("--no-tui was given"),
            "{text}"
        );
    }
}
