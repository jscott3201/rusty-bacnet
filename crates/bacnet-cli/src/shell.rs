//! Interactive REPL shell for the BACnet CLI.
//!
//! Provides tab completion for commands, object types, and properties,
//! plus command history via rustyline.

use bacnet_client::client::BACnetClient;
use bacnet_transport::port::TransportPort;
use rustyline::completion::{Completer, Pair};
use rustyline::error::ReadlineError;
use rustyline::highlight::Highlighter;
use rustyline::hint::HistoryHinter;
use rustyline::validate::Validator;
use rustyline::{CompletionType, Config, Context, Editor, Helper};

use crate::output::OutputFormat;
use crate::{commands, output, parse, resolve};

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
    "subscribe",
    "cov",
    "control",
    "dcc",
    "reinit",
    "alarms",
    "devices",
    "file-read",
    "file-write",
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
        resolve::Target::Routed(dnet, instance) => Err(format!(
            "Routed targets ({}:{}) not yet supported in shell mode",
            dnet, instance
        )),
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
  find <name> [--wait N]             Find objects by name (WhoHas)
  read <target> <object> <property>  Read a property (e.g., read 192.168.1.10 ai:1 pv)
  readm <target> <specs...>          Read multiple properties (RPM)
  write <target> <obj> <prop> <val>  Write a property (e.g., write 10.0.1.5 av:1 pv 72.5)
  writem <target> <obj> <prop=val,...>  Write multiple properties (WPM)
  file-read <target> <instance>         Read a file (AtomicReadFile)
  file-write <target> <instance> <path> Write a file (AtomicWriteFile)
  subscribe <target> <object>        Subscribe to COV notifications
  control <target> <action>          Device communication control (enable/disable)
  reinit <target> <state>            Reinitialize device (coldstart/warmstart)
  alarms <target>                    Get event/alarm summary
  devices                            List cached discovered devices
  help                               Show this help message
  exit / quit                        Exit the shell

Aliases: whois=discover, whohas=find, rp=read, rpm=readm, wp=write, wpm=writem, cov=subscribe, dcc=control

Targets: IP address (192.168.1.10), IP:port (10.0.1.5:47809), or device instance (1234)
Objects: type:instance (ai:1, analog-input:1, binary-value:3)
Properties: name or abbreviation (present-value, pv, object-name, on, ol[3])"
    );
}

/// Run the interactive BACnet shell.
///
/// Reads commands from the user, dispatches them to the appropriate command
/// handler, and loops until the user types `exit`, `quit`, or presses Ctrl-D.
pub async fn run_shell<T: TransportPort + 'static>(
    mut client: BACnetClient<T>,
    is_sc: bool,
    format: OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::builder()
        .completion_type(CompletionType::List)
        .behavior(rustyline::Behavior::PreferTerm)
        .build();
    let helper = ShellHelper::new();
    let mut rl = Editor::with_config(config)?;
    rl.set_helper(Some(helper));

    let history_path = std::env::var("HOME")
        .map(|h| std::path::PathBuf::from(h).join(".bacnet_history"))
        .unwrap_or_else(|_| std::path::PathBuf::from(".bacnet_history"));
    let _ = rl.load_history(&history_path);

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
                    "discover" | "whois" => {
                        handle_discover(&client, args, format).await;
                    }
                    "find" | "whohas" => {
                        handle_find(&client, args, format).await;
                    }
                    "read" | "rp" => {
                        handle_read(&client, args, format).await;
                    }
                    "readm" | "rpm" => {
                        handle_readm(&client, args, format).await;
                    }
                    "write" | "wp" => {
                        handle_write(&client, args, format).await;
                    }
                    "writem" | "wpm" => {
                        handle_writem(&client, args, format).await;
                    }
                    "subscribe" | "cov" => {
                        handle_subscribe(&client, args, format).await;
                    }
                    "control" | "dcc" => {
                        handle_control(&client, args, format).await;
                    }
                    "reinit" => {
                        handle_reinit(&client, args, format).await;
                    }
                    "alarms" => {
                        handle_alarms(&client, args, format).await;
                    }
                    "devices" => {
                        if let Err(e) = commands::router::devices_cmd(&client, format).await {
                            output::print_error(&e.to_string());
                        }
                    }
                    "file-read" => {
                        handle_file_read(&client, args, format).await;
                    }
                    "file-write" => {
                        handle_file_write(&client, args, format).await;
                    }
                    _ => {
                        output::print_error(&format!(
                            "Unknown command: '{cmd}'. Type 'help' for available commands."
                        ));
                    }
                }
            }
            Err(ReadlineError::Interrupted) => {
                // Ctrl-C: cancel current line, continue loop.
                continue;
            }
            Err(ReadlineError::Eof) => {
                // Ctrl-D: exit.
                break;
            }
            Err(err) => {
                output::print_error(&format!("readline error: {err}"));
                break;
            }
        }
    }

    let _ = rl.save_history(&history_path);
    client.stop().await?;
    Ok(())
}

