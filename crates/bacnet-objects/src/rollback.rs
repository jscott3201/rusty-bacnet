//! Compatibility object snapshots retained for direct object-local use.
//!
//! The bundled server's Service 16 path no longer invokes these helpers;
//! WritePropertyMultiple retains successful prefix writes.

use bacnet_types::enums::EventState;
use bacnet_types::primitives::BACnetTimeStamp;

use crate::event::PendingTransition;

pub(crate) enum IntrinsicWriteRollback {
    EventDetection {
        enabled: bool,
        event_state: EventState,
        acked_transitions: u8,
        pending: Option<PendingTransition>,
        fault_reliability: Option<u32>,
        time_stamps: [BACnetTimeStamp; 3],
        original_to_states: [Option<EventState>; 3],
        message_texts: [String; 3],
    },
    TimeDelayNormal(Option<u32>),
    ReliabilityInhibit {
        state: crate::common::ReliabilityInhibitState,
        reliability: u32,
        out_of_service: bool,
        saved_reliability: Option<u32>,
        range_fault_ownership: Option<Option<crate::analog::OwnedRangeFault>>,
        multistate_fault_ownership: Option<Option<crate::multistate::OwnedMultiStateFault>>,
    },
    MultiStateCommand {
        priority_array: [Option<u32>; 16],
        relinquish_default: u32,
        present_value: u32,
        reliability: u32,
        fault_ownership: Option<crate::multistate::OwnedMultiStateFault>,
    },
}

macro_rules! capture_multistate_command_rollback {
    (
        $object:ident,
        $reliability_field:ident;
        $fault_field:ident,
        $priority_array_field:ident,
        $relinquish_default_field:ident,
        $present_value_field:ident
    ) => {
        Some($crate::traits::WritePropertyRollback::new(
            $crate::rollback::IntrinsicWriteRollback::MultiStateCommand {
                priority_array: $object.$priority_array_field,
                relinquish_default: $object.$relinquish_default_field,
                present_value: $object.$present_value_field,
                reliability: $object.$reliability_field,
                fault_ownership: $object.$fault_field.owned_fault,
            },
        ))
    };
    ($object:ident, $reliability_field:ident $(; $fault_field:ident)?) => {
        None
    };
}

macro_rules! restore_multistate_command_rollback {
    (
        $object:ident,
        $priority_array:ident,
        $relinquish_default:ident,
        $present_value:ident,
        $reliability:ident,
        $fault_ownership:ident,
        $reliability_field:ident;
        $fault_field:ident,
        $priority_array_field:ident,
        $relinquish_default_field:ident,
        $present_value_field:ident
    ) => {{
        $object.$priority_array_field = $priority_array;
        $object.$relinquish_default_field = $relinquish_default;
        $object.$present_value_field = $present_value;
        $object.$reliability_field = $reliability;
        $object.$fault_field.owned_fault = $fault_ownership;
        Ok(())
    }};
    (
        $object:ident,
        $priority_array:ident,
        $relinquish_default:ident,
        $present_value:ident,
        $reliability:ident,
        $fault_ownership:ident,
        $reliability_field:ident
        $(; $fault_field:ident)?
    ) => {{
        let _ = (
            $priority_array,
            $relinquish_default,
            $present_value,
            $reliability,
            $fault_ownership,
        );
        Err(bacnet_types::error::Error::Encoding(
            "object received an incompatible multi-state command rollback token".into(),
        ))
    }};
}

