//! Established-node Advertisement/Solicitation admission plus the accepted
//! solicitation predicate and solicited-reply rate policy. Generic codec
//! validation lives in `sc_frame`; received-Advertisement peer status
//! tracking is not kept (no local peer store); solicited replies are
//! originated by the transport loop, keeping `ScConnection::handle_received`
//! pure.

use std::time::Duration;

use bacnet_types::enums::{ErrorClass, ErrorCode};
use bytes::BytesMut;
use tokio::sync::mpsc;
use tracing::warn;

use super::data_attributes::build_bvlc_result_nak;
use super::rejection::{RejectionBudget, RejectionExpired};
use super::{ScConnection, WebSocketPort};
use crate::port::ReceivedNpdu;
use crate::sc_frame::{
    advertisement_message_error, encode_sc_message,
    first_must_understand_destination_option_marker, ScFunction, ScMessage, Vmac, BROADCAST_VMAC,
};

/// Minimum spacing between solicited Advertisements answered by this node.
///
/// Local anti-storm policy (not a wire deadline): floods of solicitations
/// collapse to at most one Advertisement per interval. Chosen as a plain
/// constant rather than a RejectionBudget because the reply is a solicited
/// positive transmission, not a rejection NAK; the send itself stays
/// best-effort like Heartbeat-ACK.
pub(super) const SOLICITED_ADVERTISEMENT_MIN_INTERVAL: Duration = Duration::from_secs(1);

/// The listener's sole intake is merged by the existing transport receive task.
/// A closed/absent receiver remains pending, never spinning or ending hub intake.
#[derive(Default)]
pub(super) struct DirectIntake {
    npdus: Option<mpsc::Receiver<ReceivedNpdu>>,
    #[cfg(feature = "sc-tls")]
    listener: Option<ListenerStatus>,
}

#[cfg(feature = "sc-tls")]
struct ListenerStatus {
    shutdown: tokio::sync::watch::Receiver<bool>,
    vmac: Vmac,
    uuid: [u8; 16],
}

impl DirectIntake {
    pub(super) async fn recv(&mut self) -> Option<ReceivedNpdu> {
        match &mut self.npdus {
            Some(rx) => {
                let npdu = rx.recv().await;
                if npdu.is_none() {
                    self.npdus = None;
                }
                npdu
            }
            None => std::future::pending().await,
        }
    }

    /// AB.2.8.1 capability sampled from real admission and an open intake.
    /// Identity may change on a Connect retry; never advertise another VMAC's listener.
    pub(super) fn accepts_direct(&self, _conn: &ScConnection) -> bool {
        #[cfg(feature = "sc-tls")]
        return self.npdus.as_ref().is_some_and(|rx| !rx.is_closed())
            && self.listener.as_ref().is_some_and(|listener| {
                !*listener.shutdown.borrow()
                    && listener.vmac == _conn.local_vmac
                    && listener.uuid == _conn.device_uuid
            });
        #[cfg(not(feature = "sc-tls"))]
        false
    }
}

