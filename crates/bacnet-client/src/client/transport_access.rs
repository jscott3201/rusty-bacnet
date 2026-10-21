//! The transport of a built client, and the BBMD helpers that reach through it.

use bacnet_transport::bbmd::{BdtEntry, FdtEntryWire};
use bacnet_transport::bip::{AsBip, BvlcExchangeError, ForeignRegistrationSnapshot};

use super::*;

impl<T: TransportPort + 'static> BACnetClient<T> {
    /// The transport this client was built with.
    ///
    /// The client keeps its transport inside the network layer that its
    /// dispatch task shares, and lends it out only as a shared borrow.
    /// Methods that take `&mut self`, such as `start`, `stop` and the
    /// configuration setters, stay out of reach; [`stop`](Self::stop) stops
    /// the transport in place.
    ///
    /// # Supported through this borrow
    ///
    /// Reading transport state and diagnostics while the client runs:
    ///
    /// - BACnet/SC: the `connection_state_changes()` watch, the
    ///   `connection_events()` broadcast (reconnects, failover, primary
    ///   restore, disconnect causes), the `connection_status()` snapshot
    ///   (active hub, hub VMAC, heartbeat age and round-trip time),
    ///   `hub_peer_info()` and `npdu_drop_counts()`.
    /// - B/IP: `management_counters()`, `fdt_counters()`,
    ///   `fanout_counters()`, `receive_counters()`, `send_counters()`, and
    ///   `bbmd_state()` for a brief read of the BBMD tables.
    /// - MS/TP: the
    ///   [`diagnostics()`](bacnet_transport::mstp::MstpTransport::diagnostics)
    ///   handle.
    /// - B/IP, B/IPv6, BACnet/SC and MS/TP: `frame_tap()`, to subscribe to
    ///   every link-layer frame the transport sends and receives (#957).
    /// - Any transport: `local_mac()` and the APDU size limits.
    ///
    /// The watch receiver, the diagnostics handle and the BBMD state `Arc` are
    /// owned and outlive the borrow, so a long-lived UI takes them once and
    /// moves them into its own tasks, as MS/TP callers already do with
    /// `diagnostics()` before handing a transport over. The counters are cheap
    /// snapshots: poll them through the borrow as often as the UI redraws.
    ///
    /// # Not supported while the client runs
    ///
    /// These go around the client, which does not guard against them:
    ///
    /// - Sending through the transport (`send_unicast`, `send_broadcast` and
    ///   their variants). The client's transaction state machine never sees
    ///   the frame, so a hand-built confirmed request can reuse an invoke ID
    ///   that is already in flight. Send through the client's request methods,
    ///   and use its BBMD helpers for BBMD management.
    /// - Holding the `bbmd_state()` lock across an await or a client call.
    ///   It is a synchronous mutex the receive loop and a broadcast from the
    ///   client take too, so holding it stalls the transport or deadlocks.
    ///   Lock, copy what you need and release.
    ///
    /// The live MS/TP master node and the BACnet/SC connection, whose locks
    /// the token loop and the hub link take, are not public at all.
    ///
    /// # Why a borrow
    ///
    /// This is one accessor for every transport, rather than a bundle of
    /// handles returned by each builder. Each transport already offers its
    /// diagnostics on `&self`, so nothing new is needed per transport, and
    /// clients from [`generic_builder`](Self::generic_builder) get the same
    /// access. Sharing the transport through an `Arc` was ruled out too:
    /// [`stop`](Self::stop) needs sole ownership of the network layer to stop
    /// the transport, which an outstanding clone would block.
    ///
    /// ```
    /// use std::net::Ipv4Addr;
    /// use bacnet_client::client::BACnetClient;
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    /// runtime.block_on(Box::pin(async {
    ///     let mut client = BACnetClient::bip_builder()
    ///         .interface(Ipv4Addr::LOCALHOST)
    ///         .port(0)
    ///         .build()
    ///         .await?;
    ///     // Counter snapshots: poll them whenever the UI redraws.
    ///     let counters = client.transport().management_counters();
    ///     assert_eq!(counters.read_bdt_responses, 0);
    ///     assert_eq!(client.transport().fanout_counters().packets_forwarded, 0);
    ///     // This client is not a BBMD, so it has no BBMD state or FDT.
    ///     assert!(client.transport().bbmd_state().is_none());
    ///     assert!(client.transport().fdt_counters().is_none());
    ///     client.stop().await?;
    ///     Ok::<_, bacnet_types::error::Error>(())
    /// }))?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn transport(&self) -> &T {
        self.network.transport()
    }
}

