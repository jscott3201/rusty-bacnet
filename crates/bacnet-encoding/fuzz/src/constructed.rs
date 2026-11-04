//! `constructed_decode`: the constructed-value codecs of
//! `bacnet_encoding::constructed`, the productions properties and service
//! parameters are built from.
//!
//! The first octet picks a codec, by its remainder over the number of them
//! ([`CODECS`] names them in that order); the octets after it are the input.
//! A codec with an encoder round trips as [`round_trip`] states. The
//! value-only readers, which have no encoder of their own, are only
//! decoded, and the tolerant event notification decoder is checked against
//! the strict one.
//!
//! [`round_trip`]: crate::round_trip

use bacnet_encoding::constructed::*;

use crate::{at, select, whole};

codecs! {
    "BACnetRecipient": at(decode_recipient) => buf_first!(encode_recipient);
    "BACnetDestination": at(decode_destination) => buf_first!(encode_destination);
    "BACnetDestination list": whole(decode_destination_list) => buf_first!(encode_destination_list);
    "BACnetPropertyAccessResult": at(decode_property_access_result)
        => buf_first!(encode_property_access_result);
    "BACnetxyColor": at(decode_xy_color) => infallible!(encode_xy_color);
    "BACnetColorCommand": at(decode_color_command) => infallible!(encode_color_command);
    "BACnetColorCommand value": whole(decode_color_command_value);
    "BACnetAuditLogRecord result": at(decode_audit_log_record_result_at)
        => value_first!(encode_audit_log_record_result);
    "BACnetAuditLogRecord": at(decode_audit_log_record_at) => value_first!(encode_audit_log_record);
    "BACnetLiftCarCallList": at(decode_lift_car_call_list) => infallible!(encode_lift_car_call_list);
    "BACnetAssignedLandingCalls": at(decode_assigned_landing_calls)
        => infallible!(encode_assigned_landing_calls);
    "BACnetCOVSubscription": at(decode_cov_subscription) => buf_first!(encode_cov_subscription);
    "BACnetCOVMultipleSubscription": at(decode_cov_multiple_subscription)
        => buf_first!(encode_cov_multiple_subscription);
    "BACnetNameValue": at(decode_name_value) => buf_first!(encode_name_value);
    "BACnetAuthenticationPolicy": at(decode_authentication_policy)
        => infallible!(encode_authentication_policy);
    "BACnetHostAddress": at(decode_host_address) => buf_first!(encode_host_address);
    "BACnetHostNPort": at(decode_host_n_port) => buf_first!(encode_host_n_port);
    "BACnetBDTEntry": at(decode_bdt_entry) => buf_first!(encode_bdt_entry);
    "BACnetBDTEntry list": whole(decode_bdt_entry_list) => buf_first!(encode_bdt_entry_list);
    "BACnetFDTEntry": at(decode_fdt_entry) => infallible!(encode_fdt_entry);
    "BACnetFDTEntry list": whole(decode_fdt_entry_list) => infallible!(encode_fdt_entry_list);
    "BACnetPortPermission": at(decode_port_permission) => infallible!(encode_port_permission);
    "BACnetActionCommand": at(decode_action_command) => buf_first!(encode_action_command);
    "BACnetActionList": at(decode_action_list) => buf_first!(encode_action_list);
    "BACnetScale": at(decode_scale) => infallible!(encode_scale);
    "BACnetPrescale": at(decode_prescale) => infallible!(encode_prescale);
    "BACnetAssignedAccessRights": at(decode_assigned_access_rights)
        => infallible!(encode_assigned_access_rights);
    "BACnetAuthenticationFactor": at(decode_authentication_factor)
        => infallible!(encode_authentication_factor);
    "BACnetAuthenticationFactorFormat": at(decode_authentication_factor_format)
        => infallible!(encode_authentication_factor_format);
    "BACnetCredentialAuthenticationFactor": at(decode_credential_authentication_factor)
        => infallible!(encode_credential_authentication_factor);
    "BACnetLandingDoorStatus": at(decode_landing_door_status)
        => infallible!(encode_landing_door_status);
    "BACnetFaultParameter": at(decode_fault_parameters) => buf_first!(encode_fault_parameters);
    "BACnetLogMultipleRecord": at(decode_log_multiple_record)
        => value_first!(encode_log_multiple_record);
    "BACnetStageLimitValue": at(decode_stage_limit_value) => infallible!(encode_stage_limit_value);
    "BACnetDeviceObjectReference": at(decode_device_object_reference)
        => infallible!(encode_device_object_reference);
    "BACnetValueSource": at(decode_value_source) => buf_first!(encode_value_source);
    "BACnetShedLevel": at(decode_shed_level) => infallible!(encode_shed_level);
    "BACnetPropertyStates": at(decode_property_state) => buf_first!(encode_property_state);
    "BACnetDeviceObjectPropertyReference": at(decode_device_object_property_reference)
        => infallible!(encode_device_object_property_reference);
    "BACnetAuditNotification": at(decode_audit_notification_at)
        => value_first!(encode_audit_notification);
    "BACnetAccessRule": at(decode_access_rule) => infallible!(encode_access_rule);
    "BACnetTimeValue": at(decode_time_value) => buf_first!(encode_time_value);
    "BACnetSpecialEvent period": at(decode_special_event_period)
        => infallible!(encode_special_event_period);
    "BACnetSpecialEvent": at(decode_special_event) => buf_first!(encode_special_event);
    "BACnetDailySchedule": at(decode_daily_schedule) => buf_first!(encode_daily_schedule);
    "Weekly_Schedule": whole(decode_weekly_schedule) => buf_first!(encode_weekly_schedule);
    "Exception_Schedule": whole(decode_exception_schedule) => buf_first!(encode_exception_schedule);
    "BACnetLightingCommand": at(decode_lighting_command) => infallible!(encode_lighting_command);
    "BACnetLightingCommand value": whole(decode_lighting_command_value);
    "BACnetLogRecord": at(decode_log_record) => value_first!(encode_log_record);
    "BACnetLandingCallStatus": at(decode_landing_call_status)
        => buf_first!(encode_landing_call_status);
    "BACnetLandingCallStatus list": whole(decode_landing_call_status_list)
        => buf_first!(encode_landing_call_status_list);
    "BACnetEventNotificationSubscription": at(decode_event_notification_subscription)
        => buf_first!(encode_event_notification_subscription);
    "BACnetObjectPropertyReference": at(decode_object_property_reference_at)
        => infallible!(encode_object_property_reference);
    "BACnetSetpointReference": at(decode_setpoint_reference_at)
        => infallible!(encode_setpoint_reference);
    "BACnetEventLogRecord": at(decode_event_log_record) => value_first!(encode_event_log_record);
    "BACnetDateRange": at(decode_date_range) => infallible!(encode_date_range);
    "BACnetCalendarEntry": at(decode_calendar_entry) => infallible!(encode_calendar_entry);
    "BACnetCalendarEntry list": whole(decode_calendar_entry_list)
        => infallible!(encode_calendar_entry_list);
    "BACnetPropertyReference": at(decode_property_reference) => infallible!(encode_property_reference);
    "ReadAccessSpecification": at(decode_read_access_specification)
        => infallible!(encode_read_access_specification);
    "BACnetEventParameter": at(decode_event_parameter) => buf_first!(encode_event_parameter);
    "BACnetPropertyValue": at(decode_bacnet_property_value)
        => value_first!(encode_bacnet_property_value);
    "BACnetNotificationParameters": |data: &[u8]| {
        decode_notification_parameters(data, 0).map(|value| (value, data.len()))
    } => value_first!(encode_notification_parameters);
    "event notification, tolerant": tolerant_agrees;
}

