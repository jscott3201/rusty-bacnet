//! Object-owned snapshots for WritePropertyMultiple rollback.

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
        message_texts: [String; 3],
    },
    TimeDelayNormal(Option<u32>),
}

/// Preserve event state that the detection-disable reset clears, plus the raw
/// optional `Time_Delay_Normal` backing value that effective readback hides.
macro_rules! impl_intrinsic_write_rollback {
    ($detector_field:ident, $detection_enable_field:ident, $history_field:ident) => {
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
                    message_texts,
                } => {
                    self.$detection_enable_field = enabled;
                    self.$detector_field.event_state = event_state;
                    self.$detector_field.acked_transitions = acked_transitions;
                    self.$detector_field.pending = pending;
                    self.$detector_field.fault_reliability = fault_reliability;
                    self.$history_field.time_stamps = time_stamps;
                    self.$history_field.message_texts = message_texts;
                    Ok(())
                }
                $crate::rollback::IntrinsicWriteRollback::TimeDelayNormal(value) => {
                    self.$detector_field.time_delay_normal = value;
                    Ok(())
                }
            }
        }
    };
}
pub(crate) use impl_intrinsic_write_rollback;
