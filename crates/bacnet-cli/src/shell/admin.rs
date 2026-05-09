use super::*;

pub(super) async fn handle_ack_alarm<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    args: &[String],
    format: OutputFormat,
) {
    if args.len() < 2 {
        output::print_error("Usage: ack-alarm <target> <object> --state N [--source S]");
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

    let mut state: Option<u32> = None;
    let mut source = "bacnet-cli".to_string();

    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "--state" => {
                if i + 1 < args.len() {
                    match args[i + 1].parse::<u32>() {
                        Ok(s) => state = Some(s),
                        Err(_) => {
                            output::print_error("--state requires a numeric value");
                            return;
                        }
                    }
                    i += 2;
                    continue;
                } else {
                    output::print_error("--state requires a value");
                    return;
                }
            }
            "--source" => {
                if i + 1 < args.len() {
                    source = args[i + 1].clone();
                    i += 2;
                    continue;
                } else {
                    output::print_error("--source requires a value");
                    return;
                }
            }
            _ => {}
        }
        i += 1;
    }

    let state = match state {
        Some(s) => s,
        None => {
            output::print_error("--state is required");
            return;
        }
    };

    if let Err(e) = commands::device::acknowledge_alarm_cmd(
        client,
        &mac,
        object_type,
        instance,
        state,
        &source,
        format,
    )
    .await
    {
        output::print_error(&e.to_string());
    }
}

pub(super) async fn handle_time_sync<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    args: &[String],
    format: OutputFormat,
) {
    if args.is_empty() {
        output::print_error("Usage: time-sync <target> [--utc]");
        return;
    }

    let mac = match resolve_target_mac(client, &args[0]).await {
        Ok(m) => m,
        Err(e) => {
            output::print_error(&e);
            return;
        }
    };

    let utc = args[1..].iter().any(|a| a == "--utc");

    if let Err(e) = commands::device::time_sync_cmd(client, &mac, utc, format).await {
        output::print_error(&e.to_string());
    }
}

pub(super) async fn handle_create_object<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    args: &[String],
    format: OutputFormat,
) {
    if args.len() < 2 {
        output::print_error("Usage: create-object <target> <object>");
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

    if let Err(e) =
        commands::device::create_object_cmd(client, &mac, object_type, instance, format).await
    {
        output::print_error(&e.to_string());
    }
}

pub(super) async fn handle_delete_object<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    args: &[String],
    format: OutputFormat,
) {
    if args.len() < 2 {
        output::print_error("Usage: delete-object <target> <object>");
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

    if let Err(e) =
        commands::device::delete_object_cmd(client, &mac, object_type, instance, format).await
    {
        output::print_error(&e.to_string());
    }
}

pub(super) async fn handle_read_range<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    args: &[String],
    format: OutputFormat,
) {
    if args.len() < 2 {
        output::print_error("Usage: read-range <target> <object> [property]");
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

    let prop_str = if args.len() > 2 {
        &args[2]
    } else {
        "log-buffer"
    };
    let (property, index) = match parse::parse_property(prop_str) {
        Ok(v) => v,
        Err(e) => {
            output::print_error(&e);
            return;
        }
    };

    if let Err(e) =
        commands::read::read_range_cmd(client, &mac, object_type, instance, property, index, format)
            .await
    {
        output::print_error(&e.to_string());
    }
}
