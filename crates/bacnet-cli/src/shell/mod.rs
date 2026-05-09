//! Interactive REPL shell for the BACnet CLI.
//!
//! Provides tab completion for commands, object types, and properties,
//! plus command history via rustyline.

use bacnet_client::client::BACnetClient;
use bacnet_transport::bip::BipTransport;
use bacnet_transport::port::TransportPort;
use rustyline::completion::{Completer, Pair};
use rustyline::error::ReadlineError;
use rustyline::highlight::Highlighter;
use rustyline::hint::HistoryHinter;
use rustyline::validate::Validator;
use rustyline::{CompletionType, Config, Context, Editor, Helper};

use owo_colors::OwoColorize;

use crate::output::OutputFormat;
use crate::session::Session;
use crate::{commands, output, parse, resolve};

use self::admin::{
    handle_ack_alarm, handle_create_object, handle_delete_object, handle_read_range,
    handle_time_sync,
};
use self::bbmd::{
    handle_bdt, handle_bip_discover, handle_bip_register, handle_fdt, handle_unregister,
};
use self::cov_control::{handle_control, handle_reinit, handle_subscribe};
use self::discover::{handle_discover, handle_find};
use self::file::{handle_alarms, handle_file_read, handle_file_write};
use self::read_write::{handle_read, handle_readm, handle_write};
use self::writem::handle_writem;

/// All recognized shell commands.
const COMMANDS: &[&str] = &[
    "discover",
    "whois",
    "find",
    "whohas",
    "read",
    "rp",
    "readm",
    "rpm",
    "write",
    "wp",
    "writem",
    "wpm",
    "read-range",
    "rr",
    "subscribe",
    "cov",
    "control",
    "dcc",
    "reinit",
    "alarms",
    "ack-alarm",
    "ack",
    "time-sync",
    "ts",
    "create-object",
    "delete-object",
    "devices",
    "register",
    "unregister",
    "bdt",
    "fdt",
    "file-read",
    "file-write",
    "target",
    "status",
    "help",
    "exit",
    "quit",
];

/// Rustyline helper providing tab completion for the BACnet shell.
struct ShellHelper {
    commands: Vec<String>,
    object_types: Vec<String>,
    properties: Vec<String>,
}

impl ShellHelper {
    fn new() -> Self {
        Self {
            commands: COMMANDS.iter().map(|s| (*s).to_string()).collect(),
            object_types: parse::object_type_completions()
                .into_iter()
                .map(|s| s.to_string())
                .collect(),
            properties: parse::property_completions()
                .into_iter()
                .map(|s| s.to_string())
                .collect(),
        }
    }
}

impl Completer for ShellHelper {
    type Candidate = Pair;

    fn complete(
        &self,
        line: &str,
        pos: usize,
        _ctx: &Context<'_>,
    ) -> Result<(usize, Vec<Pair>), ReadlineError> {
        let line_to_cursor = &line[..pos];
        let tokens: Vec<&str> = line_to_cursor.split_whitespace().collect();

        // Determine the word being typed (may be empty if cursor is after a space).
        let (start, prefix) = if line_to_cursor.ends_with(' ') || tokens.is_empty() {
            (pos, "")
        } else {
            let word = tokens.last().unwrap();
            let start = pos - word.len();
            (start, *word)
        };

        let candidates =
            if tokens.is_empty() || (tokens.len() == 1 && !line_to_cursor.ends_with(' ')) {
                // Completing the command name.
                self.commands
                    .iter()
                    .filter(|c| c.starts_with(prefix))
                    .map(|c| Pair {
                        display: c.clone(),
                        replacement: c.clone(),
                    })
                    .collect()
            } else {
                // After the command: offer object types and properties.
                let lower_prefix = prefix.to_ascii_lowercase();
                let mut pairs: Vec<Pair> = Vec::new();
                for ot in &self.object_types {
                    let lower = ot.to_ascii_lowercase();
                    if lower.starts_with(&lower_prefix) {
                        pairs.push(Pair {
                            display: ot.clone(),
                            replacement: ot.clone(),
                        });
                    }
                }
                for p in &self.properties {
                    let lower = p.to_ascii_lowercase();
                    if lower.starts_with(&lower_prefix) {
                        pairs.push(Pair {
                            display: p.clone(),
                            replacement: p.clone(),
                        });
                    }
                }
                pairs
            };

        Ok((start, candidates))
    }
}

