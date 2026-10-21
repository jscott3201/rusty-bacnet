//! Concrete MS/TP endpoint builder over one serial owner.
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
//! suspension on the session token, resolved via egress); the MAC keeps no
//! extra state and no second serial owner exists.
//!
//! # Bounds and evidence level
//!
//! - An identity-free `SessionConfig` keeps its 480 APDU default. At start,
//!   a composed identity's requested ceiling resolves against the stable
//!   local receive capacity: 480 with standard frames, the default, or 1476
//!   with [`MstpEndpointBuilder::extended_frames`]. An omitted database
//!   generates a Device after resolution; a supplied Device must match.
//! - No router capability (RB-25 owns routing): non-routing endpoint
//!   behavior, the network layer discarding DNET-addressed traffic.
//! - **Simulator evidence only**: proofs run over [`LoopbackSerial`] with a
//!   raw-frame peer harness (ownership + frame sequencing). This is NOT a
//!   physical-bench or on-wire conformance claim; timing qualification is
//!   RB-26. Do not present MS/TP mode as bench-qualified.
//!
//! [`LoopbackSerial`]: bacnet_transport::mstp::LoopbackSerial
//! [`MstpTransport`]: bacnet_transport::mstp::MstpTransport

use std::time::Duration;

use bacnet_objects::database::ObjectDatabase;
use bacnet_transport::mstp::{MstpConfig, MstpExecutionMode, MstpTransport, SerialPort};
use bacnet_types::error::Error;

use crate::session::{EndpointSession, SessionConfig, SessionRole};

/// Concrete MS/TP endpoint builder over one serial owner.
///
/// The builder holds the serial port by value (`Option<S>`, taken once at
/// build) so exactly one [`MstpTransport`] — one serial owner, one MAC state
/// machine — enters the session. No second hidden transport is created here
/// or in [`EndpointSession`].
///
/// Simulator evidence only (see module docs): no bench or conformance claim,
/// RB-26 owns timing.
///
/// # Echo
///
/// Whether the port hears its own frames is the port's to declare, as for
/// an [`MstpTransport`] built by hand: build it with
/// `with_echo(Echo::full())` (`TokioSerialPort`, [`LoopbackSerial`]) and
/// the transport strips the echo. Echo stripping is off unless the port
/// declares it, and [`validate_only`](Self::validate_only) refuses a zero
/// grace. [`Echo`] carries the grace inside its `Full` variant, so a grace
/// without an echo can't be expressed.
///
/// [`Echo`]: bacnet_transport::mstp::Echo
/// [`LoopbackSerial`]: bacnet_transport::mstp::LoopbackSerial
///
/// ```
/// use bacnet_endpoint::mstp::MstpEndpointBuilder;
/// use bacnet_transport::mstp::{Echo, LoopbackSerial};
///
/// let (session_end, _peer_end) = LoopbackSerial::pair();
/// let builder = MstpEndpointBuilder::new(session_end, 3);
/// assert!(builder.validate_only().is_ok());
///
/// // An adapter that hears its own frames, on a link with extended frames.
/// let (session_end, _peer_end) = LoopbackSerial::pair();
/// let builder = MstpEndpointBuilder::new(session_end.with_echo(Echo::full()), 3)
///     .extended_frames(true);
/// assert!(builder.validate_only().is_ok());
/// ```
pub struct MstpEndpointBuilder<S: SerialPort> {
    serial: Option<S>,
    this_station: u8,
    max_master: u8,
    max_info_frames: u8,
    baud_rate: u32,
    extended_frames: bool,
    usage_timeout: Duration,
    reply_timeout: Duration,
    transmit_latency_pad: Duration,
    execution_mode: MstpExecutionMode,
    role: SessionRole,
    session: SessionConfig,
    database: Option<ObjectDatabase>,
    identity: Option<crate::identity::DeviceIdentity>,
}

impl<S: SerialPort> MstpEndpointBuilder<S> {
    /// Creates an MS/TP endpoint builder holding `serial` for one station.
    ///
    /// Link defaults mirror [`MstpConfig::default`] except the station,
    /// which has no meaningful default: `max_master` 127, `max_info_frames`
    /// 1, `baud_rate` 9600, standard frames only, the host timeouts (35 ms
    /// and 300 ms) and the transmit-latency pad (4 ms on Windows, zero
    /// elsewhere). Execution defaults to Tokio.
    pub fn new(serial: S, this_station: u8) -> Self {
        let link = MstpConfig::default();
        Self {
            serial: Some(serial),
            this_station,
            max_master: 127,
            max_info_frames: 1,
            baud_rate: 9600,
            extended_frames: false,
            usage_timeout: link.usage_timeout,
            reply_timeout: link.reply_timeout,
            transmit_latency_pad: link.transmit_latency_pad,
            execution_mode: MstpExecutionMode::default(),
            role: SessionRole::Both,
            session: SessionConfig::default(),
            database: None,
            identity: None,
        }
    }

