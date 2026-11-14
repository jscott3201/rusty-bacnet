//! BBMD table reads through the engine, under the rate cap: the read side
//! of BVLL only (Annex J.2.3 and J.2.7), never a table write or a
//! registration.

use std::net::SocketAddrV4;

use bacnet_ops::engine::Engine;
use bacnet_transport::bip::{BipTransport, BvlcExchangeError};
use bacnet_transport::port::TransportPort;

use super::{BdtRow, FdtRow, Outcome, Probe, mac};

/// The B/IP transport of `engine`'s client, when it has one: the BBMD
/// reads need it.
pub fn bip<T: TransportPort + 'static>(engine: &Engine<T>) -> Option<&BipTransport> {
    engine.client().transport().bip_transport()
}

/// The BBMD `engine`'s client last tried to register with as a foreign
/// device, if any. Reads the transport's registration state and sends
/// nothing.
pub fn registration_bbmd<T: TransportPort + 'static>(engine: &Engine<T>) -> Option<SocketAddrV4> {
    bip(engine)?
        .bvlc_client_snapshot()
        .foreign_registration
        .last_bbmd
}

/// Read `bbmd`'s BDT through `engine`, after a slot under the local
/// network's rate and the global one. On a client without B/IP it sends
/// nothing.
pub async fn read_bdt<T: TransportPort + 'static>(engine: &Engine<T>, bbmd: SocketAddrV4) -> Probe {
    let Some(bip) = bip(engine) else {
        return Outcome::Unreachable(NOT_BIP.into());
    };
    engine.limits().slot(Some(0)).await;
    settle(bip.read_bdt(&mac(bbmd)).await, |e| BdtRow::from(e))
}

/// Read `bbmd`'s FDT, as [`read_bdt`].
pub async fn read_fdt<T: TransportPort + 'static>(
    engine: &Engine<T>,
    bbmd: SocketAddrV4,
) -> Outcome<Vec<FdtRow>> {
    let Some(bip) = bip(engine) else {
        return Outcome::Unreachable(NOT_BIP.into());
    };
    engine.limits().slot(Some(0)).await;
    settle(bip.read_fdt(&mac(bbmd)).await, |e| FdtRow::from(e))
}

/// Why a client without B/IP reads no BBMD.
const NOT_BIP: &str = "BBMD tables need BACnet/IP";

/// A table read's outcome. A request the node refused before sending
/// never left, so the call isn't charged for it.
fn settle<E, R>(read: Result<Vec<E>, BvlcExchangeError>, row: fn(&E) -> R) -> Outcome<Vec<R>> {
    match read {
        Ok(entries) => Outcome::Table(entries.iter().map(row).collect()),
        Err(error) => {
            if matches!(error, BvlcExchangeError::Refused(_)) {
                bacnet_ops::limits::refund();
            }
            Outcome::from_error(&error)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use bacnet_client::client::{BACnetClient, ClientConfig};
    use bacnet_ops::limits::RequestLimits;
    use bacnet_transport::loopback::LoopbackTransport;

    use super::*;

    /// The BBMD reads follow the link under a wrapper: an engine over the mixed
    /// transport holding B/IP reaches the B/IP transport, and one holding
    /// loopback sends nothing (#2560).
    #[tokio::test]
    async fn the_bbmd_reads_see_through_the_mixed_transport() {
        use bacnet_transport::any::AnyTransport;
        use std::net::Ipv4Addr;

        let engine_on = async |transport: AnyTransport| {
            let client = BACnetClient::start(ClientConfig::default(), transport)
                .await
                .unwrap();
            Engine::new(Arc::new(client), Arc::new(RequestLimits::new(0)))
        };
        let bip_port = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
        let engine = engine_on(bip_port.into()).await;
        assert!(bip(&engine).is_some());
        assert_eq!(registration_bbmd(&engine), None);

        let (loopback, _peer) = LoopbackTransport::pair(vec![1], vec![2]);
        let engine = engine_on(loopback.into()).await;
        assert!(bip(&engine).is_none());
        let bbmd = "10.0.0.2:47808".parse().unwrap();
        assert!(matches!(
            read_bdt(&engine, bbmd).await,
            Outcome::Unreachable(_)
        ));
    }
}