impl Highlighter for ShellHelper {}
impl Validator for ShellHelper {}

impl rustyline::hint::Hinter for ShellHelper {
    type Hint = String;

    fn hint(&self, line: &str, pos: usize, ctx: &Context<'_>) -> Option<Self::Hint> {
        let hinter = HistoryHinter {};
        rustyline::hint::Hinter::hint(&hinter, line, pos, ctx)
    }
}

impl Helper for ShellHelper {}

/// Tokenize a line, splitting on whitespace but preserving quoted strings.
fn tokenize(line: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;

    for ch in line.chars() {
        match ch {
            '"' => {
                in_quotes = !in_quotes;
                current.push(ch);
            }
            ' ' | '\t' if !in_quotes => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(ch),
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

/// Commands that take a target address as the first positional argument.
const TARGET_COMMANDS: &[&str] = &[
    "read",
    "rp",
    "readm",
    "rpm",
    "write",
    "wp",
    "writem",
    "wpm",
    "subscribe",
    "cov",
    "control",
    "dcc",
    "reinit",
    "alarms",
    "file-read",
    "file-write",
    "ack-alarm",
    "ack",
    "time-sync",
    "ts",
    "create-object",
    "delete-object",
    "read-range",
    "rr",
    "register",
    "unregister",
    "bdt",
    "fdt",
];

/// If the first arg is not a valid target (e.g. it's an object specifier like "ai:1"),
/// prepend the session's default target so the handler receives the expected arg order.
fn maybe_prepend_default_target(args: &[String], session: &Session) -> Vec<String> {
    if args.is_empty() {
        // No args — supply default target as the only arg if set.
        if let Some(ref display) = session.default_target_display {
            return vec![display.clone()];
        }
        return vec![];
    }
    // If the first arg is NOT a valid target, prepend the default.
    if resolve::parse_target(&args[0]).is_err() {
        if let Some(ref display) = session.default_target_display {
            let mut new_args = vec![display.clone()];
            new_args.extend_from_slice(args);
            return new_args;
        }
    }
    args.to_vec()
}

/// Handle the `target` command: show, set, or clear the default target.
fn handle_target(args: &[String], session: &mut Session) {
    if args.is_empty() {
        match &session.default_target_display {
            Some(display) => println!("Default target: {}", display.cyan()),
            None => println!(
                "{}",
                "No default target set. Use 'target <addr>' to set one.".dimmed()
            ),
        }
        return;
    }
    if args[0] == "clear" {
        session.clear_target();
        println!("{}", "Default target cleared.".green());
        return;
    }
    let target_str = &args[0];
    match resolve::parse_target(target_str) {
        Ok(_) => {
            session.set_target(vec![], target_str.clone());
            println!("Default target set to: {}", target_str.cyan());
        }
        Err(e) => {
            output::print_error(&format!("invalid target: {e}"));
        }
    }
}

/// Handle the `status` command: show session and transport state.
async fn handle_status<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    session: &Session,
    transport_name: &str,
) {
    println!("{} {}", "Transport:".dimmed(), transport_name);

    let mac = client.local_mac();
    if mac.len() == 6 {
        let ip = std::net::Ipv4Addr::new(mac[0], mac[1], mac[2], mac[3]);
        let port = u16::from_be_bytes([mac[4], mac[5]]);
        println!(
            "{} {}",
            "Local address:".dimmed(),
            format!("{ip}:{port}").cyan()
        );
    } else if mac.len() == 18 {
        let mut ip_bytes = [0u8; 16];
        ip_bytes.copy_from_slice(&mac[..16]);
        let ip = std::net::Ipv6Addr::from(ip_bytes);
        let port = u16::from_be_bytes([mac[16], mac[17]]);
        println!(
            "{} {}",
            "Local address:".dimmed(),
            format!("[{ip}]:{port}").cyan()
        );
    } else {
        println!("{} {mac:02x?}", "Local MAC:".dimmed());
    }

    match &session.default_target_display {
        Some(display) => println!("{} {}", "Default target:".dimmed(), display.cyan()),
        None => println!("{} {}", "Default target:".dimmed(), "(none)".dimmed()),
    }

    match &session.bbmd_registration {
        Some(reg) => {
            println!(
                "{} {} {}",
                "BBMD registered:".dimmed(),
                reg.bbmd_display.cyan(),
                format!("(TTL {}s, auto-renewing)", reg.ttl).dimmed()
            );
        }
        None => println!("{} {}", "BBMD registered:".dimmed(), "(none)".dimmed()),
    }

    let devices = client.discovered_devices().await;
    println!(
        "{} {}",
        "Discovered:".dimmed(),
        format!("{} device(s)", devices.len()).green()
    );
}

/// Resolve a target string to a MAC address, looking up device instances from
/// the client's discovered device table.
async fn resolve_target_mac<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    target_str: &str,
) -> Result<Vec<u8>, String> {
    match resolve::parse_target(target_str)? {
        resolve::Target::Mac(mac) => Ok(mac),
        resolve::Target::Instance(n) => match client.get_device(n).await {
            Some(d) => Ok(d.mac_address.to_vec()),
            None => Err(format!(
                "Device {} not found. Run 'discover' first or use an IP address.",
                n
            )),
        },
        resolve::Target::Routed(dnet, instance) => match client.get_device(instance).await {
            Some(d) => {
                if d.source_network == Some(dnet) {
                    // Routed targets require DNET/DADR routing information, which is
                    // not available through this MAC-only resolution helper. The shell
                    // commands that use resolve_target_mac() call client.read_property()
                    // with ConfirmedTarget::Local, so returning the router MAC here
                    // would result in an unrouted request that the router will not
                    // forward. Use the routed APIs (e.g. read_property_from_device)
                    // instead of resolve_target_mac() for routed devices.
                    Err(format!(
                        "Device {} is behind router on DNET {}. Routed access is not supported via this command; use a routed API (e.g. 'read_property_from_device') instead.",
                        instance, dnet
                    ))
                } else if d.source_network.is_none() {
                    Err(format!(
                        "Device {} is local (not behind a router on DNET {}). Use '{}' directly.",
                        instance, dnet, instance
                    ))
                } else {
                    Err(format!(
                        "Device {} is on DNET {}, not DNET {}.",
                        instance,
                        d.source_network.unwrap(),
                        dnet
                    ))
                }
            }
            None => Err(format!(
                "Device {} not found. Run 'discover' first.",
                instance
            )),
        },
    }
}

/// Print the help message listing all available shell commands.
fn print_help() {
    println!(
        "\
Commands:
  discover [low-high] [--wait N]     Discover devices (WhoIs broadcast)
    [--target ADDR]                   Send directed WhoIs to a specific address
    [--dnet N]                        Target a specific remote network number
    [--bbmd ADDR] [--ttl N]           Register as foreign device before discover (BIP only)
  find <name> [--wait N]             Find objects by name (WhoHas)
  read <target> <object> <property>  Read a property (e.g., read 192.168.1.10 ai:1 pv)
  readm <target> <specs...>          Read multiple properties (RPM)
  read-range <target> <object> [prop]  Read a range (e.g., rr 10.0.1.5 trend-log:1)
  write <target> <obj> <prop> <val>  Write a property (e.g., write 10.0.1.5 av:1 pv 72.5)
  writem <target> <obj> <prop=val,...>  Write multiple properties (WPM)
  file-read <target> <instance>         Read a file (AtomicReadFile)
  file-write <target> <instance> <path> Write a file (AtomicWriteFile)
  subscribe <target> <object>        Subscribe to COV notifications
  control <target> <action>          Device communication control (enable/disable)
  reinit <target> <state>            Reinitialize device (coldstart/warmstart)
  alarms <target>                    Get event/alarm summary
  ack-alarm <target> <obj> --state N Acknowledge an alarm
  time-sync <target> [--utc]         Synchronize time with a device
  create-object <target> <object>    Create an object on a remote device
  delete-object <target> <object>    Delete an object on a remote device
  devices                            List cached discovered devices
  register <bbmd> [--ttl N]          Register as foreign device with BBMD (BIP only)
  unregister <bbmd>                  Unregister from BBMD (BIP only)
  bdt <bbmd>                         Read BBMD broadcast distribution table (BIP only)
  fdt <bbmd>                         Read BBMD foreign device table (BIP only)
  target [<addr>|clear]              Show/set/clear default target
  status                             Show session and transport state
  help                               Show this help message
  exit / quit                        Exit the shell

Aliases: whois=discover, whohas=find, rp=read, rpm=readm, rr=read-range, wp=write, wpm=writem,
         cov=subscribe, dcc=control, ack=ack-alarm, ts=time-sync

Targets: IP address (192.168.1.10), IP:port (10.0.1.5:47809), or device instance (1234)
         When a default target is set, commands that take a target can omit it.
Objects: type:instance (ai:1, analog-input:1, binary-value:3)
Properties: name or abbreviation (present-value, pv, object-name, on, ol[3])"
    );
}

/// Dispatch a common (transport-agnostic) command.
/// Returns `true` if the command was recognized and handled.
async fn dispatch_common<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    cmd: &str,
    args: &[String],
    session: &mut Session,
    format: OutputFormat,
) -> bool {
    // For commands that take a target, try to prepend the default target
    // when the first arg doesn't look like a target address.
    let resolved_args;
    let effective_args = if TARGET_COMMANDS.contains(&cmd) {
        resolved_args = maybe_prepend_default_target(args, session);
        &resolved_args
    } else {
        args
    };

    match cmd {
        "discover" | "whois" => {
            handle_discover(client, args, format).await;
        }
        "find" | "whohas" => {
            handle_find(client, args, format).await;
        }
        "read" | "rp" => {
            handle_read(client, effective_args, format).await;
        }
        "readm" | "rpm" => {
            handle_readm(client, effective_args, format).await;
        }
        "write" | "wp" => {
            handle_write(client, effective_args, format).await;
        }
        "writem" | "wpm" => {
            handle_writem(client, effective_args, format).await;
        }
        "subscribe" | "cov" => {
            handle_subscribe(client, effective_args, format).await;
        }
        "control" | "dcc" => {
            handle_control(client, effective_args, format).await;
        }
        "reinit" => {
            handle_reinit(client, effective_args, format).await;
        }
        "alarms" => {
            handle_alarms(client, effective_args, format).await;
        }
        "devices" => {
            if let Err(e) = commands::router::devices_cmd(client, format).await {
                output::print_error(&e.to_string());
            }
        }
        "file-read" => {
            handle_file_read(client, effective_args, format).await;
        }
        "file-write" => {
            handle_file_write(client, effective_args, format).await;
        }
        "ack-alarm" | "ack" => {
            handle_ack_alarm(client, effective_args, format).await;
        }
        "time-sync" | "ts" => {
            handle_time_sync(client, effective_args, format).await;
        }
        "create-object" => {
            handle_create_object(client, effective_args, format).await;
        }
        "delete-object" => {
            handle_delete_object(client, effective_args, format).await;
        }
        "read-range" | "rr" => {
            handle_read_range(client, effective_args, format).await;
        }
        _ => return false,
    }
    true
}

