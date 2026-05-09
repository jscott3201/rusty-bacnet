use super::*;

pub(super) async fn handle_subscribe<T: TransportPort + 'static>(
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

pub(super) async fn handle_control<T: TransportPort + 'static>(
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

pub(super) async fn handle_reinit<T: TransportPort + 'static>(
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
