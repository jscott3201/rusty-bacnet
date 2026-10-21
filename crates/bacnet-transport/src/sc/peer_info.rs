//! Atomic observation of the accepted active Hub socket.

use std::sync::Arc;
use std::time::SystemTime;

use super::{ScConnectionState, ScTransport};
use crate::sc_tls::{ScTlsPeerInfo, TlsWebSocket};

/// An owned observation of the accepted active Hub connection.
///
/// This is historical after it is returned, and retaining it does not retain
/// the transport. The timestamp records observation, not certificate validation.
#[derive(Clone, Debug)]
pub struct ScHubPeerInfo {
    tls: Arc<ScTlsPeerInfo>,
    observed_at: SystemTime,
}

impl ScHubPeerInfo {
    /// Certificate information for the observed physical connection.
    pub fn tls(&self) -> &ScTlsPeerInfo {
        &self.tls
    }
    /// When this connection was observed as SC Connected, not validation time.
    pub fn observed_at(&self) -> SystemTime {
        self.observed_at
    }
}

impl ScTransport<TlsWebSocket> {
    /// Copy information about the accepted active Hub; no additional dial or
    /// handshake. None before start, during disconnection/reconnect, or after
    /// stop. Failed candidates cannot replace the currently accepted socket.
    ///
    /// Reachable through `BACnetClient::transport()` and `AnyTransport::Sc`.
    pub async fn hub_peer_info(&self) -> Option<ScHubPeerInfo> {
        let ws = self.ws_shared.as_ref()?.lock().await;
        let conn = self.connection.as_ref()?.lock().await;
        if conn.state != ScConnectionState::Connected {
            return None;
        }
        Some(ScHubPeerInfo {
            tls: Arc::clone(ws.inner().peer_info()),
            observed_at: SystemTime::now(),
        })
    }
}
