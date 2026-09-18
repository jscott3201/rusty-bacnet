//! Concrete MS/TP controls at endpoint level (no generic-trait erasure).
//!
//! Composes ONE [`MstpTransport`] (one serial owner, one MAC state machine)
//! into an [`EndpointSession`] above the sibling roles, mirroring the
//! [`BipEndpointBuilder`](crate::bip::BipEndpointBuilder) pattern. Execution
//! placement (Tokio vs dedicated thread) is selected before start via
//! [`MstpEndpointBuilder::execution_mode`]; no serial or master-node setting
//! changes, and a running loop is never migrated.
//!
//! # Reply-path ownership (RB-17)
//!
//! Prompt replies route through the existing responder `reply_tx` path
//! (MAC `AnswerDataRequest` window, no token needed). Token-owned
//! initiations (client requests, broadcasts) plus deferred-after-postponed
//! responses go through the single [`EndpointEgress`](bacnet_endpoint_core::endpoint_ingress::EndpointEgress)
//! → `queue_npdu` path and transmit as `DataNotExpectingReply` at the next
//! token opportunity. The two stay distinct by construction: the responder
//! observes `reply_tx` present (prompt) or absent (token-owned), never both
//! for one request. Deferred correlation is session-side (one-shot
//! suspension on [`SessionToken`](crate::roles::SessionToken), resolved via
//! egress); the MAC keeps no extra state and no second serial owner exists.
//!
//! # Bounds and evidence level
//!
//! - `SessionConfig` keeps its 480 APDU default, which fits the 501
//!   frame-data cap; a composed identity advertising more than the
//!   transport's 480 APDU bound is rejected at build time.
//! - Standard frames only (no extended/COBS, no router capability — RB-25
//!   owns routing); non-routing endpoint behavior (the network layer
//!   discards DNET-addressed traffic).
//! - Simulator evidence level: proofs run over [`LoopbackSerial`] (ownership
//!   + frame sequencing). This is NOT a physical-bench or on-wire
//!   conformance claim; timing qualification is RB-26.

use bacnet_objects::database::ObjectDatabase;
use bacnet_transport::mstp::{MstpConfig, MstpExecutionMode, MstpTransport, SerialPort};
use bacnet_types::error::Error;

use crate::session::{EndpointSession, SessionConfig, SessionRole};

/// MS/TP transport APDU bound enforced at endpoint composition.
///
/// [`MstpTransport::max_apdu_length`](bacnet_transport::port::TransportPort::max_apdu_length)
/// advertises 480; an identity advertising more would invite oversized
/// responses that the standard-frame MAC must reject at `queue_npdu`.
const MSTP_MAX_APDU: u16 = 480;

/// Concrete MS/TP endpoint builder over one serial owner.
///
/// The builder holds the serial port by value (`Option<S>`, taken once at
/// build) so exactly one [`MstpTransport`] — one serial owner, one MAC state
/// machine — enters the session. No second hidden transport is created here
/// or in [`EndpointSession`].
#[doc(hidden)]
pub struct MstpEndpointBuilder<S: SerialPort> {
    serial: Option<S>,
    this_station: u8,
    max_master: u8,
    max_info_frames: u8,
    baud_rate: u32,
    execution_mode: MstpExecutionMode,
    role: SessionRole,
    session: SessionConfig,
    database: Option<ObjectDatabase>,
    identity: Option<crate::identity::DeviceIdentity>,
}

impl<S: SerialPort> MstpEndpointBuilder<S> {
    /// Creates an MS/TP endpoint builder holding `serial` for one station.
    ///
    /// Addressing defaults mirror [`MstpConfig::default`] except the station,
    /// which has no meaningful default: `max_master` 127, `max_info_frames`
    /// 1, `baud_rate` 9600. Execution defaults to Tokio.
    #[doc(hidden)]
    pub fn new(serial: S, this_station: u8) -> Self {
        Self {
            serial: Some(serial),
            this_station,
            max_master: 127,
            max_info_frames: 1,
            baud_rate: 9600,
            execution_mode: MstpExecutionMode::default(),
            role: SessionRole::Both,
            session: SessionConfig::default(),
            database: None,
            identity: None,
        }
    }

    /// Sets the maximum master address on the link.
    #[doc(hidden)]
    pub fn max_master(mut self, max_master: u8) -> Self {
        self.max_master = max_master;
        self
    }

    /// Sets the maximum information frames per token use.
    #[doc(hidden)]
    pub fn max_info_frames(mut self, max_info_frames: u8) -> Self {
        self.max_info_frames = max_info_frames;
        self
    }

    /// Sets the baud rate used for MAC timing derivation.
    #[doc(hidden)]
    pub fn baud_rate(mut self, baud_rate: u32) -> Self {
        self.baud_rate = baud_rate;
        self
    }