#[cfg(feature = "sc-tls")]
impl<W: WebSocketPort> super::ScTransport<W> {
    /// Start and register an opt-in direct listener before transport start.
    ///
    /// Returns `(transport, listener)`: the application must retain the listener
    /// and can stop/drop it independently. Its sole NPDU receiver is merged into
    /// the receiver returned by [`crate::port::TransportPort::start`], with the
    /// existing bounded, drop-on-full policy. No forwarding task is spawned.
    /// Keep the listener only while using the transport; transport stop/drop
    /// closes its intake but does not take ownership of this application handle.
    ///
    /// Solicited Advertisements report accept-direct 1 only while this listener
    /// is live, identities match, and both intakes are open. Stop/drop restores 0
    /// at the next send decision; already-built/sent frames cannot be recalled.
    /// Registering never enables discovery/dial-out or infers public URIs: use
    /// `with_advertised_uris` for known URIs (AB.3.3), or leave the list empty.
    ///
    /// Rejects an already-started transport, duplicate registration or mismatched
    /// VMAC/UUID before binding. Configure the device UUID first. Other errors
    /// are those of [`crate::sc_tls::DirectListener::start`].
    ///
    /// ```no_run
    /// use bacnet_transport::{port::TransportPort, sc::{ScTransport, WebSocketPort},
    ///     sc_tls::{DirectAcceptConfig, ScNodeTlsConfig}};
    /// # async fn run(ws: impl WebSocketPort, tls: ScNodeTlsConfig,
    /// #     vmac: [u8; 6], persisted_uuid: [u8; 16]) -> Result<(), bacnet_types::error::Error> {
    /// let config = DirectAcceptConfig::new(
    ///     "127.0.0.1:0".parse().unwrap(), vmac, persisted_uuid, tls);
    /// let (mut transport, mut listener) = ScTransport::new(ws, vmac)
    ///     .with_device_uuid(persisted_uuid).with_direct_listener(config).await?;
    /// let mut incoming = transport.start().await?;
    /// // Retain listener while consuming both hub and direct NPDUs here.
    /// let received = incoming.recv().await;
    /// listener.stop().await;
    /// transport.stop().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn with_direct_listener(
        mut self,
        config: crate::sc_tls::DirectAcceptConfig,
    ) -> Result<(Self, crate::sc_tls::DirectListener), bacnet_types::error::Error> {
        if self.ws.is_none() || self.direct_intake.npdus.is_some() {
            return Err(bacnet_types::error::Error::Encoding(
                "SC direct listener requires an unstarted, unregistered transport".into(),
            ));
        }
        if !config.matches_identity(self.local_vmac, self.device_uuid) {
            return Err(bacnet_types::error::Error::Encoding(
                "SC direct listener identity does not match transport".into(),
            ));
        }
        let (listener, rx) = crate::sc_tls::DirectListener::start(config).await?;
        self.direct_intake = DirectIntake {
            npdus: Some(rx),
            listener: Some(ListenerStatus {
                shutdown: listener.shutdown_status(),
                vmac: self.local_vmac,
                uuid: self.device_uuid,
            }),
        };
        Ok((self, listener))
    }
}

/// Accepted-solicitation predicate for AB.3.2 solicited replies.
///
/// Returns the reply destination (the solicitation origin, or `None` for a
/// hub-peer solicitation so the reply stays peer-addressed) when the frame
/// is a well-formed, locally-addressed solicitation. Malformed shapes keep
/// the existing NAK path; addressed or reserved-origin envelopes stay
/// silent; other functions are not solicitations.
pub(super) fn solicited_advertisement_destination(msg: &ScMessage) -> Option<Option<Vmac>> {
    if msg.function != ScFunction::AdvertisementSolicitation {
        return None;
    }
    if advertisement_message_error(msg).is_some() {
        return None;
    }
    if msg.destination_vmac.is_some() {
        return None;
    }
    if matches!(msg.originating_vmac, Some(vmac) if vmac == [0; 6] || vmac == BROADCAST_VMAC) {
        return None;
    }
    Some(msg.originating_vmac)
}

