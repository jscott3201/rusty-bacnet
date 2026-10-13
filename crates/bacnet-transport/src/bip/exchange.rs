//! This transport's own management requests (`read_bdt` and the other
//! helpers): one exchange with the node each, from admission to the
//! response, a timeout, a failed send or the caller giving up.

use std::net::{Ipv4Addr, SocketAddrV4};
use std::sync::Arc;
use std::time::Duration;

use bacnet_bip_codec::{self as codec, BdtEntries, BvlcFunction, BvlcResultCode, Bvll};
use bacnet_bip_core::node::{ExchangeId, ExchangeRefusal, LocalFailure};
use bacnet_bip_core::Duration as CoreDuration;
use bacnet_types::error::Error;
use bytes::BytesMut;
use tokio::sync::oneshot;

use super::node::{NodeHandle, Outcome, Waiter};
use super::{BipTransport, BvlcClientSnapshot, BvlcClientStatus};
use crate::bbmd::{self, BdtEntry, FdtEntryWire};
use crate::bvll::{decode_bip_mac, encode_bvll, BvllMessage};

/// A manual exchange admitted by the node. Dropping it before it ends
/// gives it up, as when the caller stops waiting.
pub(super) struct Exchange {
    node: Arc<NodeHandle>,
    id: ExchangeId,
}

impl NodeHandle {
    /// Send `request` to the B/IP address `target` and wait up to `timeout`
    /// after the send for the response that answers it.
    pub(super) async fn request(
        self: &Arc<Self>,
        target: &[u8],
        request: Bvll<'_>,
        timeout: Duration,
    ) -> Result<BvllMessage, Error> {
        let (ip, port) = decode_bip_mac(target)?;
        let peer = SocketAddrV4::new(Ipv4Addr::from(ip), port);
        let (exchange, response) = self.begin(peer, request, timeout)?;
        let mut buf = BytesMut::new();
        if let Err(error) = encode_bvll(&mut buf, &request) {
            exchange.failed();
            return Err(error);
        }
        if let Err(error) = self.socket().send_to(&buf, peer).await {
            exchange.failed();
            return Err(Error::Transport(error));
        }
        exchange.sent();
        match response.await {
            Ok(outcome) => outcome,
            Err(_) => Err(Error::Encoding("BVLC response channel dropped".into())),
        }
    }

    /// Admit a manual exchange of `request` with `peer`.
    pub(super) fn begin(
        self: &Arc<Self>,
        peer: SocketAddrV4,
        request: Bvll<'_>,
        timeout: Duration,
    ) -> Result<(Exchange, oneshot::Receiver<Outcome>), Error> {
        let (tx, rx) = oneshot::channel();
        let id = self.with(|state, now| {
            if state.stopped {
                return Err(Error::Encoding("B/IP management client is stopped".into()));
            }
            let (id, _) = state
                .node
                // A timeout past some 584 years is one that never ends.
                .begin_exchange(peer, request, CoreDuration::saturating_from_core(timeout), now)
                .map_err(refusal_error)?;
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
            let outcome =
                early.unwrap_or_else(|| Err(Error::Encoding("BVLC response lost".into())));
            state.complete(self.id, outcome);
        });
    }

    /// Encoding or sending the request failed.
    pub(super) fn failed(&self) {
        self.end(LocalFailure::SendFailed);
    }

    fn end(&self, why: LocalFailure) {
        self.node.with(|state, now| {
            state.waiters.remove(&self.id);
            state.node.exchange_failed(self.id, why, now);
        });
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
    async fn bvlc_request(&self, target: &[u8], request: Bvll<'_>) -> Result<BvllMessage, Error> {
        let node = Arc::clone(self.require_node()?);
        node.request(target, request, Self::BVLC_RESPONSE_TIMEOUT)
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

    /// Send Read-Broadcast-Distribution-Table and return the response entries.
    pub async fn read_bdt(&self, target: &[u8]) -> Result<Vec<BdtEntry>, Error> {
        let msg = self
            .bvlc_request(target, Bvll::ReadBroadcastDistributionTable)
            .await?;
        match msg.message()? {
            Bvll::BvlcResult(code) => Err(Error::Encoding(format!("BVLC-Result: {code:?}"))),
            Bvll::ReadBroadcastDistributionTableAck(table) => bbmd::bdt_from_wire(table),
            other => Err(unexpected_response(
                BvlcFunction::READ_BROADCAST_DISTRIBUTION_TABLE_ACK,
                other.function(),
            )),
        }
    }

    /// Send Write-Broadcast-Distribution-Table and return the result code.
    ///
    /// Outbound client helper only. A conforming 135-2020 receiver answers
    /// with the not-supported result and leaves its table unchanged.
    pub async fn write_bdt(
        &self,
        target: &[u8],
        entries: &[BdtEntry],
    ) -> Result<BvlcResultCode, Error> {
        let rows: Vec<codec::BdtEntry> = entries.iter().map(codec::BdtEntry::from).collect();
        let msg = self
            .bvlc_request(
                target,
                Bvll::WriteBroadcastDistributionTable(BdtEntries::from_entries(&rows)),
            )
            .await?;
        decode_bvlc_result_code(&msg)
    }

    /// Send Read-Foreign-Device-Table and return the response entries.
    pub async fn read_fdt(&self, target: &[u8]) -> Result<Vec<FdtEntryWire>, Error> {
        let msg = self
            .bvlc_request(target, Bvll::ReadForeignDeviceTable)
            .await?;
        match msg.message()? {
            Bvll::BvlcResult(code) => Err(Error::Encoding(format!("BVLC-Result: {code:?}"))),
            Bvll::ReadForeignDeviceTableAck(table) => bbmd::fdt_from_wire(table),
            other => Err(unexpected_response(
                BvlcFunction::READ_FOREIGN_DEVICE_TABLE_ACK,
                other.function(),
            )),
        }
    }

    /// Send Delete-Foreign-Device-Table-Entry for `ip`:`port` to `target`, and
    /// return the BVLC-Result code it answers with.
    pub async fn delete_fdt_entry(
        &self,
        target: &[u8],
        ip: [u8; 4],
        port: u16,
    ) -> Result<BvlcResultCode, Error> {
        let entry = SocketAddrV4::new(Ipv4Addr::from(ip), port);
        let msg = self
            .bvlc_request(target, Bvll::DeleteForeignDeviceTableEntry(entry))
            .await?;
        decode_bvlc_result_code(&msg)
    }

    /// Send a Register-Foreign-Device BVLC message to a BBMD and return the result code.
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
    ) -> Result<BvlcResultCode, Error> {
        let msg = self
            .bvlc_request(target, Bvll::RegisterForeignDevice { ttl })
            .await?;
        decode_bvlc_result_code(&msg)
    }
}

fn refusal_error(refusal: ExchangeRefusal) -> Error {
    match refusal {
        ExchangeRefusal::SameExchange | ExchangeRefusal::ResultClass => Error::Encoding(format!(
            "BVLC management request already in flight: {refusal}"
        )),
        // Full, ManagedRegistration, NotARequest, and any later refusal.
        _ => Error::Encoding(format!("BVLC management request refused: {refusal}")),
    }
}

/// The error for a response of the wrong function.
pub(super) fn unexpected_response(expected: BvlcFunction, got: BvlcFunction) -> Error {
    Error::Encoding(format!("expected BVLC response {expected:?}, got {got:?}"))
}

/// The result code of a BVLC-Result, or why `msg` isn't a well-formed one.
pub(super) fn decode_bvlc_result_code(msg: &BvllMessage) -> Result<BvlcResultCode, Error> {
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
