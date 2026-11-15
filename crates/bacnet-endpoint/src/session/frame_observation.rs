//! Observation of the session's own link, without another socket or reader.

use super::EndpointSession;
use bacnet_transport::port::TransportPort;
use bacnet_transport::tap::FrameTapReceiver;

impl<T: TransportPort + 'static> EndpointSession<T> {
    /// Subscribe to raw frames on a B/IP-builder session's transport.
    ///
    /// Returns `None` outside the running state or when the builder did not
    /// supply a tap. Each receiver has its own bounded queue; full queues
    /// discard new frames and count them in [`FrameTapReceiver::dropped`].
    /// The receiver does not keep the transport alive. As with the transport's
    /// tap, stopping alone does not end it: drop it when observation is over.
    pub fn frames(&self, capacity: usize) -> Option<FrameTapReceiver> {
        self.is_running()
            .then(|| {
                self.frame_tap
                    .as_ref()
                    .map(|tap| tap.subscribe_with_capacity(capacity))
            })
            .flatten()
    }
}

#[cfg(test)]
mod tests {
    use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
    use std::time::Duration;

    use bacnet_transport::loopback::LoopbackTransport;
    use bacnet_transport::tap::{TapDirection, TapPeer};

    use crate::bip::BipEndpointBuilder;
    use crate::session::{EndpointSession, SessionConfig, SessionRole};

    /// A B/IP-builder session hands out receivers on its own socket's tap
    /// while it runs, malformed datagrams included, and none before start
    /// or after stop. Real UDP on loopback, so real time.
    #[tokio::test]
    async fn a_running_bip_session_observes_its_own_socket() {
        let mut session = BipEndpointBuilder::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::LOCALHOST)
            .role(SessionRole::ClientOnly)
            .build_session()
            .unwrap();
        assert!(session.frames(4).is_none(), "not started");
        session.start().await.unwrap();
        let mut frames = session.frames(4).expect("running");
        let to = session.bip_local_address().unwrap();
        let peer = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        peer.send_to(b"not a BVLL message", SocketAddr::V4(to))
            .unwrap();
        let frame = tokio::time::timeout(Duration::from_secs(5), frames.recv())
            .await
            .expect("the datagram is tapped")
            .unwrap();
        assert_eq!(frame.direction, TapDirection::Inbound);
        assert_eq!(frame.peer, TapPeer::Udp(peer.local_addr().unwrap()));
        assert_eq!(&frame.bytes[..], b"not a BVLL message");
        assert_eq!(frame.missed_before, 0);
        session.stop().await.unwrap();
        assert!(session.frames(4).is_none(), "stopped");
    }

    /// A session no B/IP builder made has no tap to hand out, running or not.
    #[tokio::test(start_paused = true)]
    async fn a_session_without_a_tap_hands_out_none() {
        let (transport, _peer) = LoopbackTransport::pair(vec![0x01], vec![0x02]);
        let mut session =
            EndpointSession::new(transport, SessionRole::ClientOnly, SessionConfig::default())
                .unwrap();
        session.start().await.unwrap();
        assert!(session.frames(4).is_none());
        session.stop().await.unwrap();
    }
}