/// Set up the readline editor with history and tab completion.
fn setup_readline(
) -> Result<Editor<ShellHelper, rustyline::history::DefaultHistory>, Box<dyn std::error::Error>> {
    let config = Config::builder()
        .completion_type(CompletionType::List)
        .behavior(rustyline::Behavior::PreferTerm)
        .build();
    let helper = ShellHelper::new();
    let mut rl = Editor::with_config(config)?;
    rl.set_helper(Some(helper));

    let history_path = history_path();
    let _ = rl.load_history(&history_path);
    Ok(rl)
}

fn history_path() -> std::path::PathBuf {
    std::env::var("HOME")
        .map(|h| std::path::PathBuf::from(h).join(".bacnet_history"))
        .unwrap_or_else(|_| std::path::PathBuf::from(".bacnet_history"))
}

/// Run the interactive BACnet shell (non-BIP transports: SC, BIP6).
///
/// BBMD commands are not available. For BIP transport, use `run_bip_shell`.
pub async fn run_shell<T: TransportPort + 'static>(
    mut client: BACnetClient<T>,
    is_sc: bool,
    format: OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut rl = setup_readline()?;
    let mut session = Session::new();
    let transport_name = if is_sc { "BACnet/SC" } else { "BACnet/IP" };
    let prompt = if is_sc { "bacnet[sc]> " } else { "bacnet> " };

    println!(
        "BACnet CLI v{}. Type 'help' for commands, 'exit' to quit.",
        env!("CARGO_PKG_VERSION")
    );

    loop {
        match rl.readline(prompt) {
            Ok(line) => {
                let line = line.trim().to_string();
                if line.is_empty() {
                    continue;
                }
                let _ = rl.add_history_entry(&line);

                let tokens = tokenize(&line);
                let cmd = tokens[0].to_ascii_lowercase();
                let args = &tokens[1..];

                match cmd.as_str() {
                    "exit" | "quit" => break,
                    "help" => print_help(),
                    "target" => handle_target(args, &mut session),
                    "status" => {
                        handle_status(&client, &session, transport_name).await;
                    }
                    "register" | "unregister" | "bdt" | "fdt" => {
                        output::print_error(
                            "BBMD commands are only available on BACnet/IP transport",
                        );
                    }
                    _ => {
                        if !dispatch_common(&client, &cmd, args, &mut session, format).await {
                            output::print_error(&format!(
                                "Unknown command: '{cmd}'. Type 'help' for available commands."
                            ));
                        }
                    }
                }
            }
            Err(ReadlineError::Interrupted) => continue,
            Err(ReadlineError::Eof) => break,
            Err(err) => {
                output::print_error(&format!("readline error: {err}"));
                break;
            }
        }
    }

    let _ = rl.save_history(&history_path());
    client.stop().await?;
    Ok(())
}

