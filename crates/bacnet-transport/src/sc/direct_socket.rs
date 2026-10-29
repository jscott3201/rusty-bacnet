//! Private dial evidence: arbitrary public factories remain send-only.
use super::WebSocketPort;
use bacnet_types::error::Error;
use std::{future::Future, pin::Pin, sync::Arc};

pub(crate) enum DirectFrame {
    Binary(Vec<u8>),
    #[cfg(feature = "sc-tls")]
    Control,
}
pub(crate) enum DirectSocket<W> {
    Custom(W),
    #[cfg(feature = "sc-tls")]
    Verified(Box<crate::sc_tls::TlsWebSocket>),
}
pub(crate) type CustomDialer<W> =
    Arc<dyn Fn(String) -> Pin<Box<dyn Future<Output = Result<W, Error>> + Send>> + Send + Sync>;
pub(crate) enum DirectDialer<W> {
    Custom(CustomDialer<W>),
    #[cfg(feature = "sc-tls")]
    Tls(crate::sc_tls::ScNodeTlsConfig),
}
impl<W> Clone for DirectDialer<W> {
    fn clone(&self) -> Self {
        match self {
            Self::Custom(f) => Self::Custom(f.clone()),
            #[cfg(feature = "sc-tls")]
            Self::Tls(c) => Self::Tls(c.clone()),
        }
    }
}
impl<W: WebSocketPort> DirectDialer<W> {
    pub(crate) async fn dial(&self, uri: String) -> Result<DirectSocket<W>, Error> {
        match self {
            Self::Custom(f) => f(uri).await.map(DirectSocket::Custom),
            #[cfg(feature = "sc-tls")]
            Self::Tls(config) => crate::sc_tls::TlsWebSocket::connect_direct(&uri, config.clone())
                .await
                .map(|ws| DirectSocket::Verified(Box::new(ws))),
        }
    }
}
impl<W: WebSocketPort> DirectSocket<W> {
    #[cfg(feature = "sc-tls")]
    pub(crate) fn leaf(&self) -> Option<[u8; 32]> {
        match self {
            Self::Custom(_) => None,
            #[cfg(feature = "sc-tls")]
            Self::Verified(ws) => Some(ws.verified_leaf),
        }
    }
    /// The verified leaf certificate of a TLS dial, for the provenance of
    /// what the peer sends (#1965); empty for a custom socket.
    #[cfg(feature = "sc-tls")]
    pub(crate) fn leaf_certificate(&self) -> crate::port::LeafCertificate {
        match self {
            Self::Custom(_) => Default::default(),
            Self::Verified(ws) => ws
                .peer_info()
                .certificates()
                .first()
                .map_or_else(Default::default, |leaf| {
                    crate::port::LeafCertificate::verified(Arc::new(leaf.clone()))
                }),
        }
    }
    #[cfg(feature = "sc-tls")]
    pub(crate) fn peer_address(&self) -> Option<std::net::SocketAddr> {
        match self {
            Self::Custom(_) => None,
            Self::Verified(ws) => Some(ws.peer_address),
        }
    }
    pub(crate) async fn next_frame(&self) -> Result<DirectFrame, Error> {
        match self {
            Self::Custom(ws) => ws.recv().await.map(DirectFrame::Binary),
            #[cfg(feature = "sc-tls")]
            Self::Verified(ws) => ws.direct_frame().await,
        }
    }
}
impl<W: WebSocketPort> WebSocketPort for DirectSocket<W> {
    async fn send(&self, bytes: &[u8]) -> Result<(), Error> {
        match self {
            Self::Custom(ws) => ws.send(bytes).await,
            #[cfg(feature = "sc-tls")]
            Self::Verified(ws) => ws.send(bytes).await,
        }
    }
    async fn recv(&self) -> Result<Vec<u8>, Error> {
        #[cfg(not(feature = "sc-tls"))]
        {
            let Self::Custom(ws) = self;
            ws.recv().await
        }
        #[cfg(feature = "sc-tls")]
        loop {
            match self.next_frame().await? {
                DirectFrame::Binary(b) => return Ok(b),
                DirectFrame::Control => tokio::task::yield_now().await,
            }
        }
    }
}