async fn handle_discover<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    args: &[String],
    format: OutputFormat,
) {
    let mut low = None;
    let mut high = None;
    let mut wait_secs = 3;
    let mut target: Option<String> = None;
    let mut dnet: Option<u16> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--wait" => {
                if i + 1 < args.len() {
                    match args[i + 1].parse::<u64>() {
                        Ok(w) => wait_secs = w,
                        Err(_) => {
                            output::print_error("--wait requires a numeric value");
                            return;
                        }
                    }
                    i += 2;
                    continue;
                } else {
                    output::print_error("--wait requires a value");
                    return;
                }
            }
            "--target" => {
                if i + 1 < args.len() {
                    target = Some(args[i + 1].clone());
                    i += 2;
                    continue;
                } else {
                    output::print_error("--target requires an address");
                    return;
                }
            }
            "--dnet" => {
                if i + 1 < args.len() {
                    match args[i + 1].parse::<u16>() {
                        Ok(n) => dnet = Some(n),
                        Err(_) => {
                            output::print_error("--dnet requires a network number");
                            return;
                        }
                    }
                    i += 2;
                    continue;
                } else {
                    output::print_error("--dnet requires a value");
                    return;
                }
            }
            s if s.starts_with("--") => {
                output::print_error(&format!("unknown option: '{s}'"));
                return;
            }
            _ => {
                // Try parsing as range "low-high".
                if let Some((lo, hi)) = args[i].split_once('-') {
                    match (lo.parse::<u32>(), hi.parse::<u32>()) {
                        (Ok(l), Ok(h)) => {
                            if l > h {
                                output::print_error(&format!(
                                    "invalid range: low ({l}) > high ({h})"
                                ));
                                return;
                            }
                            low = Some(l);
                            high = Some(h);
                        }
                        _ => {
                            output::print_error(&format!(
                                "invalid range: '{}', expected 'low-high'",
                                args[i]
                            ));
                            return;
                        }
                    }
                } else {
                    output::print_error(&format!(
                        "unexpected argument: '{}'. Use 'discover [low-high] [--wait N] [--target ADDR] [--dnet N]'",
                        args[i]
                    ));
                    return;
                }
            }
        }
        i += 1;
    }

    let result = if let Some(target_str) = &target {
        match resolve::parse_target(target_str) {
            Ok(resolve::Target::Mac(mac)) => {
                commands::discover::discover_directed(client, &mac, low, high, wait_secs, format)
                    .await
            }
            Ok(_) => {
                output::print_error("--target requires an IP address, not a device instance or routed address");
                return;
            }
            Err(e) => {
                output::print_error(&e);
                return;
            }
        }
    } else if let Some(network) = dnet {
        commands::discover::discover_network(client, network, low, high, wait_secs, format).await
    } else {
        commands::discover::discover(client, low, high, wait_secs, format).await
    };

    if let Err(e) = result {
        output::print_error(&e.to_string());
    }
}

async fn handle_find<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    args: &[String],
    format: OutputFormat,
) {
    let mut name = None;
    let mut wait_secs = 3;

    let mut i = 0;
    while i < args.len() {
        if args[i] == "--wait" {
            if i + 1 < args.len() {
                match args[i + 1].parse::<u64>() {
                    Ok(w) => wait_secs = w,
                    Err(_) => {
                        output::print_error("--wait requires a numeric value");
                        return;
                    }
                }
                i += 2;
                continue;
            } else {
                output::print_error("--wait requires a value");
                return;
            }
        }
        if args[i] == "--name" {
            if i + 1 < args.len() {
                name = Some(args[i + 1].clone());
                i += 2;
                continue;
            } else {
                output::print_error("--name requires a value");
                return;
            }
        }
        // Positional: treat as name if not yet set.
        if name.is_none() {
            name = Some(args[i].clone());
        }
        i += 1;
    }

    match name {
        Some(n) => {
            if let Err(e) = commands::discover::find_by_name(client, &n, wait_secs, format).await {
                output::print_error(&e.to_string());
            }
        }
        None => {
            output::print_error("Usage: find <name> [--wait N]");
        }
    }
}

