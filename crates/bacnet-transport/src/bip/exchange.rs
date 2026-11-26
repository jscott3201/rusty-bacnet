//! This transport's own management requests (`read_bdt` and the other
//! helpers): one exchange with the node each, from admission to the
//! response, a timeout, a failed send or the caller giving up. Each helper
//! reports a failure as a [`BvlcExchangeError`] (#1822).

use std::fmt;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use bacnet_bip_codec::{self as codec, BdtEntries, BvlcFunction, BvlcResultCode, Bvll};
use bacnet_bip_core::Duration as CoreDuration;
use bacnet_bip_core::node::{ExchangeId, ExchangeRefusal, LocalFailure};
use bacnet_types::error::{Error, Lifecycle};
use bacnet_types::sync::MutexExt;
use bytes::BytesMut;
use tokio::sync::oneshot;

use super::datagram::OutgoingDatagram;
use super::node::{NodeHandle, Outcome, Waiter};
use super::{BipTransport, BvlcClientSnapshot, BvlcClientStatus};
use crate::bbmd::{self, BdtEntry, FdtEntryWire};
use crate::bvll::{BvllMessage, decode_bip_mac, encode_bvll};

/// Why one of [`BipTransport`]'s BVLC management requests
/// ([`read_bdt`](BipTransport::read_bdt) and its siblings) failed.
///
/// A peer that answers with a NAK result, one that doesn't answer and a
/// local failure each have their own variant, so a caller tells a refusal
/// from silence without reading the message text.
#[derive(Debug)]
#[non_exhaustive]
pub enum BvlcExchangeError {
    /// The peer answered with a BVLC-Result other than success: it refused
    /// the request, or doesn't carry such requests out, as a node that is no
    /// BBMD answers the table reads. The code is the one it sent.
    Nak(BvlcResultCode),
    /// No response came within this long. The wait starts when the
    /// exchange is admitted, just before the request is sent.
    NoReply(Duration),
    /// The peer answered with a message this request does not take, such as
    /// a success result to a table read.
    UnexpectedResponse {
        /// The function the request takes as its answer.
        expected: BvlcFunction,
        /// The function that came.
        got: BvlcFunction,
    },
    /// The node refused the request before sending it (#1841): the same
    /// request, or another one a success answers, is already in flight to
    /// the peer, every exchange slot is in use, or the peer is this foreign
    /// device's own BBMD for a request the node's registration owns. Nothing
    /// was sent; a busy refusal may pass on a later try.
    Refused(ExchangeRefusal),
    /// The request failed on this side or the answer could not be read: the
    /// transport is not started or had stopped, with nothing sent (an
    /// [`Error::State`] of [`Lifecycle::NotRunning`], #1833); it stopped
    /// while the request waited for its answer, which may have been
    /// carried out ([`Error::RequestCutOff`] with `sent: true`); the send
    /// failed, the request could not be encoded or the response was
    /// malformed.
    Transport(Error),
}

impl BvlcExchangeError {
    /// The result code of a NAK, or `None` for any other failure.
    #[must_use]
    pub fn nak(&self) -> Option<BvlcResultCode> {
        match self {
            Self::Nak(code) => Some(*code),
            _ => None,
        }
    }
}

impl fmt::Display for BvlcExchangeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Nak(code) => write!(f, "BVLC-Result NAK {code} ({:#06X})", code.to_raw()),
            Self::NoReply(timeout) => write!(f, "no BVLC response within {timeout:?}"),
            Self::UnexpectedResponse { expected, got } => {
                write!(f, "expected BVLC response {expected:?}, got {got:?}")
            }
            Self::Refused(
                refusal @ (ExchangeRefusal::SameExchange | ExchangeRefusal::ResultClass),
            ) => write!(f, "BVLC management request already in flight: {refusal}"),
            Self::Refused(refusal) => write!(f, "BVLC management request refused: {refusal}"),
            Self::Transport(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for BvlcExchangeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Refused(refusal) => Some(refusal),
            Self::Transport(error) => Some(error),
            _ => None,
        }
    }
}

/// A local failure, or a timeout as [`BvlcExchangeError::NoReply`]: the
/// node fails a waiter that saw no answer with [`Error::Timeout`].
impl From<Error> for BvlcExchangeError {
    fn from(error: Error) -> Self {
        match error {
            Error::Timeout(timeout) => Self::NoReply(timeout),
            other => Self::Transport(other),
        }
    }
}

