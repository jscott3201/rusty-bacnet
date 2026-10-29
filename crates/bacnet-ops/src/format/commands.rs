//! Command, group, staging, accumulator, load control and access control
//! elements as text: a Command's action lists, Group and Global Group
//! members and results, Staging stages, Scale and Prescale, shed levels,
//! port permissions, authentication factors and policies, and access rules.

use bacnet_encoding::constructed::{
    decode_access_rule, decode_action_list, decode_authentication_factor,
    decode_authentication_factor_format, decode_authentication_policy, decode_port_permission,
    decode_prescale, decode_property_access_result, decode_read_access_specification, decode_scale,
    decode_shed_level, decode_stage_limit_value,
};
use bacnet_encoding::tags;
use bacnet_services::rpm::{ReadAccessResult, ReadPropertyMultipleACK};
use bacnet_types::constructed::{
    AccessResult, BACnetActionCommand, BACnetAuthenticationFactor, BACnetScale, BACnetShedLevel,
};
use bacnet_types::enums::{
    AccessRuleLocationSpecifier, AccessRuleTimeRangeSpecifier, AuthenticationFactorType,
    ErrorClass, ErrorCode, PropertyIdentifier,
};
use bacnet_types::error::Error;

use super::constructed::{object_reference, on_device, reference};
use super::primitive::hex;
use super::{StateTexts, ValueFormat};
use crate::label::{kebab, object_label};

/// The renderer signature of the element table (see `constructed`).
type Rendered = Result<(String, usize), Error>;

/// An error class and code: `error property:unknown-property`.
fn error_text(class: ErrorClass, code: ErrorCode) -> String {
    format!("error {}:{}", kebab(class), kebab(code))
}

/// A property and its array index: `present-value[3]`.
fn property_text(property: PropertyIdentifier, index: Option<u32>) -> String {
    match index {
        Some(index) => format!("{}[{index}]", kebab(property)),
        None => kebab(property),
    }
}

/// One write of an action list: `analog-output 1 present-value = 50 @8`,
/// with the device, delay and stop-on-failure when given.
fn action_command(format: &ValueFormat<'_>, command: &BACnetActionCommand) -> String {
    let texts = StateTexts::default();
    // The value names the target's states, so it takes the target's type.
    let target = format.for_object(command.object_identifier.object_type(), &texts);
    let mut text = format!(
        "{} {} = {}",
        object_label(command.object_identifier),
        property_text(command.property_identifier, command.property_array_index),
        target.element(command.property_identifier, &command.property_value)
    );
    if let Some(priority) = command.priority {
        text.push_str(&format!(" @{priority}"));
    }
    if let Some(device) = command.device_identifier {
        text.push_str(&format!(" on device {}", device.instance_number()));
    }
    if let Some(delay) = command.post_delay {
        text.push_str(&format!(", then wait {delay} s"));
    }
    if command.quit_on_failure {
        text.push_str(", stop on failure");
    }
    text
}

/// One Action element: its writes in order, `(… ; …)`, or `no writes`.
pub(super) fn action_list(
    format: &ValueFormat<'_>,
    _: usize,
    octets: &[u8],
    offset: usize,
) -> Rendered {
    decode_action_list(octets, offset).map(|(list, end)| {
        let text = if list.commands.is_empty() {
            "no writes".to_string()
        } else {
            let writes: Vec<String> = list
                .commands
                .iter()
                .map(|command| action_command(format, command))
                .collect();
            format!("({})", writes.join("; "))
        };
        (text, end)
    })
}

/// A Group member: `analog-input 1: present-value, status-flags`.
pub(super) fn read_access_specification(
    _: &ValueFormat<'_>,
    _: usize,
    octets: &[u8],
    offset: usize,
) -> Rendered {
    decode_read_access_specification(octets, offset).map(|(spec, end)| {
        let properties: Vec<String> = spec
            .list_of_property_references
            .iter()
            .map(|r| property_text(r.property_identifier, r.property_array_index))
            .collect();
        let text = format!(
            "{}: {}",
            object_label(spec.object_identifier),
            properties.join(", ")
        );
        (text, end)
    })
}