/// Run the interactive BACnet shell with BIP transport (supports BBMD commands).
pub async fn run_bip_shell(
    client: BACnetClient<BipTransport>,
    format: OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut rl = setup_readline()?;
    let mut session = Session::new();
    let client = std::sync::Arc::new(client);

    println!(
        "BACnet CLI v{}. Type 'help' for commands, 'exit' to quit.",
        env!("CARGO_PKG_VERSION")
    );

    loop {
        match rl.readline("bacnet> ") {
            Ok(line) => {
                let line = line.trim().to_string();
                if line.is_empty() {
                    continue;
                }
                let _ = rl.add_history_entry(&line);

                let tokens = tokenize(&line);
                let cmd = tokens[0].to_ascii_lowercase();
                let args = &tokens[1..];

                match cmd.as_str() {
                    "exit" | "quit" => break,
                    "help" => print_help(),
                    "target" => handle_target(args, &mut session),
                    "status" => {
                        handle_status(&*client, &session, "BACnet/IP").await;
                    }
                    "discover" | "whois" => {
                        handle_bip_discover(&client, args, format).await;
                    }
                    "register" => {
                        handle_bip_register(&client, args, &mut session, format).await;
                    }
                    "unregister" => {
                        let effective = maybe_prepend_default_target(args, &session);
                        handle_unregister(&client, &effective, format).await;
                        // If we just unregistered from our tracked BBMD, cancel renewal.
                        if !effective.is_empty() {
                            if let Some(ref reg) = session.bbmd_registration {
                                if reg.bbmd_display == effective[0] {
                                    session.cancel_bbmd_renewal();
                                }
                            }
                        }
                    }
                    "bdt" => {
                        let effective = maybe_prepend_default_target(args, &session);
                        handle_bdt(&client, &effective, format).await;
                    }
                    "fdt" => {
                        let effective = maybe_prepend_default_target(args, &session);
                        handle_fdt(&client, &effective, format).await;
                    }
                    _ => {
                        if !dispatch_common(&*client, &cmd, args, &mut session, format).await {
                            output::print_error(&format!(
                                "Unknown command: '{cmd}'. Type 'help' for available commands."
                            ));
                        }
                    }
                }
            }
            Err(ReadlineError::Interrupted) => continue,
            Err(ReadlineError::Eof) => break,
            Err(err) => {
                output::print_error(&format!("readline error: {err}"));
                break;
            }
        }
    }

    let _ = rl.save_history(&history_path());
    // Drop session first to release the Arc clone held by the BBMD renewal closure.
    drop(session);
    // Unwrap the Arc to call stop(). If there are outstanding references
    // (shouldn't happen in normal flow), we just log and move on.
    match std::sync::Arc::try_unwrap(client) {
        Ok(mut c) => c.stop().await?,
        Err(_) => eprintln!("Warning: could not stop client (outstanding references)"),
    }
    Ok(())
}

mod admin;
mod bbmd;
mod cov_control;
mod discover;
mod file;
mod read_write;
mod writem;

#[cfg(test)]
mod tests;