/// A manual exchange admitted by the node. Dropping it before it ends
/// gives it up, as when the caller stops waiting.
pub(super) struct Exchange {
    node: Arc<NodeHandle>,
    id: ExchangeId,
}

impl NodeHandle {
    /// Send `request` to the B/IP address `target` and wait up to `timeout`
    /// after the send for the response that answers it.
    /// With `give_up`, the exchange is admitted through it, so that it can
    /// be given up from elsewhere.
    pub(super) async fn request(
        self: &Arc<Self>,
        target: &[u8],
        request: Bvll<'_>,
        timeout: Duration,
        give_up: Option<&BvlcGiveUp>,
    ) -> Result<BvllMessage, BvlcExchangeError> {
        let (ip, port) = decode_bip_mac(target)?;
        let peer = SocketAddrV4::new(Ipv4Addr::from(ip), port);
        let (exchange, response) = match give_up {
            Some(give_up) => give_up.begin(self, peer, request, timeout)?,
            None => self.begin(peer, request, timeout)?,
        };
        let mut buf = BytesMut::new();
        if let Err(error) = encode_bvll(&mut buf, &request) {
            exchange.failed();
            return Err(error.into());
        }
        // As every send goes (#2617): it fails once the node has stopped,
        // and one the socket makes wait holds no socket.
        let sent = self
            .send_now(|_| Ok((OutgoingDatagram::whole(&buf), peer)))
            .await;
        if let Err(error) = sent {
            exchange.failed();
            return Err(error.into());
        }
        exchange.sent();
        match response.await {
            Ok(outcome) => Ok(outcome?),
            // Its waiter went without an answer: it was given up, or the
            // node dropped it as the transport stopped (#2612, #2617). The
            // request had gone out, and a change it asked for (a new BDT,
            // an FDT entry deleted) may have been made, so it is cut off as
            // sent, not refused as one after the stop is (#1833).
            Err(_) if give_up.is_some_and(BvlcGiveUp::is_given_up) => Err(given_up()),
            Err(_) => Err(BvlcExchangeError::Transport(cut_off_as_sent())),
        }
    }

    /// Admit a manual exchange of `request` with `peer`.
    pub(super) fn begin(
        self: &Arc<Self>,
        peer: SocketAddrV4,
        request: Bvll<'_>,
        timeout: Duration,
    ) -> Result<(Exchange, oneshot::Receiver<Outcome>), BvlcExchangeError> {
        let (tx, rx) = oneshot::channel();
        let id = self.with(|state, now| {
            if state.stopped {
                return Err(BvlcExchangeError::Transport(crate::port::stopped()));
            }
            let (id, _) = state
                .node
                // A timeout past some 584 years is one that never ends.
                .begin_exchange(peer, request, CoreDuration::saturating_from_core(timeout), now)
                .map_err(BvlcExchangeError::Refused)?;
            let waiter = Waiter {
                tx,
                timeout,
                early: None,
            };
            state.waiters.insert(id, waiter);
            Ok(id)
        })?;
        let exchange = Exchange {
            node: Arc::clone(self),
            id,
        };
        Ok((exchange, rx))
    }
}

impl Exchange {
    /// The request went out: its timeout starts, and a response that beat
    /// the send is handed over now.
    pub(super) fn sent(&self) {
        self.node.with(|state, now| {
            if !state.node.exchange_sent(self.id, now) {
                return;
            }
            let early = state
                .waiters
                .get_mut(&self.id)
                .and_then(|waiter| waiter.early.take());
            // The request went out and its early answer was lost: what it
            // did is unknown, so it is cut off as sent (#1833).
            let outcome = early.unwrap_or_else(|| Err(cut_off_as_sent()));
            state.complete(self.id, outcome);
        });
    }

    /// Encoding or sending the request failed.
    pub(super) fn failed(&self) {
        self.end(LocalFailure::SendFailed);
    }

    fn end(&self, why: LocalFailure) {
        self.node.end_exchange(self.id, why);
    }
}

impl NodeHandle {
    /// End `id` on this side: its waiter goes, and the node frees its slot.
    /// An exchange that already ended is left as it is.
    fn end_exchange(&self, id: ExchangeId, why: LocalFailure) {
        self.with(|state, now| {
            state.waiters.remove(&id);
            state.node.exchange_failed(id, why, now);
        });
    }
}

