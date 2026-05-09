use super::*;

pub(super) async fn handle_writem<T: TransportPort + 'static>(
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

#[allow(dead_code)] // Superseded by handle_bip_register for session-aware registration.
pub(super) async fn handle_register(
    client: &BACnetClient<BipTransport>,
    args: &[String],
    format: OutputFormat,
) {
    if args.is_empty() {
        output::print_error("Usage: register <bbmd-address> [--ttl N]");
        return;
    }

    let mac = match resolve_target_mac(client, &args[0]).await {
        Ok(m) => m,
        Err(e) => {
            output::print_error(&e);
            return;
        }
    };

    let mut ttl: u16 = 300;
    let mut i = 1;
    while i < args.len() {
        if args[i] == "--ttl" {
            if i + 1 < args.len() {
                match args[i + 1].parse::<u16>() {
                    Ok(t) => ttl = t,
                    Err(_) => {
                        output::print_error("--ttl requires a numeric value");
                        return;
                    }
                }
                i += 2;
                continue;
            } else {
                output::print_error("--ttl requires a value");
                return;
            }
        }
        i += 1;
    }

    if let Err(e) = commands::router::register_cmd(client, &mac, ttl, format).await {
        output::print_error(&e.to_string());
    }
}
