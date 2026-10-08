//! Matching and decoding of BVLC management responses for the B/IP client
//! helpers (`read_bdt`, `write_bdt`, `read_fdt`, `delete_fdt_entry`,
//! `register_foreign_device_bvlc`).

use std::sync::Arc;
use tokio::sync::oneshot;
use tokio::time::Instant;

use crate::bvll::BvllMessage;
use bacnet_types::enums::{BvlcFunction, BvlcResultCode};
use bacnet_types::error::Error;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum BvlcResponseKind {
    Result,
    ReadBroadcastDistributionTableAck,
    ReadForeignDeviceTableAck,
}

impl BvlcResponseKind {
    pub(super) fn accepts(self, function: BvlcFunction) -> bool {
        match self {
            Self::Result => function == BvlcFunction::BVLC_RESULT,
            Self::ReadBroadcastDistributionTableAck => {
                function == BvlcFunction::READ_BROADCAST_DISTRIBUTION_TABLE_ACK
                    || function == BvlcFunction::BVLC_RESULT
            }
            Self::ReadForeignDeviceTableAck => {
                function == BvlcFunction::READ_FOREIGN_DEVICE_TABLE_ACK
                    || function == BvlcFunction::BVLC_RESULT
            }
        }
    }
}

pub(super) struct PendingBvlcResponse {
    pub(super) owner: Arc<()>,
    pub(super) target: ([u8; 4], u16),
    pub(super) function: BvlcFunction,
    pub(super) expected: BvlcResponseKind,
    pub(super) ttl: Option<u16>,
    pub(super) deadline: Option<Instant>,
    /// At most one response received before local send completion; the bool
    /// records payload validity so accounting need not parse under its lock.
    pub(super) early_response: Option<(BvllMessage, bool)>,
    pub(super) tx: oneshot::Sender<BvllMessage>,
}

impl PendingBvlcResponse {
    pub(super) fn matches(&self, sender: ([u8; 4], u16), msg: &BvllMessage) -> bool {
        if self.target != sender || !self.expected.accepts(msg.function) {
            return false;
        }
        if msg.function != BvlcFunction::BVLC_RESULT {
            return true;
        }
        let Ok(code) = decode_bvlc_result_code(msg) else {
            return false;
        };
        let nak = match self.function {
            BvlcFunction::WRITE_BROADCAST_DISTRIBUTION_TABLE => {
                BvlcResultCode::WRITE_BROADCAST_DISTRIBUTION_TABLE_NAK
            }
            BvlcFunction::READ_BROADCAST_DISTRIBUTION_TABLE => {
                BvlcResultCode::READ_BROADCAST_DISTRIBUTION_TABLE_NAK
            }
            BvlcFunction::REGISTER_FOREIGN_DEVICE => BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK,
            BvlcFunction::READ_FOREIGN_DEVICE_TABLE => {
                BvlcResultCode::READ_FOREIGN_DEVICE_TABLE_NAK
            }
            BvlcFunction::DELETE_FOREIGN_DEVICE_TABLE_ENTRY => {
                BvlcResultCode::DELETE_FOREIGN_DEVICE_TABLE_ENTRY_NAK
            }
            _ => return false,
        };
        code == nak
            || (self.expected == BvlcResponseKind::Result
                && code == BvlcResultCode::SUCCESSFUL_COMPLETION)
    }
}

pub(super) fn expect_bvlc_function(msg: &BvllMessage, expected: BvlcFunction) -> Result<(), Error> {
    if msg.function == expected {
        Ok(())
    } else {
        Err(Error::Encoding(format!(
            "expected BVLC response {expected:?}, got {:?}",
            msg.function
        )))
    }
}

pub(super) fn decode_bvlc_result_code(msg: &BvllMessage) -> Result<BvlcResultCode, Error> {
    expect_bvlc_function(msg, BvlcFunction::BVLC_RESULT)?;
    if msg.payload.len() != std::mem::size_of::<u16>() {
        return Err(Error::Encoding(format!(
            "BVLC-Result payload must be 2 bytes, got {}",
            msg.payload.len()
        )));
    }

    Ok(BvlcResultCode::from_raw(u16::from_be_bytes([
        msg.payload[0],
        msg.payload[1],
    ])))
}

pub(super) fn bvlc_result_error(msg: &BvllMessage) -> Error {
    match decode_bvlc_result_code(msg) {
        Ok(code) => Error::Encoding(format!("BVLC-Result: {code:?}")),
        Err(err) => err,
    }
}