    /// Sets the maximum master address on the link (0..=127).
    pub fn max_master(mut self, max_master: u8) -> Self {
        self.max_master = max_master;
        self
    }

    /// Sets the maximum information frames per token use.
    pub fn max_info_frames(mut self, max_info_frames: u8) -> Self {
        self.max_info_frames = max_info_frames;
        self
    }

    /// Sets the baud rate used for MAC timing derivation.
    pub fn baud_rate(mut self, baud_rate: u32) -> Self {
        self.baud_rate = baud_rate;
        self
    }

    /// Takes and sends extended frames, Types 32 and 33, which carry NPDUs
    /// of up to 1497 octets (default: off, standard frames of up to 501).
    ///
    /// Clause 9.3 leaves them optional for a device. With them on, the
    /// transport carries APDUs of 1476 octets instead of 480, so the
    /// composed identity may advertise a Max_APDU_Length_Accepted of up to
    /// 1476 (startup resolves the requested ceiling), and the
    /// link's other nodes must take extended frames too: one without them
    /// counts a frame of over 501 octets as invalid. See
    /// [`MstpConfig::extended_frames`].
    ///
    /// Memory: the port's transmit queues reserve 512 slots, 768 KiB of
    /// 1536-octet slots with extended frames against 272 KiB of 544-octet
    /// ones without; pages a queue never fills usually don't become
    /// resident.
    pub fn extended_frames(mut self, extended_frames: bool) -> Self {
        self.extended_frames = extended_frames;
        self
    }

    /// Sets T_usage_timeout, 20 to 35 ms (default: the host's 35 ms); see
    /// [`MstpConfig::usage_timeout`]. A value outside that range fails
    /// [`validate_only`](Self::validate_only).
    pub fn usage_timeout(mut self, usage_timeout: Duration) -> Self {
        self.usage_timeout = usage_timeout;
        self
    }

    /// Sets T_reply_timeout, 255 to 300 ms (default: the host's 300 ms); see
    /// [`MstpConfig::reply_timeout`]. A value outside that range fails
    /// [`validate_only`](Self::validate_only).
    pub fn reply_timeout(mut self, reply_timeout: Duration) -> Self {
        self.reply_timeout = reply_timeout;
        self
    }

    /// Sets the transmit-latency pad for a port with no drain, at most
    /// 15 ms (default: 4 ms on Windows, zero elsewhere); see
    /// [`MstpConfig::transmit_latency_pad`]. A longer one fails
    /// [`validate_only`](Self::validate_only).
    pub fn transmit_latency_pad(mut self, pad: Duration) -> Self {
        self.transmit_latency_pad = pad;
        self
    }

    /// Selects MAC-loop execution placement for the next start.
    ///
    /// Tokio (default) polls the MAC loop on the caller's runtime;
    /// DedicatedThread isolates MAC polling on its own OS thread + runtime.
    /// Same logical behavior in both; the RB-17 proof runs every wire
    /// scenario in each mode. Never migrates a running loop.
    pub fn execution_mode(mut self, mode: MstpExecutionMode) -> Self {
        self.execution_mode = mode;
        self
    }

    /// Selects the composed roles (default [`Both`](SessionRole::Both)).
    pub fn role(mut self, role: SessionRole) -> Self {
        self.role = role;
        self
    }

    /// Sets the bounded queue capacity for every ingress/egress queue.
    pub fn queue_capacity(mut self, capacity: usize) -> Self {
        self.session.queue_capacity = capacity;
        self
    }

    /// Sets the server role's read work limit (default 256): the result rows
    /// one ReadProperty may expand, a Group's member rows included. See
    /// [`SessionConfig::read_work_limit`]; zero fails
    /// [`build_session`](Self::build_session) via [`EndpointSession::new`].
    pub fn read_work_limit(mut self, limit: usize) -> Self {
        self.session.read_work_limit = limit;
        self
    }