/// Preserve intrinsic event/inhibit state hidden by property readback and,
/// when configured, exact retained multi-state command configuration.
macro_rules! impl_intrinsic_write_rollback {
    (
        $detector_field:ident,
        $detection_enable_field:ident,
        $history_field:ident,
        $inhibit_field:ident,
        $reliability_field:ident,
        $out_of_service_field:ident,
        $saved_reliability_field:ident
        $(, $range_fault_field:ident)?
        $(; $multistate_fault_field:ident
            $(, $priority_array_field:ident,
                $relinquish_default_field:ident,
                $present_value_field:ident)?
        )?
    ) => {
        fn capture_write_property_rollback(
            &mut self,
            property: bacnet_types::enums::PropertyIdentifier,
            _value: &bacnet_types::primitives::PropertyValue,
        ) -> Option<$crate::traits::WritePropertyRollback> {
            match property {
                bacnet_types::enums::PropertyIdentifier::EVENT_DETECTION_ENABLE => {
                    Some($crate::traits::WritePropertyRollback::new(
                        $crate::rollback::IntrinsicWriteRollback::EventDetection {
                            enabled: self.$detection_enable_field,
                            event_state: self.$detector_field.event_state,
                            acked_transitions: self.$detector_field.acked_transitions,
                            pending: self.$detector_field.pending.clone(),
                            fault_reliability: self.$detector_field.fault_reliability,
                            time_stamps: self.$history_field.time_stamps.clone(),
                            original_to_states: self.$history_field.original_to_states,
                            message_texts: self.$history_field.message_texts.clone(),
                        },
                    ))
                }
                bacnet_types::enums::PropertyIdentifier::TIME_DELAY_NORMAL => {
                    Some($crate::traits::WritePropertyRollback::new(
                        $crate::rollback::IntrinsicWriteRollback::TimeDelayNormal(
                            self.$detector_field.time_delay_normal,
                        ),
                    ))
                }
                bacnet_types::enums::PropertyIdentifier::RELIABILITY_EVALUATION_INHIBIT
                | bacnet_types::enums::PropertyIdentifier::RELIABILITY
                | bacnet_types::enums::PropertyIdentifier::OUT_OF_SERVICE => {
                    let range_fault_ownership = None$(.or(Some(
                        self.$range_fault_field.owned_fault,
                    )))?;
                    let multistate_fault_ownership = None$(.or(Some(
                        self.$multistate_fault_field.owned_fault,
                    )))?;
                    Some($crate::traits::WritePropertyRollback::new(
                        $crate::rollback::IntrinsicWriteRollback::ReliabilityInhibit {
                            state: self.$inhibit_field,
                            reliability: self.$reliability_field,
                            out_of_service: self.$out_of_service_field,
                            saved_reliability: self.$saved_reliability_field,
                            range_fault_ownership,
                            multistate_fault_ownership,
                        },
                    ))
                }
                bacnet_types::enums::PropertyIdentifier::PRIORITY_ARRAY
                | bacnet_types::enums::PropertyIdentifier::RELINQUISH_DEFAULT => {
                    $crate::rollback::capture_multistate_command_rollback!(
                        self,
                        $reliability_field
                        $(; $multistate_fault_field
                            $(, $priority_array_field,
                                $relinquish_default_field,
                                $present_value_field)?
                        )?
                    )
                }
                _ => None,
            }
        }

        fn restore_write_property_rollback(
            &mut self,
            rollback: $crate::traits::WritePropertyRollback,
        ) -> Result<(), bacnet_types::error::Error> {
            match rollback.downcast::<$crate::rollback::IntrinsicWriteRollback>()? {
                $crate::rollback::IntrinsicWriteRollback::EventDetection {
                    enabled,
                    event_state,
                    acked_transitions,
                    pending,
                    fault_reliability,
                    time_stamps,
                    original_to_states,
                    message_texts,
                } => {
                    self.$detection_enable_field = enabled;
                    self.$detector_field.event_state = event_state;
                    self.$detector_field.acked_transitions = acked_transitions;
                    self.$detector_field.pending = pending;
                    self.$detector_field.fault_reliability = fault_reliability;
                    self.$history_field.time_stamps = time_stamps;
                    self.$history_field.original_to_states = original_to_states;
                    self.$history_field.message_texts = message_texts;
                    Ok(())
                }
                $crate::rollback::IntrinsicWriteRollback::TimeDelayNormal(value) => {
                    self.$detector_field.time_delay_normal = value;
                    Ok(())
                }
                $crate::rollback::IntrinsicWriteRollback::ReliabilityInhibit {
                    state,
                    reliability,
                    out_of_service,
                    saved_reliability,
                    range_fault_ownership,
                    multistate_fault_ownership,
                } => {
                    $(
                        let range_fault_ownership = range_fault_ownership.ok_or_else(|| {
                            bacnet_types::error::Error::Encoding(
                                concat!(
                                    "analog rollback token omitted ",
                                    stringify!($range_fault_field),
                                    " ownership",
                                )
                                .into(),
                            )
                        })?;
                    )?
                    $(
                        let multistate_fault_ownership = multistate_fault_ownership.ok_or_else(|| {
                            bacnet_types::error::Error::Encoding(
                                concat!(
                                    "multi-state rollback token omitted ",
                                    stringify!($multistate_fault_field),
                                    " ownership",
                                )
                                .into(),
                            )
                        })?;
                    )?
                    self.$inhibit_field = state;
                    self.$reliability_field = reliability;
                    self.$out_of_service_field = out_of_service;
                    self.$saved_reliability_field = saved_reliability;
                    $(
                        self.$range_fault_field.owned_fault = range_fault_ownership;
                    )?
                    $(
                        self.$multistate_fault_field.owned_fault = multistate_fault_ownership;
                    )?
                    let _ = (range_fault_ownership, multistate_fault_ownership);
                    Ok(())
                }
                $crate::rollback::IntrinsicWriteRollback::MultiStateCommand {
                    priority_array,
                    relinquish_default,
                    present_value,
                    reliability,
                    fault_ownership,
                } => $crate::rollback::restore_multistate_command_rollback!(
                    self,
                    priority_array,
                    relinquish_default,
                    present_value,
                    reliability,
                    fault_ownership,
                    $reliability_field
                    $(; $multistate_fault_field
                        $(, $priority_array_field,
                            $relinquish_default_field,
                            $present_value_field)?
                    )?
                ),
            }
        }
    };
}
pub(crate) use capture_multistate_command_rollback;
pub(crate) use impl_intrinsic_write_rollback;
pub(crate) use restore_multistate_command_rollback;