/// A Group's Present_Value element: `analog-input 1: present-value = 72.5,
/// units = error property:unknown-property`.
pub(super) fn read_access_result(
    format: &ValueFormat<'_>,
    _: usize,
    octets: &[u8],
    offset: usize,
) -> Rendered {
    let (result, end) = decode_read_access_result(octets, offset)?;
    let texts = StateTexts::default();
    let member = format.for_object(result.object_identifier.object_type(), &texts);
    let values: Vec<String> = result
        .list_of_results
        .iter()
        .map(|element| {
            let value = match (&element.property_value, element.error) {
                (Some(data), _) => member.value(element.property_identifier, data),
                (None, Some((class, code))) => error_text(class, code),
                (None, None) => "no value".into(),
            };
            format!(
                "{} = {value}",
                property_text(element.property_identifier, element.property_array_index)
            )
        })
        .collect();
    let text = format!(
        "{}: {}",
        object_label(result.object_identifier),
        values.join(", ")
    );
    Ok((text, end))
}

/// The Group Present_Value element at `offset`: an object identifier in
/// context tag 0, then its results inside an opening and closing tag 1.
/// The element is one result of a ReadPropertyMultiple ACK, so that ACK's
/// decoder checks it.
fn decode_read_access_result(
    octets: &[u8],
    offset: usize,
) -> Result<(ReadAccessResult, usize), Error> {
    let (tag, contents) = tags::decode_tag(octets, offset)?;
    if !tag.is_context(0) || tag.is_opening || tag.is_closing {
        return Err(Error::decoding(
            offset,
            "expected the object identifier [0]",
        ));
    }
    let after = contents
        .checked_add(tag.length as usize)
        .filter(|&after| after <= octets.len())
        .ok_or_else(|| Error::decoding(contents, "object identifier cut short"))?;
    let (tag, contents) = tags::decode_tag(octets, after)?;
    if !tag.is_opening_tag(1) {
        return Err(Error::decoding(after, "expected the results [1]"));
    }
    let (_, end) = tags::extract_context_value(octets, contents, 1)?;
    let ack = ReadPropertyMultipleACK::decode(&octets[offset..end])?;
    let [result] = <[ReadAccessResult; 1]>::try_from(ack.list_of_read_access_results)
        .map_err(|_| Error::decoding(offset, "expected one result"))?;
    Ok((result, end))
}

/// A Global Group member's result: `analog-input 1 present-value on device
/// 9 = 72.5`, or the error its read gave.
pub(super) fn property_access_result(
    format: &ValueFormat<'_>,
    _: usize,
    octets: &[u8],
    offset: usize,
) -> Rendered {
    decode_property_access_result(octets, offset).map(|(result, end)| {
        let r = &result.reference;
        let named = on_device(
            reference(
                r.object_identifier,
                r.property_identifier,
                r.property_array_index,
            ),
            r.device_identifier,
        );
        let value = match &result.access_result {
            AccessResult::Value(value) => {
                let texts = StateTexts::default();
                format
                    .for_object(r.object_identifier.object_type(), &texts)
                    .element(PropertyIdentifier::from_raw(r.property_identifier), value)
            }
            AccessResult::Error { class, code } => error_text(*class, *code),
        };
        (format!("{named} = {value}"), end)
    })
}

/// A Staging stage: `limit 50, deadband 2, targets 1010`, one digit a
/// target reference, 1 for active.
pub(super) fn stage_limit_value(
    format: &ValueFormat<'_>,
    _: usize,
    octets: &[u8],
    offset: usize,
) -> Rendered {
    decode_stage_limit_value(octets, offset).map(|(stage, end)| {
        let targets: String = stage
            .values
            .iter()
            .map(|on| if *on { '1' } else { '0' })
            .collect();
        let text = format!(
            "limit {}, deadband {}, targets {}",
            format.real(stage.limit),
            format.real(stage.deadband),
            if targets.is_empty() { "none" } else { &targets }
        );
        (text, end)
    })
}

/// An Accumulator's Scale: `factor 0.5`, or a power of ten `factor 10^-2`.
pub(super) fn scale(format: &ValueFormat<'_>, _: usize, octets: &[u8], offset: usize) -> Rendered {
    decode_scale(octets, offset).map(|(scale, end)| {
        let text = match scale {
            BACnetScale::FloatScale(factor) => format!("factor {}", format.real(factor)),
            BACnetScale::IntegerScale(power) => format!("factor 10^{power}"),
        };
        (text, end)
    })
}

/// An Accumulator's Prescale: `multiplier 1, modulo divide 10`.
pub(super) fn prescale(_: &ValueFormat<'_>, _: usize, octets: &[u8], offset: usize) -> Rendered {
    decode_prescale(octets, offset).map(|(prescale, end)| {
        let text = format!(
            "multiplier {}, modulo divide {}",
            prescale.multiplier, prescale.modulo_divide
        );
        (text, end)
    })
}

