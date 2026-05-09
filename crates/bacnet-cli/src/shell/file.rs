use super::*;

pub(super) async fn handle_alarms<T: TransportPort + 'static>(
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

pub(super) async fn handle_file_read<T: TransportPort + 'static>(
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

pub(super) async fn handle_file_write<T: TransportPort + 'static>(
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
