//! `main.rs`'s tests: offline dispatch, the write guard and target
//! resolution.

use std::net::Ipv6Addr;

use bacnet_transport::bip6::Bip6Transport;

use super::*;

#[test]
fn sc_check_never_invokes_runtime_or_network_initialization() {
    use clap::Parser;
    let cli = parse_on_large_stack(|| {
        Cli::parse_from([
            "bacnet",
            "sc-check",
            "--sc-cert",
            "missing-offline-input",
            "--json",
        ])
    });
    let result = dispatch(cli, &mut guard::Stdin, |_| {
        panic!("offline command entered network initialization")
    });
    assert_eq!(result.unwrap(), std::process::ExitCode::from(3));
}

/// Stdin that is not a terminal, and must not be read.
struct Piped;

impl guard::Confirm for Piped {
    fn interactive(&self) -> bool {
        false
    }
    fn ask(&mut self, _: &str) -> Option<String> {
        panic!("a piped stdin was prompted")
    }
}

fn one_shot(args: &'static [&'static str]) -> Cli {
    parse_on_large_stack(move || {
        let mut matches = Cli::command().try_get_matches_from(args).unwrap();
        let command_name = matches.subcommand_name().map(str::to_owned);
        let mut cli = Cli::from_arg_matches_mut(&mut matches).unwrap();
        cli.command_name = command_name;
        cli
    })
}

/// The exit status, and whether the network path ran.
fn run_guarded(args: &'static [&'static str]) -> (std::process::ExitCode, bool) {
    let mut ran = false;
    let status = dispatch(one_shot(args), &mut Piped, |_| {
        ran = true;
        Ok(())
    })
    .unwrap();
    (status, ran)
}

/// A refused one-shot never reaches runtime, tracing or transport
/// setup, so nothing can go on the wire (#972).
#[test]
fn a_refused_one_shot_never_reaches_the_network() {
    let refused = std::process::ExitCode::from(guard::EXIT_REFUSED);
    for args in [
        &["bacnet", "--json", "write", "10.0.0.1", "av:1", "pv", "1"][..],
        &["bacnet", "--json", "time-sync", "10.0.0.1"],
        &["bacnet", "--json", "writem", "10.0.0.1", "av:1", "pv=1"],
        &[
            "bacnet",
            "--json",
            "--allow-write",
            "reinit",
            "10.0.0.1",
            "warmstart",
        ],
        &["bacnet", "--json", "--yes", "control", "10.0.0.1", "enable"],
    ] {
        assert_eq!(run_guarded(args), (refused, false), "{args:?}");
    }
    for args in [
        &[
            "bacnet",
            "write",
            "10.0.0.1",
            "av:1",
            "pv",
            "1",
            "--allow-write",
        ][..],
        &[
            "bacnet",
            "--allow-write",
            "--yes",
            "reinit",
            "10.0.0.1",
            "warmstart",
        ],
        &["bacnet", "read", "10.0.0.1", "av:1", "pv"],
        &["bacnet", "--allow-write", "wpm", "10.0.0.1", "av:1", "pv=1"],
        &["bacnet", "tui"],
    ] {
        assert_eq!(
            run_guarded(args),
            (std::process::ExitCode::SUCCESS, true),
            "{args:?}"
        );
    }
}

/// A format the command doesn't print is a usage error (status 2) before
/// the write guard, `sc-check`, `mstp-ports` or the network path (#2778);
/// one it prints goes on as before.
#[test]
fn a_refused_format_never_reaches_the_network() {
    let usage = std::process::ExitCode::from(2);
    for args in [
        &[
            "bacnet", "read", "10.0.0.1", "av:1", "pv", "--format", "csv",
        ][..],
        &[
            "bacnet", "--format", "csv", "write", "10.0.0.1", "av:1", "pv", "1",
        ],
        &["bacnet", "--json", "export", "223-experimental", "10.0.0.1"],
        &["bacnet", "--json", "tui"],
        &[
            "bacnet",
            "mcp",
            "serve",
            "--stdio",
            "--passive",
            "--format",
            "json",
        ],
        &[
            "bacnet",
            "sc-check",
            "--sc-cert",
            "missing",
            "--format",
            "csv",
        ],
        &["bacnet", "mstp-ports", "--format", "csv"],
    ] {
        assert_eq!(run_guarded(args), (usage, false), "{args:?}");
    }
    for args in [
        &["bacnet", "inventory", "10.0.0.1", "--format", "csv"][..],
        &[
            "bacnet", "--format", "table", "read", "10.0.0.1", "av:1", "pv",
        ],
        &["bacnet", "export", "223-experimental", "10.0.0.1"],
    ] {
        assert_eq!(
            run_guarded(args),
            (std::process::ExitCode::SUCCESS, true),
            "{args:?}"
        );
    }
}

