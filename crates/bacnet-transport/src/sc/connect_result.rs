use bacnet_types::enums::{ErrorClass, ErrorCode};
use bacnet_types::error::Error;

use crate::sc_frame::{ScBvlcResult, ScFunction};

use super::{ScConnection, ScConnectionState, generate_random48_vmac};

impl ScConnection {
    pub(crate) fn abort_connect(&mut self) {
        self.state = ScConnectionState::Disconnected;
        self.pending_connect_message_id = None;
    }

    /// Handle a BVLC-Result received while waiting for Connect-Accept.
    ///
    /// A NODE_DUPLICATE_VMAC NAK means another node already uses our VMAC,
    /// so per AB.6.2.2 the initiating peer has to draw a fresh Random-48 VMAC
    /// and use it for its next connection attempt.
    pub fn handle_connect_result(
        &mut self,
        result_message_id: u16,
        result: &ScBvlcResult,
    ) -> Result<bool, Error> {
        let duplicate_vmac = self.connect_result_requires_random48_vmac(result_message_id, result);
        self.abort_connect();

        if duplicate_vmac {
            match generate_random48_vmac() {
                Ok(vmac) => {
                    self.local_vmac = vmac;
                    self.connect_retry_allowed = true;
                }
                Err(e) => {
                    self.connect_retry_allowed = false;
                    return Err(e);
                }
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn connect_result_requires_random48_vmac(
        &self,
        result_message_id: u16,
        result: &ScBvlcResult,
    ) -> bool {
        if self.pending_connect_message_id != Some(result_message_id) {
            return false;
        }

        let ScBvlcResult::Nak {
            result_for,
            error_class,
            error_code,
            ..
        } = result
        else {
            return false;
        };

        *result_for == ScFunction::ConnectRequest
            && *error_class == ErrorClass::COMMUNICATION.to_raw()
            && *error_code == ErrorCode::NODE_DUPLICATE_VMAC.to_raw()
    }
}
