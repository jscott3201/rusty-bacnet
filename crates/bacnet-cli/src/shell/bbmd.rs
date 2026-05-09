use super::*;

/// Handle register in BIP shell with auto-renewal via session.
pub(super) async fn handle_bip_register(
    client: &std::sync::Arc<BACnetClient<BipTransport>>,
    args: &[String],
    session: &mut Session,
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
        return;
    }

    // Set up auto-renewal in the session.
    let bbmd_display = args[0].clone();
    let renewal_mac = mac.clone();
    let renewal_client = std::sync::Arc::clone(client);
    session.set_bbmd_registration(mac, bbmd_display, ttl, move || {
        let client = std::sync::Arc::clone(&renewal_client);
        let mac = renewal_mac.clone();
        Box::pin(async move {
            client
                .register_foreign_device_bvlc(&mac, ttl)
                .await
                .map(|_| ())
                .map_err(|e| e.to_string())
        })
    });
}

pub(super) async fn handle_unregister(
    client: &BACnetClient<BipTransport>,
    args: &[String],
    format: OutputFormat,
) {
    if args.is_empty() {
        output::print_error("Usage: unregister <bbmd-address>");
        return;
    }

    let mac = match resolve_target_mac(client, &args[0]).await {
        Ok(m) => m,
        Err(e) => {
            output::print_error(&e);
            return;
        }
    };

    if let Err(e) = commands::router::unregister_cmd(client, &mac, format).await {
        output::print_error(&e.to_string());
    }
}

pub(super) async fn handle_bdt(
    client: &BACnetClient<BipTransport>,
    args: &[String],
    format: OutputFormat,
) {
    if args.is_empty() {
        output::print_error("Usage: bdt <bbmd-address>");
        return;
    }

    let mac = match resolve_target_mac(client, &args[0]).await {
        Ok(m) => m,
        Err(e) => {
            output::print_error(&e);
            return;
        }
    };

    if let Err(e) = commands::router::bdt_cmd(client, &mac, format).await {
        output::print_error(&e.to_string());
    }
}

pub(super) async fn handle_fdt(
    client: &BACnetClient<BipTransport>,
    args: &[String],
    format: OutputFormat,
) {
    if args.is_empty() {
        output::print_error("Usage: fdt <bbmd-address>");
        return;
    }

    let mac = match resolve_target_mac(client, &args[0]).await {
        Ok(m) => m,
        Err(e) => {
            output::print_error(&e);
            return;
        }
    };

    if let Err(e) = commands::router::fdt_cmd(client, &mac, format).await {
        output::print_error(&e.to_string());
    }
}

/// BIP-specific discover handler that supports --bbmd for foreign device registration.
pub(super) async fn handle_bip_discover(
    client: &std::sync::Arc<BACnetClient<BipTransport>>,
    args: &[String],
    format: OutputFormat,
) {
    let mut low = None;
    let mut high = None;
    let mut wait_secs = 3;
    let mut target: Option<String> = None;
    let mut dnet: Option<u16> = None;
    let mut bbmd: Option<String> = None;
    let mut ttl: u16 = 300;

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
            "--bbmd" => {
                if i + 1 < args.len() {
                    bbmd = Some(args[i + 1].clone());
                    i += 2;
                    continue;
                } else {
                    output::print_error("--bbmd requires an address");
                    return;
                }
            }
            "--ttl" => {
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
            s if s.starts_with("--") => {
                output::print_error(&format!("unknown option: '{s}'"));
                return;
            }
            _ => {
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
                        "unexpected argument: '{}'. Use 'discover [low-high] [--wait N] [--target ADDR] [--dnet N] [--bbmd ADDR] [--ttl N]'",
                        args[i]
                    ));
                    return;
                }
            }
        }
        i += 1;
    }

    if let Some(bbmd_addr) = &bbmd {
        let bbmd_mac = match resolve::parse_target(bbmd_addr) {
            Ok(resolve::Target::Mac(m)) => m,
            Ok(_) => {
                output::print_error("--bbmd requires an IP address, not a device instance");
                return;
            }
            Err(e) => {
                output::print_error(&e);
                return;
            }
        };
        match client.register_foreign_device_bvlc(&bbmd_mac, ttl).await {
            Ok(result) => {
                eprintln!(
                    "{}",
                    format!("Registered as foreign device with BBMD: {result:?}").green()
                );
            }
            Err(e) => {
                output::print_error(&format!("BBMD registration failed: {e}"));
                return;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }

    let result = if let Some(target_str) = &target {
        match resolve::parse_target(target_str) {
            Ok(resolve::Target::Mac(mac)) => {
                commands::discover::discover_directed(client, &mac, low, high, wait_secs, format)
                    .await
            }
            Ok(_) => {
                output::print_error(
                    "--target requires an IP address, not a device instance or routed address",
                );
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