#[cfg(unix)]
#[test]
fn sc_check_parser_preserves_non_utf8_credential_bytes() {
    use clap::Parser;
    use std::os::unix::ffi::{OsStrExt, OsStringExt};
    let cli = parse_on_large_stack(|| {
        let mut arguments = ["bacnet", "sc-check", "--sc-cert"]
            .map(std::ffi::OsString::from)
            .to_vec();
        arguments.push(std::ffi::OsString::from_vec(b"identity-\xff.pem".to_vec()));
        Cli::parse_from(arguments)
    });
    assert_eq!(
        cli.sc_cert.unwrap().as_os_str().as_bytes(),
        b"identity-\xff.pem"
    );
}

/// A one-shot command's `[ip]:port` target is the VMAC of the node
/// there, found with a Virtual-Address-Resolution before the command
/// runs (Clause U.5); `bip6:1234` names the same VMAC directly.
#[tokio::test]
async fn a_bip6_address_target_resolves_to_the_vmac_there() {
    let mut server = Bip6Transport::new(Ipv6Addr::LOCALHOST, 0, Some(1234));
    let _server_rx = server.start().await.unwrap();
    let port = server.local_address().unwrap().port();
    let mut client = BACnetClient::bip6_builder()
        .interface(Ipv6Addr::LOCALHOST)
        .port(0)
        // An instance VMAC: a random one needs a multicast probe at start,
        // which Linux loopback can't carry.
        .device_instance(4321)
        .build()
        .await
        .unwrap();
    let target = format!("[::1]:{port}");
    let mac = resolve_target_mac(&client, &target).await.unwrap();
    assert_eq!(mac, [0x00, 0x04, 0xD2]);
    let mac = resolve_target_mac(&client, "bip6:1234").await.unwrap();
    assert_eq!(mac, [0x00, 0x04, 0xD2]);
    client.stop().await.unwrap();
    server.stop().await.unwrap();
}

/// An error whose message leaves out its source, as a library's might.
#[derive(Debug)]
struct Outer(std::io::Error);

impl std::fmt::Display for Outer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("could not open the store")
    }
}

impl std::error::Error for Outer {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.0)
    }
}

fn reported(error: &(dyn std::error::Error + 'static)) -> String {
    let mut out = Vec::new();
    report(error, &mut out).unwrap();
    String::from_utf8(out).unwrap()
}

/// A fatal error prints with Display, not Debug's quotes and escapes
/// (#2325); a source its message drops follows after `: `, and one it
/// already holds isn't repeated.
#[test]
fn a_fatal_error_reports_with_display_and_its_sources() {
    let plain: Box<dyn std::error::Error> = r#"no profile in C:\cfg: run "x""#.into();
    assert_eq!(
        reported(&*plain),
        "Error: no profile in C:\\cfg: run \"x\"\n"
    );
    let dropped = Outer(std::io::Error::other("denied"));
    assert_eq!(
        reported(&dropped),
        "Error: could not open the store: denied\n"
    );
    let held = bacnet_types::error::Error::Transport(std::io::Error::other("denied"));
    assert_eq!(reported(&held), "Error: transport error: denied\n");
}

/// A broadcast refused on a loopback interface names the CLI's own option,
/// whether it arrives as the client's error or inside bacnet-ops' (#2691).
#[test]
fn a_refused_loopback_broadcast_names_the_broadcast_option() {
    let refused = || {
        let io = std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            bacnet_transport::bip::LoopbackBroadcastRefused::default(),
        );
        bacnet_types::error::Error::Transport(io)
    };
    let hint = "127.255.255.255 (the -b/--broadcast option)\n";
    assert!(reported(&refused()).ends_with(hint));
    let ops = bacnet_ops::OpsError::Client(refused());
    assert!(reported(&ops).ends_with(hint), "{}", reported(&ops));
    let other = bacnet_types::error::Error::Transport(std::io::Error::other("denied"));
    assert!(!reported(&other).contains("--broadcast"));
}