pub(super) async fn reject<W: WebSocketPort>(
    msg: &ScMessage,
    wire: &[u8],
    ws: &W,
    budget: RejectionBudget,
) -> Result<bool, RejectionExpired> {
    if !matches!(
        msg.function,
        ScFunction::Advertisement | ScFunction::AdvertisementSolicitation
    ) {
        return Ok(false);
    }
    // Hub-connector AB.5.4: explicit destinations are not for the local BVLL.
    // Reserved origins are also silent; neither decision spends a budget.
    if msg.destination_vmac.is_some()
        || matches!(msg.originating_vmac, Some(vmac) if vmac == [0; 6] || vmac == BROADCAST_VMAC)
    {
        return Ok(true);
    }
    // Valid shapes are consumed silently with accepted activity (no NPDU, no
    // state change). Solicited Advertisement transmissions for accepted
    // solicitations are originated by the transport loop, not admission.
    let Some(code) = advertisement_message_error(msg) else {
        return Ok(false);
    };
    let marker = if code == ErrorCode::HEADER_NOT_UNDERSTOOD {
        match first_must_understand_destination_option_marker(wire) {
            Some(marker) => marker,
            None => return Ok(true),
        }
    } else {
        0
    };
    let nak = build_bvlc_result_nak(
        msg.message_id,
        msg.function,
        marker,
        msg.originating_vmac,
        ErrorClass::COMMUNICATION,
        code,
    );
    let mut bytes = BytesMut::new();
    encode_sc_message(&mut bytes, &nak);
    if let Err(e) = budget.send(ws, &bytes).await? {
        warn!("BACnet/SC advertisement NAK send error: {}", e);
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sc_frame::decode_sc_message;

    fn solicitation(origin: Option<Vmac>, dest: Option<Vmac>, payload: &[u8]) -> ScMessage {
        ScMessage {
            function: ScFunction::AdvertisementSolicitation,
            message_id: 0x2233,
            originating_vmac: origin,
            destination_vmac: dest,
            dest_options: Vec::new(),
            data_options: Vec::new(),
            payload: bytes::Bytes::copy_from_slice(payload),
        }
    }

    #[test]
    fn predicate_accepts_only_locally_addressed_valid_solicitations() {
        assert_eq!(
            solicited_advertisement_destination(&solicitation(None, None, &[])),
            Some(None)
        );
        assert_eq!(
            solicited_advertisement_destination(&solicitation(Some([0x22; 6]), None, &[])),
            Some(Some([0x22; 6]))
        );
    }

    #[test]
    fn predicate_rejects_malformed_addressed_and_reserved_solicitations() {
        // Malformed shapes keep the NAK path (no reply destination).
        assert_eq!(
            solicited_advertisement_destination(&solicitation(None, None, &[0x00])),
            None
        );
        // Explicit destinations are not for the local BVLL.
        assert_eq!(
            solicited_advertisement_destination(&solicitation(None, Some([0x44; 6]), &[])),
            None
        );
        assert_eq!(
            solicited_advertisement_destination(&solicitation(
                Some([0x22; 6]),
                Some([0x44; 6]),
                &[]
            )),
            None
        );
        // Reserved origins stay silent.
        for origin in [Some([0; 6]), Some(BROADCAST_VMAC)] {
            assert_eq!(
                solicited_advertisement_destination(&solicitation(origin, None, &[])),
                None
            );
        }
        // Must-Understand destination options are faults, not replies.
        let mut mu = solicitation(None, None, &[]);
        mu.dest_options.push(crate::sc_frame::ScOption {
            option_type: 2,
            must_understand: true,
            data: Vec::new(),
        });
        assert_eq!(solicited_advertisement_destination(&mu), None);
    }

    #[test]
    fn predicate_ignores_other_functions() {
        for function in [
            ScFunction::Advertisement,
            ScFunction::EncapsulatedNpdu,
            ScFunction::HeartbeatRequest,
        ] {
            let mut msg = solicitation(None, None, &[]);
            msg.function = function;
            assert_eq!(solicited_advertisement_destination(&msg), None);
        }
    }

    #[test]
    fn predicate_agrees_with_wire_codec() {
        // The predicate accepts exactly the wire shapes the rejection gate
        // lets through as valid solicitations.
        let wire_valid = [0x05u8, 0x00, 0x22, 0x33];
        let msg = decode_sc_message(&wire_valid).unwrap();
        assert_eq!(solicited_advertisement_destination(&msg), Some(None));
        let wire_payload = [0x05u8, 0x00, 0x22, 0x33, 0x00];
        let msg = decode_sc_message(&wire_payload).unwrap();
        assert_eq!(solicited_advertisement_destination(&msg), None);
    }
}