async fn handle_read<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    args: &[String],
    format: OutputFormat,
) {
    if args.len() < 3 {
        output::print_error("Usage: read <target> <object> <property>");
        return;
    }

    let mac = match resolve_target_mac(client, &args[0]).await {
        Ok(m) => m,
        Err(e) => {
            output::print_error(&e);
            return;
        }
    };

    let (object_type, instance) = match parse::parse_object_specifier(&args[1]) {
        Ok(v) => v,
        Err(e) => {
            output::print_error(&e);
            return;
        }
    };

    let (property, index) = match parse::parse_property(&args[2]) {
        Ok(v) => v,
        Err(e) => {
            output::print_error(&e);
            return;
        }
    };

    if let Err(e) = commands::read::read_property_cmd(
        client,
        &mac,
        object_type,
        instance,
        property,
        index,
        format,
    )
    .await
    {
        output::print_error(&e.to_string());
    }
}

async fn handle_readm<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    args: &[String],
    format: OutputFormat,
) {
    if args.len() < 2 {
        output::print_error("Usage: readm <target> <object> <prop,...> [<object> <prop,...> ...]");
        return;
    }

    let mac = match resolve_target_mac(client, &args[0]).await {
        Ok(m) => m,
        Err(e) => {
            output::print_error(&e);
            return;
        }
    };

    let specs: Vec<String> = args[1..].to_vec();
    if let Err(e) = commands::read::read_multiple_cmd(client, &mac, &specs, format).await {
        output::print_error(&e.to_string());
    }
}

async fn handle_write<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    args: &[String],
    format: OutputFormat,
) {
    if args.len() < 4 {
        output::print_error("Usage: write <target> <object> <property> <value> [--priority N]");
        return;
    }

    let mac = match resolve_target_mac(client, &args[0]).await {
        Ok(m) => m,
        Err(e) => {
            output::print_error(&e);
            return;
        }
    };

    let (object_type, instance) = match parse::parse_object_specifier(&args[1]) {
        Ok(v) => v,
        Err(e) => {
            output::print_error(&e);
            return;
        }
    };

    let (property, index) = match parse::parse_property(&args[2]) {
        Ok(v) => v,
        Err(e) => {
            output::print_error(&e);
            return;
        }
    };

    // Parse value, which may have @priority inline.
    let (value, inline_priority) = match parse::parse_value_with_priority(&args[3]) {
        Ok(v) => v,
        Err(e) => {
            output::print_error(&e);
            return;
        }
    };

    // Check for explicit --priority flag (overrides inline @priority).
    let mut priority = inline_priority;
    let mut i = 4;
    while i < args.len() {
        if args[i] == "--priority" {
            if i + 1 < args.len() {
                match args[i + 1].parse::<u8>() {
                    Ok(p) if (1..=16).contains(&p) => priority = Some(p),
                    Ok(p) => {
                        output::print_error(&format!("priority must be 1-16, got {p}"));
                        return;
                    }
                    Err(_) => {
                        output::print_error("--priority requires a numeric value (1-16)");
                        return;
                    }
                }
                i += 2;
                continue;
            } else {
                output::print_error("--priority requires a value");
                return;
            }
        }
        i += 1;
    }

    if let Err(e) = commands::write::write_property_cmd(
        client,
        &mac,
        object_type,
        instance,
        property,
        index,
        value,
        priority,
        format,
    )
    .await
    {
        output::print_error(&e.to_string());
    }
}

async fn handle_subscribe<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    args: &[String],
    format: OutputFormat,
) {
    if args.len() < 2 {
        output::print_error("Usage: subscribe <target> <object> [--lifetime N] [--confirmed]");
        return;
    }

    let mac = match resolve_target_mac(client, &args[0]).await {
        Ok(m) => m,
        Err(e) => {
            output::print_error(&e);
            return;
        }
    };

    let (object_type, instance) = match parse::parse_object_specifier(&args[1]) {
        Ok(v) => v,
        Err(e) => {
            output::print_error(&e);
            return;
        }
    };

    let mut lifetime = None;
    let mut confirmed = false;

    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "--lifetime" => {
                if i + 1 < args.len() {
                    match args[i + 1].parse::<u32>() {
                        Ok(l) => lifetime = Some(l),
                        Err(_) => {
                            output::print_error("--lifetime requires a numeric value");
                            return;
                        }
                    }
                    i += 2;
                    continue;
                } else {
                    output::print_error("--lifetime requires a value");
                    return;
                }
            }
            "--confirmed" => confirmed = true,
            _ => {}
        }
        i += 1;
    }

    if let Err(e) = commands::subscribe::subscribe_cmd(
        client,
        &mac,
        object_type,
        instance,
        lifetime,
        confirmed,
        format,
    )
    .await
    {
        output::print_error(&e.to_string());
    }
}