    /// Sets client APDU timeout/retries (session-owned timers).
    pub fn client_timers(mut self, timeout_ms: u64, retries: u8) -> Self {
        self.session.apdu_timeout_ms = timeout_ms;
        self.session.apdu_retries = retries;
        self
    }

    /// Least time between the client role's confirmed requests to one
    /// destination (default 0: no pacing). See
    /// [`SessionConfig::min_request_interval_ms`]; more than an hour fails
    /// [`build_session`](Self::build_session).
    pub fn min_request_interval_ms(mut self, ms: u64) -> Self {
        self.session.min_request_interval_ms = ms;
        self
    }

    /// Attaches the object database for the server responder.
    ///
    /// Build it from the same identity passed to
    /// [`device`](Self::device) so Device readback agrees with I-Am.
    pub fn database(mut self, db: ObjectDatabase) -> Self {
        self.database = Some(db);
        self
    }

    /// Composes the single Device identity.
    ///
    /// At start, its requested ceiling resolves against the transport's
    /// stable receive capacity: 480, or 1476 with
    /// [`extended_frames`](Self::extended_frames). With no database, the
    /// resolved identity generates a Device. A supplied database is preserved
    /// and its selected Device must match the resolved identity.
    pub fn device(mut self, identity: crate::identity::DeviceIdentity) -> Self {
        self.identity = Some(identity);
        self
    }

    /// Validates the MS/TP configuration without consuming the serial owner.
    ///
    /// Runs the transport's own checks ([`MstpConfig::validate`]):
    /// `max_master <= 127`, `this_station <= max_master`,
    /// `max_info_frames >= 1`, a baud rate Clause 9 timing allows, the two
    /// timeouts within Clause 9.5.3's ranges and a pad of at most 15 ms,
    /// plus a nonzero echo grace on a port that declares an echo
    /// ([`Echo::validate`](bacnet_transport::mstp::Echo::validate)); returns
    /// a typed [`Error::Encoding`] otherwise.
    pub fn validate_only(&self) -> Result<(), Error> {
        self.config().validate()?;
        match &self.serial {
            Some(serial) => serial.echo().validate(),
            None => Ok(()),
        }
    }

    fn config(&self) -> MstpConfig {
        MstpConfig {
            this_station: self.this_station,
            max_master: self.max_master,
            max_info_frames: self.max_info_frames,
            baud_rate: self.baud_rate,
            // A device, not a router: standard frames unless asked for
            // (Clause 9.3 requires extended frames of routers only).
            extended_frames: self.extended_frames,
            usage_timeout: self.usage_timeout,
            reply_timeout: self.reply_timeout,
            transmit_latency_pad: self.transmit_latency_pad,
        }
    }

    /// Builds the concrete MS/TP transport, taking the serial owner once.
    ///
    /// Validates the configuration first, then takes the serial port by
    /// value: a second call returns `"MS/TP endpoint serial owner is missing"`.
    pub fn build_transport(mut self) -> Result<MstpTransport<S>, Error> {
        self.validate_only()?;
        let config = self.config();
        let serial = self
            .serial
            .take()
            .ok_or_else(|| Error::Encoding("MS/TP endpoint serial owner is missing".into()))?;
        Ok(MstpTransport::new(serial, config).with_execution_mode(self.execution_mode))
    }

    /// Builds an unstarted session (caller drives `start()`/`stop()` once).
    ///
    /// The transport's counters stay reachable through
    /// [`EndpointSession::mstp_diagnostics`].
    ///
    /// One-link proof: this builds exactly one [`MstpTransport`] (one serial
    /// owner after `start()`); no second serial owner or duplicated MAC state
    /// machine is created here or in [`EndpointSession`]. At start, a composed
    /// identity resolves against the transport's receive capacity before
    /// generating or validating its Device. An identity-free session keeps
    /// the [`SessionConfig`] 480 default.
    pub fn build_session(mut self) -> Result<EndpointSession<MstpTransport<S>>, Error> {
        self.validate_only()?;
        let role = self.role;
        let session = self.session.clone();
        let database = self.database.take();
        let identity = self.identity.take();
        let transport = self.build_transport()?;
        let diagnostics = transport.diagnostics();
        let mut endpoint = EndpointSession::new(transport, role, session)?;
        endpoint.mstp_diagnostics = Some(diagnostics);
        if let Some(db) = database {
            endpoint = endpoint.with_database(db);
        }
        if let Some(id) = identity {
            endpoint = endpoint.device(id);
        }
        Ok(endpoint)
    }
}
