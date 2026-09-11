//! Required-payload admission, deliberately separate from generic wire syntax.
use super::{ScFunction, ScMessage};

pub(crate) fn missing_npdu_payload(msg: &ScMessage) -> bool {
    msg.function == ScFunction::EncapsulatedNpdu && msg.payload.is_empty()
}