/// Gives up one [`BvlcTableReader`] read from outside the task that runs it
/// (#2612). Dropping the read's future gives it up too, but a bridge whose
/// caller cancels before the task is dropped, as an asyncio cancellation
/// does, needs the request's slot free at once: a read sent right after it
/// would find the same request still in flight and be refused. Give one
/// read to [`BvlcTableReader::with_give_up`]; [`give_up`](Self::give_up)
/// ends it then, or keeps it from starting.
///
/// A read given up, waiting for its answer or not yet started, fails with
/// [`BvlcExchangeError::Transport`] carrying an [`Error::State`] with
/// [`Lifecycle::Closed`] that says it was given up; one that had already
/// ended keeps its own outcome.
#[derive(Clone, Debug, Default)]
pub struct BvlcGiveUp(Arc<Mutex<GiveUp>>);

/// What an exchange whose request went out gets when the transport's stop
/// drops its waiter, or its answer is lost (#1833): the request may have
/// been carried out.
#[cold]
#[inline(never)]
fn cut_off_as_sent() -> Error {
    Error::RequestCutOff { sent: true }
}

/// The error of a read given up through its [`BvlcGiveUp`].
fn given_up() -> BvlcExchangeError {
    BvlcExchangeError::Transport(Error::state(Lifecycle::Closed, "BVLC table read given up"))
}

/// Where a [`BvlcGiveUp`]'s read is.
#[derive(Debug, Default)]
enum GiveUp {
    /// Not admitted yet.
    #[default]
    Waiting,
    /// Admitted as this exchange of this node.
    Admitted(Weak<NodeHandle>, ExchangeId),
    /// Given up: a read not admitted yet is refused.
    GivenUp,
}

