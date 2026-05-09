use super::*;

pub(super) async fn handle_read<T: TransportPort + 'static>(
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

pub(super) async fn handle_readm<T: TransportPort + 'static>(
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

pub(super) async fn handle_write<T: TransportPort + 'static>(
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
