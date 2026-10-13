use super::*;

pub(super) async fn handle_discover<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    args: &[String],
    format: OutputFormat,
) {
    let mut range = None;
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
                if args[i].contains('-') {
                    match crate::core::range::parse_discover_range(Some(&args[i])) {
                        Ok(parsed) => range = parsed,
                        Err(e) => {
                            output::print_error(&e.to_string());
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
        match resolve::address_mac(
            client.transport(),
            target_str,
            resolve::TARGET_NOT_AN_ADDRESS,
        )
        .await
        {
            Ok(mac) => {
                commands::discover::discover_directed(client, &mac, range, wait_secs, format).await
            }
            Err(e) => {
                output::print_error(&e);
                return;
            }
        }
    } else if let Some(network) = dnet {
        commands::discover::discover_network(client, network, range, wait_secs, format).await
    } else {
        commands::discover::discover(client, range, wait_secs, format).await
    };

    if let Err(e) = result {
        output::print_error(&e.to_string());
    }
}

pub(super) async fn handle_find<T: TransportPort + 'static>(
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