impl BvlcGiveUp {
    /// Give the read up: end its exchange now, or refuse it if it hasn't
    /// started. One that already ended is left as it is.
    pub fn give_up(&self) {
        let state = std::mem::replace(&mut *self.lock(), GiveUp::GivenUp);
        if let GiveUp::Admitted(node, id) = state
            && let Some(node) = node.upgrade()
        {
            node.end_exchange(id, LocalFailure::Cancelled);
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, GiveUp> {
        self.0.locked()
    }

    /// Whether the read was given up.
    fn is_given_up(&self) -> bool {
        matches!(*self.lock(), GiveUp::GivenUp)
    }

    /// Admit the read through `node`, unless it was given up. The slot's
    /// lock is held across the admission, so a give-up can't fall between.
    fn begin(
        &self,
        node: &Arc<NodeHandle>,
        peer: SocketAddrV4,
        request: Bvll<'_>,
        timeout: Duration,
    ) -> Result<(Exchange, oneshot::Receiver<Outcome>), BvlcExchangeError> {
        let mut state = self.lock();
        if matches!(*state, GiveUp::GivenUp) {
            return Err(given_up());
        }
        let (exchange, response) = node.begin(peer, request, timeout)?;
        *state = GiveUp::Admitted(Arc::downgrade(node), exchange.id);
        Ok((exchange, response))
    }
}

impl Drop for Exchange {
    /// Give the exchange up, unless it already ended.
    fn drop(&mut self) {
        self.end(LocalFailure::Cancelled);
    }
}

impl BipTransport {
    /// Send a BVLC management request and await the response.
    async fn bvlc_request(
        &self,
        target: &[u8],
        request: Bvll<'_>,
    ) -> Result<BvllMessage, BvlcExchangeError> {
        let node = Arc::clone(self.require_node()?);
        node.request(target, request, Self::BVLC_RESPONSE_TIMEOUT, None)
            .await
    }

    /// Snapshot this transport's own manual BVLC and automatic foreign-device
    /// exchanges. Counts are cumulative per instance; registration status,
    /// its next-attempt countdown and its lease reset on stop/start. This
    /// never polls a BBMD: the lease is a conservative local view of what the
    /// BBMD's successes prove. See [`BvlcClientSnapshot`] for matching limits
    /// and the meaning of each observation.
    pub fn bvlc_client_snapshot(&self) -> BvlcClientSnapshot {
        self.client_record.live_snapshot()
    }

    /// A handle that reads [`Self::bvlc_client_snapshot`] after this
    /// transport is handed to an owner that keeps it out of reach, as an
    /// endpoint session does.
    pub fn bvlc_client_status(&self) -> BvlcClientStatus {
        BvlcClientStatus(Arc::clone(&self.client_record))
    }

    /// A handle that sends [`Self::read_bdt`] and [`Self::read_fdt`] through
    /// this transport after it is handed to an owner that keeps it out of
    /// reach, as an endpoint session does (#2612).
    pub fn bvlc_table_reader(&self) -> BvlcTableReader {
        BvlcTableReader {
            record: Arc::clone(&self.client_record),
            give_up: None,
        }
    }

    /// Send Read-Broadcast-Distribution-Table and return the response entries.
    ///
    /// A peer that refuses the read, as one that is no BBMD does, fails it
    /// with [`BvlcExchangeError::Nak`]; one that stays silent, with
    /// [`BvlcExchangeError::NoReply`].
    pub async fn read_bdt(&self, target: &[u8]) -> Result<Vec<BdtEntry>, BvlcExchangeError> {
        read_bdt(self.require_node()?, target, None).await
    }

    /// Send Write-Broadcast-Distribution-Table; `Ok` when the peer answers
    /// with success.
    ///
    /// Outbound client helper only. A conforming 135-2020 receiver answers
    /// with the not-supported result, [`BvlcExchangeError::Nak`] here, and
    /// leaves its table unchanged.
    pub async fn write_bdt(
        &self,
        target: &[u8],
        entries: &[BdtEntry],
    ) -> Result<(), BvlcExchangeError> {
        let rows: Vec<codec::BdtEntry> = entries.iter().map(codec::BdtEntry::from).collect();
        let msg = self
            .bvlc_request(
                target,
                Bvll::WriteBroadcastDistributionTable(BdtEntries::from_entries(&rows)),
            )
            .await?;
        acknowledged(&msg)
    }

    /// Send Read-Foreign-Device-Table and return the response entries.
    ///
    /// It fails as [`read_bdt`](Self::read_bdt) does.
    pub async fn read_fdt(&self, target: &[u8]) -> Result<Vec<FdtEntryWire>, BvlcExchangeError> {
        read_fdt(self.require_node()?, target, None).await
    }

    /// Send Delete-Foreign-Device-Table-Entry for `ip`:`port` to `target`;
    /// `Ok` when it answers with success, [`BvlcExchangeError::Nak`] when it
    /// refuses.
    pub async fn delete_fdt_entry(
        &self,
        target: &[u8],
        ip: [u8; 4],
        port: u16,
    ) -> Result<(), BvlcExchangeError> {
        let entry = SocketAddrV4::new(Ipv4Addr::from(ip), port);
        let msg = self
            .bvlc_request(target, Bvll::DeleteForeignDeviceTableEntry(entry))
            .await?;
        acknowledged(&msg)
    }

    /// Send a Register-Foreign-Device BVLC message to a BBMD; `Ok` when it
    /// accepts, [`BvlcExchangeError::Nak`] when it refuses.
    ///
    /// This is a low-level BVLC management operation. It does NOT configure this
    /// transport as a foreign device for broadcast behavior (use
    /// [`register_as_foreign_device`](Self::register_as_foreign_device) before `start()` for that).
    /// A zero TTL is passed through for a one-shot removal request; automatic
    /// mode's positive-TTL validation does not apply to this helper.
    pub async fn register_foreign_device_bvlc(
        &self,
        target: &[u8],
        ttl: u16,
    ) -> Result<(), BvlcExchangeError> {
        let msg = self
            .bvlc_request(target, Bvll::RegisterForeignDevice { ttl })
            .await?;
        acknowledged(&msg)
    }
}

/// Reads a BBMD's two tables, its BDT and its FDT, through one B/IP
/// transport wherever the transport went, for an owner that takes it over,
/// as an endpoint session does (#2612).
///
/// It sends what [`BipTransport::read_bdt`] and [`BipTransport::read_fdt`]
/// send, through the transport's own socket and node, beside the transport's
/// other traffic, and each read fails as theirs does, counted in the same
/// [`BvlcClientSnapshot`]. While the transport is stopped, a read fails as
/// one before start does. It only reads: the table writes and the
/// registration stay on the transport. Cloning it is cheap.
#[derive(Clone, Debug)]
pub struct BvlcTableReader {
    record: Arc<super::ClientRecord>,
    give_up: Option<BvlcGiveUp>,
}

impl BvlcTableReader {
    /// This reader, whose reads `give_up` can give up: one read for each.
    #[must_use]
    pub fn with_give_up(&self, give_up: &BvlcGiveUp) -> Self {
        Self {
            record: Arc::clone(&self.record),
            give_up: Some(give_up.clone()),
        }
    }

    /// Send Read-Broadcast-Distribution-Table to the B/IP MAC `target`, as
    /// [`BipTransport::read_bdt`] does.
    pub async fn read_bdt(&self, target: &[u8]) -> Result<Vec<BdtEntry>, BvlcExchangeError> {
        read_bdt(&self.node()?, target, self.give_up.as_ref()).await
    }

    /// Send Read-Foreign-Device-Table to the B/IP MAC `target`, as
    /// [`BipTransport::read_fdt`] does.
    pub async fn read_fdt(&self, target: &[u8]) -> Result<Vec<FdtEntryWire>, BvlcExchangeError> {
        read_fdt(&self.node()?, target, self.give_up.as_ref()).await
    }

    /// The transport's running node.
    fn node(&self) -> Result<Arc<NodeHandle>, Error> {
        self.record.node().ok_or_else(super::not_started)
    }
}

/// One Read-Broadcast-Distribution-Table exchange through `node`.
async fn read_bdt(
    node: &Arc<NodeHandle>,
    target: &[u8],
    give_up: Option<&BvlcGiveUp>,
) -> Result<Vec<BdtEntry>, BvlcExchangeError> {
    let msg = node
        .request(
            target,
            Bvll::ReadBroadcastDistributionTable,
            BipTransport::BVLC_RESPONSE_TIMEOUT,
            give_up,
        )
        .await?;
    match msg.message()? {
        Bvll::ReadBroadcastDistributionTableAck(table) => Ok(bbmd::bdt_from_wire(table)?),
        Bvll::BvlcResult(code) => Err(result_error(
            code,
            BvlcFunction::READ_BROADCAST_DISTRIBUTION_TABLE_ACK,
        )),
        other => Err(unexpected_response(
            BvlcFunction::READ_BROADCAST_DISTRIBUTION_TABLE_ACK,
            other.function(),
        )),
    }
}

/// One Read-Foreign-Device-Table exchange through `node`.
async fn read_fdt(
    node: &Arc<NodeHandle>,
    target: &[u8],
    give_up: Option<&BvlcGiveUp>,
) -> Result<Vec<FdtEntryWire>, BvlcExchangeError> {
    let msg = node
        .request(
            target,
            Bvll::ReadForeignDeviceTable,
            BipTransport::BVLC_RESPONSE_TIMEOUT,
            give_up,
        )
        .await?;
    match msg.message()? {
        Bvll::ReadForeignDeviceTableAck(table) => Ok(bbmd::fdt_from_wire(table)?),
        Bvll::BvlcResult(code) => Err(result_error(
            code,
            BvlcFunction::READ_FOREIGN_DEVICE_TABLE_ACK,
        )),
        other => Err(unexpected_response(
            BvlcFunction::READ_FOREIGN_DEVICE_TABLE_ACK,
            other.function(),
        )),
    }
}

/// The error for a response of the wrong function.
pub(super) fn unexpected_response(expected: BvlcFunction, got: BvlcFunction) -> BvlcExchangeError {
    BvlcExchangeError::UnexpectedResponse { expected, got }
}

/// The error for a BVLC-Result answering a request that takes `expected`:
/// a NAK, or a success, which answers no such request.
pub(super) fn result_error(code: BvlcResultCode, expected: BvlcFunction) -> BvlcExchangeError {
    if code == BvlcResultCode::SUCCESSFUL_COMPLETION {
        unexpected_response(expected, BvlcFunction::BVLC_RESULT)
    } else {
        BvlcExchangeError::Nak(code)
    }
}

/// The outcome of a request a BVLC-Result answers: success, or the NAK.
fn acknowledged(msg: &BvllMessage) -> Result<(), BvlcExchangeError> {
    match decode_bvlc_result_code(msg)? {
        BvlcResultCode::SUCCESSFUL_COMPLETION => Ok(()),
        code => Err(BvlcExchangeError::Nak(code)),
    }
}

/// The result code of a BVLC-Result, or why `msg` isn't a well-formed one.
pub(super) fn decode_bvlc_result_code(
    msg: &BvllMessage,
) -> Result<BvlcResultCode, BvlcExchangeError> {
    if msg.function != BvlcFunction::BVLC_RESULT {
        return Err(unexpected_response(BvlcFunction::BVLC_RESULT, msg.function));
    }
    match msg.message()? {
        Bvll::BvlcResult(code) => Ok(code),
        other => Err(unexpected_response(
            BvlcFunction::BVLC_RESULT,
            other.function(),
        )),
    }
}