async fn handle_control<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    args: &[String],
    format: OutputFormat,
) {
    if args.len() < 2 {
        output::print_error(
            "Usage: control <target> <enable|disable|disable-initiation> [--duration M] [--password P]",
        );
        return;
    }

    let mac = match resolve_target_mac(client, &args[0]).await {
        Ok(m) => m,
        Err(e) => {
            output::print_error(&e);
            return;
        }
    };

    let action = args[1].clone();
    let mut duration = None;
    let mut password = None;

    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "--duration" => {
                if i + 1 < args.len() {
                    match args[i + 1].parse::<u16>() {
                        Ok(d) => duration = Some(d),
                        Err(_) => {
                            output::print_error("--duration requires a numeric value");
                            return;
                        }
                    }
                    i += 2;
                    continue;
                } else {
                    output::print_error("--duration requires a value");
                    return;
                }
            }
            "--password" => {
                if i + 1 < args.len() {
                    password = Some(args[i + 1].clone());
                    i += 2;
                    continue;
                } else {
                    output::print_error("--password requires a value");
                    return;
                }
            }
            _ => {}
        }
        i += 1;
    }

    if let Err(e) =
        commands::device::control_cmd(client, &mac, &action, duration, password.as_deref(), format)
            .await
    {
        output::print_error(&e.to_string());
    }
}

async fn handle_reinit<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    args: &[String],
    format: OutputFormat,
) {
    if args.len() < 2 {
        output::print_error("Usage: reinit <target> <coldstart|warmstart> [--password P]");
        return;
    }

    let mac = match resolve_target_mac(client, &args[0]).await {
        Ok(m) => m,
        Err(e) => {
            output::print_error(&e);
            return;
        }
    };

    let state = args[1].clone();
    let mut password = None;

    let mut i = 2;
    while i < args.len() {
        if args[i] == "--password" {
            if i + 1 < args.len() {
                password = Some(args[i + 1].clone());
                i += 2;
                continue;
            } else {
                output::print_error("--password requires a value");
                return;
            }
        }
        i += 1;
    }

    if let Err(e) =
        commands::device::reinit_cmd(client, &mac, &state, password.as_deref(), format).await
    {
        output::print_error(&e.to_string());
    }
}

async fn handle_alarms<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    args: &[String],
    format: OutputFormat,
) {
    if args.is_empty() {
        output::print_error("Usage: alarms <target>");
        return;
    }

    let mac = match resolve_target_mac(client, &args[0]).await {
        Ok(m) => m,
        Err(e) => {
            output::print_error(&e);
            return;
        }
    };

    if let Err(e) = commands::device::alarms_cmd(client, &mac, format).await {
        output::print_error(&e.to_string());
    }
}

async fn handle_file_read<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    args: &[String],
    format: OutputFormat,
) {
    if args.len() < 2 {
        output::print_error(
            "Usage: file-read <target> <file-instance> [--start N] [--count N] [--output PATH]",
        );
        return;
    }
    let mac = match resolve_target_mac(client, &args[0]).await {
        Ok(m) => m,
        Err(e) => {
            output::print_error(&e);
            return;
        }
    };
    let file_instance = match args[1].parse::<u32>() {
        Ok(n) => n,
        Err(_) => {
            output::print_error("invalid file instance number");
            return;
        }
    };
    let mut start = 0i32;
    let mut count = 1024u32;
    let mut output_path: Option<String> = None;
    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "--start" => {
                if i + 1 < args.len() {
                    match args[i + 1].parse::<i32>() {
                        Ok(s) => start = s,
                        Err(_) => {
                            output::print_error("--start requires a numeric value");
                            return;
                        }
                    }
                    i += 2;
                    continue;
                } else {
                    output::print_error("--start requires a value");
                    return;
                }
            }
            "--count" => {
                if i + 1 < args.len() {
                    match args[i + 1].parse::<u32>() {
                        Ok(c) => count = c,
                        Err(_) => {
                            output::print_error("--count requires a numeric value");
                            return;
                        }
                    }
                    i += 2;
                    continue;
                } else {
                    output::print_error("--count requires a value");
                    return;
                }
            }
            "--output" => {
                if i + 1 < args.len() {
                    output_path = Some(args[i + 1].clone());
                    i += 2;
                    continue;
                } else {
                    output::print_error("--output requires a path");
                    return;
                }
            }
            _ => {}
        }
        i += 1;
    }
    if let Err(e) = commands::file::file_read_cmd(
        client,
        &mac,
        file_instance,
        start,
        count,
        output_path.as_deref(),
        format,
    )
    .await
    {
        output::print_error(&e.to_string());
    }
}

