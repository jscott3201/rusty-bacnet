//! Each BVLC management helper reports a NAK, silence and a local failure as
//! their own [`BvlcExchangeError`] variants (#1822).

use super::*;

/// One of the five public BVLC management helpers.
#[derive(Debug, Clone, Copy)]
enum Helper {
    ReadBdt,
    WriteBdt,
    ReadFdt,
    DeleteFdtEntry,
    RegisterForeignDevice,
}

impl Helper {
    const ALL: [Self; 5] = [
        Self::ReadBdt,
        Self::WriteBdt,
        Self::ReadFdt,
        Self::DeleteFdtEntry,
        Self::RegisterForeignDevice,
    ];

    /// The request it sends.
    fn function(self) -> BvlcFunction {
        match self {
            Self::ReadBdt => BvlcFunction::READ_BROADCAST_DISTRIBUTION_TABLE,
            Self::WriteBdt => BvlcFunction::WRITE_BROADCAST_DISTRIBUTION_TABLE,
            Self::ReadFdt => BvlcFunction::READ_FOREIGN_DEVICE_TABLE,
            Self::DeleteFdtEntry => BvlcFunction::DELETE_FOREIGN_DEVICE_TABLE_ENTRY,
            Self::RegisterForeignDevice => BvlcFunction::REGISTER_FOREIGN_DEVICE,
        }
    }

    /// Run it against `target`, keeping only how it failed.
    async fn call(self, transport: &BipTransport, target: &[u8]) -> Result<(), BvlcExchangeError> {
        match self {
            Self::ReadBdt => transport.read_bdt(target).await.map(drop),
            Self::WriteBdt => transport.write_bdt(target, &[]).await,
            Self::ReadFdt => transport.read_fdt(target).await.map(drop),
            Self::DeleteFdtEntry => {
                transport
                    .delete_fdt_entry(target, [10, 0, 0, 2], 0xBAC0)
                    .await
            }
            Self::RegisterForeignDevice => transport.register_foreign_device_bvlc(target, 60).await,
        }
    }
}

async fn peer() -> (UdpSocket, [u8; 6]) {
    let peer = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let mac = encode_bip_mac(
        Ipv4Addr::LOCALHOST.octets(),
        peer.local_addr().unwrap().port(),
    );
    (peer, mac)
}

async fn started() -> BipTransport {
    let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let _rx = transport.start().await.unwrap();
    transport
}

/// A peer that refuses each request fails it with `Nak` carrying the code
/// it sent, not with a transport error.
#[tokio::test]
async fn each_helper_reports_a_nak_as_the_nak_variant() {
    let (peer, target) = peer().await;
    let mut transport = started().await;
    for helper in Helper::ALL {
        let code = helper.function().nak().unwrap();
        let request = helper.call(&transport, &target);
        tokio::pin!(request);
        let mut packet = [0; 64];
        let (_, source) = tokio::select! {
            result = &mut request => panic!("{helper:?} ended before a response: {result:?}"),
            received = peer.recv_from(&mut packet) => received.unwrap(),
        };
        assert_eq!(packet[1], helper.function().to_raw(), "{helper:?}");
        let [high, low] = code.to_raw().to_be_bytes();
        peer.send_to(&[0x81, 0x00, 0x00, 0x06, high, low], source)
            .await
            .unwrap();
        let error = request.await.unwrap_err();
        assert!(
            matches!(error, BvlcExchangeError::Nak(got) if got == code),
            "{helper:?}: {error:?}"
        );
        assert_eq!(error.nak(), Some(code));
        assert_eq!(
            error.to_string(),
            format!("BVLC-Result NAK {code} ({:#06X})", code.to_raw())
        );
    }
    transport.stop().await.unwrap();
}

/// A peer that never answers fails each request with `NoReply` once the
/// response timeout has passed, on paused time.
#[tokio::test]
async fn each_helper_reports_silence_as_no_reply() {
    let (peer, target) = peer().await;
    let mut transport = started().await;
    for helper in Helper::ALL {
        // Read the clock before the request starts: its timeout begins when
        // the exchange is admitted, on real time, before the pause below, so
        // a reading taken after the datagram arrives would miss those
        // milliseconds and come up short of the timeout.
        let sent = tokio::time::Instant::now();
        let request = helper.call(&transport, &target);
        tokio::pin!(request);
        let mut packet = [0; 64];
        tokio::select! {
            result = &mut request => panic!("{helper:?} ended before a response: {result:?}"),
            received = peer.recv_from(&mut packet) => { received.unwrap(); }
        }
        tokio::time::pause();
        let error = request.await.unwrap_err();
        let waited = sent.elapsed();
        tokio::time::resume();
        assert!(
            matches!(error, BvlcExchangeError::NoReply(timeout)
                if timeout == BipTransport::BVLC_RESPONSE_TIMEOUT),
            "{helper:?}: {error:?}"
        );
        assert_eq!(error.nak(), None);
        assert!(waited >= BipTransport::BVLC_RESPONSE_TIMEOUT, "{waited:?}");
        assert!(error.to_string().starts_with("no BVLC response"), "{error}");
    }
    transport.stop().await.unwrap();
}

/// Before start there is no exchange to run: a transport error, which keeps
/// the underlying one as its source.
#[tokio::test]
async fn each_helper_reports_a_local_failure_as_a_transport_error() {
    let transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let target = encode_bip_mac(Ipv4Addr::LOCALHOST.octets(), 0xBAC0);
    for helper in Helper::ALL {
        let error = helper.call(&transport, &target).await.unwrap_err();
        assert!(
            matches!(error, BvlcExchangeError::Transport(_)),
            "{helper:?}: {error:?}"
        );
        assert!(std::error::Error::source(&error).is_some());
    }
}

/// A timeout from the node becomes `NoReply`; any other error stays a
/// transport error.
#[test]
fn only_a_timeout_converts_to_no_reply() {
    let timeout = Duration::from_secs(3);
    assert!(matches!(
        BvlcExchangeError::from(Error::Timeout(timeout)),
        BvlcExchangeError::NoReply(t) if t == timeout
    ));
    assert!(matches!(
        BvlcExchangeError::from(Error::Encoding("x".into())),
        BvlcExchangeError::Transport(Error::Encoding(_))
    ));
}

/// A NAK answering a request is the NAK; a success answering a table read,
/// which the node never hands a read but a peer could send, is a response
/// the read does not take.
#[test]
fn a_success_answering_a_read_is_unexpected_not_a_nak() {
    let ack = BvlcFunction::READ_BROADCAST_DISTRIBUTION_TABLE_ACK;
    let nak = BvlcResultCode::READ_BROADCAST_DISTRIBUTION_TABLE_NAK;
    assert!(matches!(
        exchange::result_error(nak, ack),
        BvlcExchangeError::Nak(code) if code == nak
    ));
    assert!(matches!(
        exchange::result_error(BvlcResultCode::SUCCESSFUL_COMPLETION, ack),
        BvlcExchangeError::UnexpectedResponse { expected, got }
            if expected == ack && got == BvlcFunction::BVLC_RESULT
    ));
}