/// A Load Control shed level: `80 %`, `level 2` or `reduce 1.5 kW`.
pub(super) fn shed_level(
    format: &ValueFormat<'_>,
    _: usize,
    octets: &[u8],
    offset: usize,
) -> Rendered {
    decode_shed_level(octets, offset).map(|(level, end)| {
        let text = match level {
            BACnetShedLevel::Percent(percent) => format!("{percent} %"),
            BACnetShedLevel::Level(level) => format!("level {level}"),
            BACnetShedLevel::Amount(kilowatts) => format!("reduce {} kW", format.real(kilowatts)),
        };
        (text, end)
    })
}

/// A Port_Filter element: `port 1 enabled`.
pub(super) fn port_permission(
    _: &ValueFormat<'_>,
    _: usize,
    octets: &[u8],
    offset: usize,
) -> Rendered {
    decode_port_permission(octets, offset).map(|(permission, end)| {
        let state = if permission.enabled {
            "enabled"
        } else {
            "disabled"
        };
        (format!("port {} {state}", permission.port_id), end)
    })
}

/// An authentication factor: `simple-number16 class 0 value 0x04d2`.
pub(super) fn factor_text(factor: &BACnetAuthenticationFactor) -> String {
    format!(
        "{} class {} value 0x{}",
        kebab(factor.format_type),
        factor.format_class,
        hex(&factor.value, "")
    )
}

/// An authentication factor: `simple-number16 class 0 value 0x04d2`.
pub(super) fn authentication_factor(
    _: &ValueFormat<'_>,
    _: usize,
    octets: &[u8],
    offset: usize,
) -> Rendered {
    decode_authentication_factor(octets, offset).map(|(factor, end)| (factor_text(&factor), end))
}

/// A Supported_Formats element: the format, and for a custom one its
/// vendor and number, `custom (vendor 7 format 2)`.
pub(super) fn authentication_factor_format(
    _: &ValueFormat<'_>,
    _: usize,
    octets: &[u8],
    offset: usize,
) -> Rendered {
    decode_authentication_factor_format(octets, offset).map(|(format, end)| {
        let mut text = kebab(format.format_type);
        let vendor = format.vendor_id.unwrap_or(0);
        let number = format.vendor_format.unwrap_or(0);
        if format.format_type == AuthenticationFactorType::CUSTOM || vendor != 0 || number != 0 {
            text.push_str(&format!(" (vendor {vendor} format {number})"));
        }
        (text, end)
    })
}

/// An Authentication_Policy_List element: the factors with their steps,
/// whether their order is enforced, and the timeout,
/// `credential-data-input 1 step 1, credential-data-input 2 step 2, in
/// order, timeout 30 s`.
pub(super) fn authentication_policy(
    _: &ValueFormat<'_>,
    _: usize,
    octets: &[u8],
    offset: usize,
) -> Rendered {
    decode_authentication_policy(octets, offset).map(|(policy, end)| {
        let mut parts: Vec<String> = policy
            .policy
            .iter()
            .map(|entry| {
                format!(
                    "{} step {}",
                    object_reference(&entry.credential_data_input),
                    entry.index
                )
            })
            .collect();
        if parts.is_empty() {
            parts.push("no factors".into());
        }
        parts.push(if policy.order_enforced {
            "in order".into()
        } else {
            "any order".into()
        });
        parts.push(match policy.timeout {
            0 => "no timeout".into(),
            seconds => format!("timeout {seconds} s"),
        });
        (parts.join(", "), end)
    })
}

/// An access rule: when and where it holds, and whether it is in force,
/// `always at access-point 1, enabled`.
pub(super) fn access_rule(_: &ValueFormat<'_>, _: usize, octets: &[u8], offset: usize) -> Rendered {
    decode_access_rule(octets, offset).map(|(rule, end)| {
        let when = match (&rule.time_range, rule.time_range_specifier) {
            (_, AccessRuleTimeRangeSpecifier::ALWAYS) => "always".to_string(),
            (Some(r), _) => format!(
                "when {}",
                on_device(
                    reference(
                        r.object_identifier,
                        r.property_identifier,
                        r.property_array_index
                    ),
                    r.device_identifier
                )
            ),
            (None, specifier) => format!("time range {}", kebab(specifier)),
        };
        let place = match (&rule.location, rule.location_specifier) {
            (_, AccessRuleLocationSpecifier::ALL) => "every point".to_string(),
            (Some(location), _) => object_reference(location),
            (None, specifier) => format!("location {}", kebab(specifier)),
        };
        let state = if rule.enable { "enabled" } else { "disabled" };
        (format!("{when} at {place}, {state}"), end)
    })
}