/// BBMD management helpers (Annex J), for a client whose transport is B/IP.
///
/// They work on a client over [`BipTransport`], and on one over
/// [`AnyTransport`](bacnet_transport::any::AnyTransport) when its variant is
/// `Bip`. Each request fails with a [`BvlcExchangeError`]: a NAK, no reply,
/// or a local failure. On any other variant each returns
/// [`Error::UnsupportedTransport`], as [`BvlcExchangeError::Transport`],
/// before sending anything.
///
/// ```
/// use bacnet_client::client::BACnetClient;
/// use bacnet_transport::any::AnyTransport;
/// use bacnet_transport::bip::BvlcExchangeError;
/// use bacnet_transport::loopback::LoopbackTransport;
/// use bacnet_transport::mstp::NoSerial;
/// use bacnet_types::data_link::DataLink;
/// use bacnet_types::error::Error;
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
/// runtime.block_on(Box::pin(async {
///     let (ours, _peer) = LoopbackTransport::pair(vec![1], vec![2]);
///     let mut client = BACnetClient::generic_builder()
///         .transport(AnyTransport::<NoSerial>::Loopback(ours))
///         .build()
///         .await?;
///     let bbmd = [127, 0, 0, 1, 0xBA, 0xC0];
///     assert!(matches!(
///         client.read_bdt(&bbmd).await,
///         Err(BvlcExchangeError::Transport(Error::UnsupportedTransport {
///             required: DataLink::Bip,
///             actual: DataLink::Loopback,
///         }))
///     ));
///     client.stop().await?;
///     Ok::<_, Error>(())
/// }))?;
/// # Ok(())
/// # }
/// ```
impl<T: TransportPort + AsBip + 'static> BACnetClient<T> {
    /// How this client's foreign-device registration is going (#938): whether
    /// it holds a live lease, the time left on it, and the last attempt's
    /// result and outcome, a BBMD's refusal included. Outside
    /// [`BipMode::Foreign`] only manual registrations show. It reads
    /// [`BipTransport::bvlc_client_snapshot`] and sends nothing.
    pub fn foreign_registration(&self) -> Result<ForeignRegistrationSnapshot, Error> {
        Ok(self
            .transport()
            .as_bip()?
            .bvlc_client_snapshot()
            .foreign_registration)
    }

    /// Read the Broadcast Distribution Table from a BBMD.
    pub async fn read_bdt(&self, target: &[u8]) -> Result<Vec<BdtEntry>, BvlcExchangeError> {
        self.transport().as_bip()?.read_bdt(target).await
    }

    /// Write the Broadcast Distribution Table to a BBMD.
    ///
    /// A BBMD that follows the 2020 standard answers with the not-supported
    /// result, [`BvlcExchangeError::Nak`], and keeps its table.
    pub async fn write_bdt(
        &self,
        target: &[u8],
        entries: &[BdtEntry],
    ) -> Result<(), BvlcExchangeError> {
        self.transport().as_bip()?.write_bdt(target, entries).await
    }

    /// Read the Foreign Device Table from a BBMD.
    pub async fn read_fdt(&self, target: &[u8]) -> Result<Vec<FdtEntryWire>, BvlcExchangeError> {
        self.transport().as_bip()?.read_fdt(target).await
    }

    /// Delete a Foreign Device Table entry on a BBMD.
    pub async fn delete_fdt_entry(
        &self,
        target: &[u8],
        ip: [u8; 4],
        port: u16,
    ) -> Result<(), BvlcExchangeError> {
        self.transport()
            .as_bip()?
            .delete_fdt_entry(target, ip, port)
            .await
    }

    /// Register as a foreign device with a BBMD; `Ok` when it accepts,
    /// [`BvlcExchangeError::Nak`] when it refuses.
    ///
    /// This sends one registration. It does not make the transport a
    /// foreign device for broadcasts or renew the registration.
    pub async fn register_foreign_device_bvlc(
        &self,
        target: &[u8],
        ttl: u16,
    ) -> Result<(), BvlcExchangeError> {
        self.transport()
            .as_bip()?
            .register_foreign_device_bvlc(target, ttl)
            .await
    }
}