    /// Selects MAC-loop execution placement for the next start.
    ///
    /// Tokio (default) polls the MAC loop on the caller's runtime;
    /// DedicatedThread isolates MAC polling on its own OS thread + runtime.
    /// Same logical behavior in both; the RB-17 proof runs every wire
    /// scenario in each mode.
    #[doc(hidden)]
    pub fn execution_mode(mut self, mode: MstpExecutionMode) -> Self {
        self.execution_mode = mode;
        self
    }

    /// Selects the composed roles.
    #[doc(hidden)]
    pub fn role(mut self, role: SessionRole) -> Self {
        self.role = role;
        self
    }

    /// Sets the bounded queue capacity for every ingress/egress queue.
    #[doc(hidden)]
    pub fn queue_capacity(mut self, capacity: usize) -> Self {
        self.session.queue_capacity = capacity;
        self
    }

    /// Sets client APDU timeout/retries (session-owned timers).
    #[doc(hidden)]
    pub fn client_timers(mut self, timeout_ms: u64, retries: u8) -> Self {
        self.session.apdu_timeout_ms = timeout_ms;
        self.session.apdu_retries = retries;
        self
    }

    /// Attaches the object database for the server responder.
    #[doc(hidden)]
    pub fn database(mut self, db: ObjectDatabase) -> Self {
        self.database = Some(db);
        self
    }

    /// Composes the single Device identity.
    ///
    /// Truth direction (mirror [`BipEndpointBuilder`](crate::bip::BipEndpointBuilder)):
    /// the database should already be built from the same identity; this
    /// only wires I-Am + role limits. The identity max-APDU must fit the
    /// MS/TP 480 transport bound (checked at [`build_session`](Self::build_session)).
    #[doc(hidden)]
    pub fn identity(mut self, identity: crate::identity::DeviceIdentity) -> Self {
        self.identity = Some(identity);
        self
    }

    /// Validates addressing without consuming the serial owner.
    #[doc(hidden)]
    pub fn validate_only(&self) -> Result<(), Error> {
        Self::validate_addressing(self.this_station, self.max_master)
    }

    fn validate_addressing(this_station: u8, max_master: u8) -> Result<(), Error> {
        // Mirror `MasterNode::new` so misconfiguration fails fast at
        // composition time: master-only endpoint, station within the ring.
        if max_master > 127 {
            return Err(Error::Encoding(format!(
                "MS/TP endpoint: max_master {max_master} exceeds MAX_MASTER (127)"
            )));
        }
        if this_station > max_master {
            return Err(Error::Encoding(format!(
                "MS/TP endpoint: this_station {this_station} exceeds configured max_master ({max_master})"
            )));
        }
        Ok(())
    }

    /// Builds the concrete MS/TP transport, taking the serial owner once.
    #[doc(hidden)]
    pub fn build_transport(mut self) -> Result<MstpTransport<S>, Error> {
        Self::validate_addressing(self.this_station, self.max_master)?;
        let serial = self
            .serial
            .take()
            .ok_or_else(|| Error::Encoding("MS/TP endpoint serial owner is missing".into()))?;
        let config = MstpConfig {
            this_station: self.this_station,
            max_master: self.max_master,
            max_info_frames: self.max_info_frames,
            baud_rate: self.baud_rate,
        };
        Ok(MstpTransport::new(serial, config).with_execution_mode(self.execution_mode))
    }

    /// Builds an unstarted session (caller drives `start()`/`stop()` once).
    ///
    /// One-link proof: this builds exactly one [`MstpTransport`] (one serial
    /// owner after `start()`); no second serial owner or duplicated MAC state
    /// machine is created here or in [`EndpointSession`]. The composed
    /// identity must fit the 480 APDU bound; the standalone
    /// [`SessionConfig`] 480 default is kept untouched otherwise.
    #[doc(hidden)]
    pub fn build_session(mut self) -> Result<EndpointSession<MstpTransport<S>>, Error> {
        Self::validate_addressing(self.this_station, self.max_master)?;
        if let Some(identity) = self.identity.as_ref() {
            // Respect the transport's advertised standard-frame/APDU bounds:
            // never compose an identity that invites oversized responses.
            if identity.max_apdu_length() > MSTP_MAX_APDU {
                return Err(Error::Encoding(format!(
                    "MS/TP endpoint: identity max-APDU {} exceeds MS/TP transport bound {MSTP_MAX_APDU}",
                    identity.max_apdu_length()
                )));
            }
        }
        let role = self.role;
        let session = self.session.clone();
        let database = self.database.take();
        let identity = self.identity.take();
        let transport = self.build_transport()?;
        let mut endpoint = EndpointSession::new(transport, role, session)?;
        if let Some(db) = database {
            endpoint = endpoint.with_database(db);
        }
        if let Some(id) = identity {
            endpoint = endpoint.with_identity(id);
        }
        Ok(endpoint)
    }
}
