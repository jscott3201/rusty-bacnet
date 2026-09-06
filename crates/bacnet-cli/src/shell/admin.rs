use super::*;

use bacnet_types::primitives::BACnetTimeStamp;

const ACK_ALARM_USAGE: &str = "Usage: ack-alarm <target> <object> --state N \
--timestamp <SPEC> --ack-time <SPEC> [--source S]";

#[derive(Debug)]
pub(super) struct AckAlarmArguments<'a> {
    pub(super) target: &'a str,
    pub(super) object: &'a str,
    pub(super) state: u32,
    pub(super) source: String,
    pub(super) timestamp: BACnetTimeStamp,
    pub(super) time_of_acknowledgment: BACnetTimeStamp,
}

pub(super) fn parse_ack_alarm_arguments(args: &[String]) -> Result<AckAlarmArguments<'_>, String> {
    if args.len() < 2 {
        return Err(ACK_ALARM_USAGE.into());
    }

    let mut state = None;
    let mut source = "bacnet-cli".to_string();
    let mut timestamp = None;
    let mut time_of_acknowledgment = None;
    let mut i = 2;
    while i < args.len() {
        let flag = args[i].as_str();
        let value = args
            .get(i + 1)
            .ok_or_else(|| format!("{flag} requires a value"))?;
        match flag {
            "--state" => {
                if state.is_some() {
                    return Err("--state may only be specified once".into());
                }
                state = Some(
                    value
                        .parse::<u32>()
                        .map_err(|_| "--state requires a numeric value".to_string())?,
                );
            }
            "--source" => source = value.clone(),
            "--timestamp" => {
                if timestamp.is_some() {
                    return Err("--timestamp may only be specified once".into());
                }
                timestamp = Some(timestamp::parse_bacnet_timestamp(value)?);
            }
            "--ack-time" => {
                if time_of_acknowledgment.is_some() {
                    return Err("--ack-time may only be specified once".into());
                }
                time_of_acknowledgment = Some(timestamp::parse_bacnet_timestamp(value)?);
            }
            _ => {
                return Err(format!(
                    "unknown ack-alarm option '{flag}'; {ACK_ALARM_USAGE}"
                ))
            }
        }
        i += 2;
    }

    Ok(AckAlarmArguments {
        target: &args[0],
        object: &args[1],
        state: state.ok_or_else(|| "--state is required".to_string())?,
        source,
        timestamp: timestamp.ok_or_else(|| "--timestamp is required".to_string())?,
        time_of_acknowledgment: time_of_acknowledgment
            .ok_or_else(|| "--ack-time is required".to_string())?,
    })
}

pub(super) async fn handle_ack_alarm<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    args: &[String],
    format: OutputFormat,
) {
    let arguments = match parse_ack_alarm_arguments(args) {
        Ok(arguments) => arguments,
        Err(e) => {
            output::print_error(&e);
            return;
        }
    };

    let (object_type, instance) = match parse::parse_object_specifier(arguments.object) {
        Ok(v) => v,
        Err(e) => {
            output::print_error(&e);
            return;
        }
    };

    let mac = match resolve_target_mac(client, arguments.target).await {
        Ok(m) => m,
        Err(e) => {
            output::print_error(&e);
            return;
        }
    };

    if let Err(e) = commands::device::acknowledge_alarm_cmd(
        client,
        &mac,
        object_type,
        instance,
        arguments.state,
        &arguments.source,
        arguments.timestamp,
        arguments.time_of_acknowledgment,
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
