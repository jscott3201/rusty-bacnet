//! Offline command boundary, execution diagnostics and fallible result output.

use crate::{args::Cli, output::OutputFormat};
use clap::{ArgMatches, CommandFactory, error::ErrorKind, parser::ValueSource};
use serde::Serialize;
use std::io::{self, Write};

#[cfg(feature = "sc-tls")]
pub(crate) mod lint;
#[cfg(feature = "sc-tls")]
mod projection;
#[cfg(feature = "sc-tls")]
mod render;
#[cfg(all(feature = "sc-tls", feature = "tui"))]
pub(crate) use render::date;
#[cfg(all(test, feature = "sc-tls"))]
mod tests;

/// Where this node's hub check goes past the Annex AB.7.4 default, kept on
/// purpose. Printed by sc-check and shown on the TUI's credentials view.
#[cfg(any(feature = "sc-tls", feature = "tui"))]
pub(crate) const HUB_NAME_NOTE: &str = "Hub name: this node also requires the hub's certificate to name the host in the hub URL (a DNS or IP SAN) and to permit server authentication. That is stricter than the Annex AB.7.4 default and is kept on purpose; see https://github.com/jscott3201/rusty-bacnet/blob/dev/docs/sc-check.md#hub-name-verification";

const UNASSESSED: &[&str] = &[
    "issuer_authorization",
    "certificate_path",
    "peer_name_matching",
    "revocation",
    "remote_acceptance",
];

/// Validate only this command, retaining Clap's normal usage/help behavior.
pub(super) fn validate(matches: &ArgMatches) -> Result<(), clap::Error> {
    if matches.subcommand_name() != Some("sc-check") {
        return Ok(());
    }
    if matches.get_one::<std::path::PathBuf>("sc_cert").is_none() {
        return Err(Cli::command().error(
            ErrorKind::MissingRequiredArgument,
            "sc-check requires --sc-cert <FILE>; --sc-ca and --sc-key are optional",
        ));
    }
    if let Some(format) = matches.get_one::<String>("format")
        && !matches!(format.as_str(), "table" | "json")
    {
        return Err(Cli::command().error(
            ErrorKind::InvalidValue,
            "sc-check --format must be table or json, including when --json is supplied",
        ));
    }
    for (id, flag) in [
        ("interface", "--interface"),
        ("port", "--port"),
        ("broadcast", "--broadcast"),
        ("timeout", "--timeout"),
        ("min_interval_ms", "--min-interval-ms"),
        ("foreign_bbmd", "--foreign-bbmd"),
        ("bip_multicast", "--bip-multicast"),
        ("bip_multicast_ttl", "--bip-multicast-ttl"),
        ("bip_multicast_port", "--bip-multicast-port"),
        ("ttl", "--ttl"),
        ("renewal", "--renewal"),
        ("ipv6", "--ipv6"),
        ("ipv6_interface", "--ipv6-interface"),
        ("device_instance", "--device-instance"),
        ("sc", "--sc"),
        ("sc_url", "--sc-url"),
        ("sc_failover_url", "--sc-failover-url"),
        ("sc_reconnect", "--sc-reconnect"),
        ("sc_connect_timeout", "--sc-connect-timeout"),
        ("sc_vmac", "--sc-vmac"),
        ("sc_device_uuid", "--sc-device-uuid"),
    ] {
        if matches.value_source(id) == Some(ValueSource::CommandLine) {
            return Err(Cli::command().error(
                ErrorKind::ArgumentConflict,
                format!("{flag} is a transport option and cannot be used with offline sc-check"),
            ));
        }
    }
    Ok(())
}

#[derive(Serialize)]
struct Document<T> {
    outcome: &'static str,
    summary: Option<Summary>,
    inspection: Option<T>,
    diagnostics: Vec<Diagnostic>,
    unassessed: &'static [&'static str],
    error: Option<ExecutionError>,
}

#[derive(Default, Serialize)]
struct Summary {
    errors: usize,
    warnings: usize,
    notices: usize,
}

#[derive(Serialize)]
struct Diagnostic {
    code: String,
    severity: &'static str,
    role: Option<&'static str>,
    index: Option<usize>,
    message: String,
    #[cfg(feature = "sc-tls")]
    #[serde(skip)]
    area: &'static str,
}

#[derive(Serialize)]
struct ExecutionError {
    category: &'static str,
    role: Option<&'static str>,
    index: Option<usize>,
    kind: Option<&'static str>,
    os_code: Option<i32>,
    message: &'static str,
}

fn json(document: &impl Serialize, output: &mut impl Write) -> io::Result<()> {
    serde_json::to_writer(&mut *output, document).map_err(io::Error::other)?;
    output.write_all(b"\n")?;
    output.flush()
}

fn output_failure(error: io::Error, stderr: &mut impl Write) -> u8 {
    let _ = writeln!(
        stderr,
        "Error: could not write or flush sc-check output ({})",
        error.kind()
    );
    let _ = stderr.flush();
    3
}

fn execution_error(
    error: ExecutionError,
    format: OutputFormat,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> u8 {
    if format == OutputFormat::Json {
        let document = Document::<()> {
            outcome: "not_completed",
            summary: None,
            inspection: None,
            diagnostics: Vec::new(),
            unassessed: UNASSESSED,
            error: Some(error),
        };
        if let Err(error) = json(&document, stdout) {
            return output_failure(error, stderr);
        }
    } else {
        let _ = write!(
            stderr,
            "SC inspection could not complete: {}",
            error.category
        );
        if let Some(role) = error.role {
            let _ = write!(stderr, " {role}");
        }
        if let Some(index) = error.index {
            let _ = write!(stderr, "[{index}]");
        }
        if let Some(kind) = error.kind {
            let _ = write!(stderr, " ({kind})");
        }
        if let Some(code) = error.os_code {
            let _ = write!(stderr, " (OS code {code})");
        }
        let _ = writeln!(stderr, ": {}", error.message);
        let _ = stderr.flush();
    }
    3
}

/// Execute once with explicit writers, so output/flush failures control status.
pub(super) fn execute(
    cli: &Cli,
    at_unix: Option<i64>,
    format: OutputFormat,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> u8 {
    #[cfg(feature = "sc-tls")]
    {
        use bacnet_transport::sc_tls::inspection::inspect_sc_pem_files;
        let report = match inspect_sc_pem_files(
            cli.sc_cert.as_deref().expect("validated identity path"),
            cli.sc_ca.as_deref(),
            cli.sc_key.as_deref(),
            at_unix,
        ) {
            Ok(report) => report,
            Err(error) => {
                return execution_error(projection::input_error(error), format, stdout, stderr);
            }
        };
        let document = projection::completed(&report);
        let status = u8::from(document.summary.as_ref().is_some_and(|s| s.errors != 0));
        let result = match format {
            OutputFormat::Json => json(&document, stdout),
            OutputFormat::Table => render::human(&document, stdout).and_then(|()| stdout.flush()),
        };
        match result {
            Ok(()) => status,
            Err(error) => output_failure(error, stderr),
        }
    }
    #[cfg(not(feature = "sc-tls"))]
    {
        let _ = (cli, at_unix);
        execution_error(
            ExecutionError {
                category: "feature_unavailable",
                role: None,
                index: None,
                kind: None,
                os_code: None,
                message: "Install or rebuild bacnet-cli with --features sc-tls to inspect SC files.",
            },
            format,
            stdout,
            stderr,
        )
    }
}