/// The tolerant event notification decoder against the strict one: where
/// the strict one accepts, the tolerant one reads the same request, and
/// where only the tolerant one does, it has dropped the message text.
fn tolerant_agrees(data: &[u8]) -> Result<((), usize), bacnet_types::error::Error> {
    let tolerant = decode_event_notification_tolerant(data);
    match (decode_event_notification(data), &tolerant) {
        (Ok(strict), Ok(tolerant)) => {
            assert_eq!(format!("{strict:?}"), format!("{tolerant:?}"))
        }
        (Ok(strict), Err(error)) => panic!("the tolerant decoder refused {strict:?}: {error:?}"),
        (Err(_), Ok(tolerant)) => assert!(
            tolerant.message_text.is_none(),
            "the tolerant decoder kept a text the strict one refused: {tolerant:?}"
        ),
        (Err(_), Err(_)) => {}
    }
    tolerant.map(|_| ((), data.len()))
}

/// Runs the codec the first octet picks on the rest.
pub fn check(input: &[u8]) {
    let (selector, rest) = select(input);
    run(usize::from(selector) % CODECS.len(), rest);
}

#[cfg(test)]
mod tests {
    use super::CODECS;

    /// A seed names its codec by index, so a codec inserted before the end
    /// re-points every seed after it: append new codecs instead.
    #[test]
    fn selectors_stay_put() {
        assert_eq!(CODECS.len(), 65);
        assert_eq!(CODECS[0], "BACnetRecipient");
        assert_eq!(CODECS[32], "BACnetLogMultipleRecord");
        assert_eq!(CODECS[64], "event notification, tolerant");
    }
}