async fn handle_file_write<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    args: &[String],
    format: OutputFormat,
) {
    if args.len() < 3 {
        output::print_error("Usage: file-write <target> <file-instance> <input-path> [--start N]");
        return;
    }
    let mac = match resolve_target_mac(client, &args[0]).await {
        Ok(m) => m,
        Err(e) => {
            output::print_error(&e);
            return;
        }
    };
    let file_instance = match args[1].parse::<u32>() {
        Ok(n) => n,
        Err(_) => {
            output::print_error("invalid file instance number");
            return;
        }
    };
    let input_path = args[2].clone();
    let mut start = 0i32;
    let mut i = 3;
    while i < args.len() {
        if args[i] == "--start" {
            if i + 1 < args.len() {
                match args[i + 1].parse::<i32>() {
                    Ok(s) => start = s,
                    Err(_) => {
                        output::print_error("--start requires a numeric value");
                        return;
                    }
                }
                i += 2;
                continue;
            } else {
                output::print_error("--start requires a value");
                return;
            }
        }
        i += 1;
    }
    if let Err(e) =
        commands::file::file_write_cmd(client, &mac, file_instance, start, &input_path, format)
            .await
    {
        output::print_error(&e.to_string());
    }
}

async fn handle_writem<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    args: &[String],
    format: OutputFormat,
) {
    if args.len() < 3 {
        output::print_error(
            "Usage: writem <target> <object> <prop>=<value>[,<prop>=<value>] [<object> ...]",
        );
        return;
    }
    let mac = match resolve_target_mac(client, &args[0]).await {
        Ok(m) => m,
        Err(e) => {
            output::print_error(&e);
            return;
        }
    };

    // Parse specs: alternating object specifiers and prop=value lists
    let mut specs = Vec::new();
    let mut i = 1;
    while i < args.len() {
        let (obj_type, instance) = match parse::parse_object_specifier(&args[i]) {
            Ok(v) => v,
            Err(e) => {
                output::print_error(&e);
                return;
            }
        };
        i += 1;
        if i >= args.len() {
            output::print_error("expected property=value after object specifier");
            return;
        }
        // Parse comma-separated prop=value pairs
        let mut props = Vec::new();
        for pair in args[i].split(',') {
            let pair = pair.trim();
            let (prop_str, val_str) = match pair.split_once('=') {
                Some(pv) => pv,
                None => {
                    output::print_error(&format!("expected 'property=value' format, got '{pair}'"));
                    return;
                }
            };
            let (prop, idx) = match parse::parse_property(prop_str) {
                Ok(v) => v,
                Err(e) => {
                    output::print_error(&e);
                    return;
                }
            };
            let (val, priority) = match parse::parse_value_with_priority(val_str) {
                Ok(v) => v,
                Err(e) => {
                    output::print_error(&e);
                    return;
                }
            };
            props.push((prop, idx, val, priority));
        }
        specs.push((obj_type, instance, props));
        i += 1;
    }

    if let Err(e) = commands::write::write_property_multiple_cmd(client, &mac, specs, format).await
    {
        output::print_error(&e.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenize_simple() {
        let tokens = tokenize("read 192.168.1.10 ai:1 pv");
        assert_eq!(tokens, vec!["read", "192.168.1.10", "ai:1", "pv"]);
    }

    #[test]
    fn tokenize_quoted_string() {
        let tokens = tokenize("write 10.0.1.5 av:1 pv \"hello world\"");
        assert_eq!(
            tokens,
            vec!["write", "10.0.1.5", "av:1", "pv", "\"hello world\""]
        );
    }

    #[test]
    fn tokenize_empty() {
        let tokens = tokenize("");
        assert!(tokens.is_empty());
    }

    #[test]
    fn tokenize_extra_whitespace() {
        let tokens = tokenize("  read   10.0.1.5   ai:1   pv  ");
        assert_eq!(tokens, vec!["read", "10.0.1.5", "ai:1", "pv"]);
    }

    #[test]
    fn shell_helper_completions() {
        let helper = ShellHelper::new();
        assert!(!helper.commands.is_empty());
        assert!(!helper.object_types.is_empty());
        assert!(!helper.properties.is_empty());
    }
}
