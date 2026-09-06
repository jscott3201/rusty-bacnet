//! Validated phase budgets for accepting hub connections.

use bacnet_types::error::Error;
use std::time::Duration;

/// Independent absolute budgets for TCP-to-TLS, TLS-to-WebSocket, and
/// WebSocket-to-valid-Connect-Request admission.
///
/// TLS and HTTP upgrade bounds are local resource policy: greater than zero
/// through 300 seconds. The Connect-Request range is 5–300 seconds under
/// Annex AB.6.2.3; its recommended default is 10 seconds. All defaults are 10s.
/// Expired upgraded peers get a best-effort Close with a separate one-second
/// local cleanup grace, followed by transport release. This is not a promise
/// of a completed reciprocal WebSocket close handshake.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScHubHandshakeTimeouts {
    tls: Duration,
    websocket_upgrade: Duration,
    connect_request: Duration,
}

impl ScHubHandshakeTimeouts {
    /// Construct checked phase budgets. Invalid or excessive durations return
    /// [`Error::Encoding`] before any socket is opened.
    pub fn new(
        tls: Duration,
        websocket_upgrade: Duration,
        connect_request: Duration,
    ) -> Result<Self, Error> {
        for (name, value, minimum) in [
            ("TLS", tls, Duration::from_nanos(1)),
            (
                "WebSocket upgrade",
                websocket_upgrade,
                Duration::from_nanos(1),
            ),
            ("Connect-Request", connect_request, Duration::from_secs(5)),
        ] {
            if value < minimum || value > Duration::from_secs(300) {
                return Err(Error::Encoding(format!(
                    "Hub {name} timeout must be between {minimum:?} and 300s"
                )));
            }
        }
        Ok(Self {
            tls,
            websocket_upgrade,
            connect_request,
        })
    }

    /// Budget beginning at TCP admission.
    pub fn tls(self) -> Duration {
        self.tls
    }

    /// Budget beginning when the TLS handshake succeeds.
    pub fn websocket_upgrade(self) -> Duration {
        self.websocket_upgrade
    }

    /// Budget beginning at successful WebSocket acceptance and ending at
    /// valid Request registration commit, before Connect-Accept is sent.
    pub fn connect_request(self) -> Duration {
        self.connect_request
    }
}

impl Default for ScHubHandshakeTimeouts {
    fn default() -> Self {
        Self {
            tls: Duration::from_secs(10),
            websocket_upgrade: Duration::from_secs(10),
            connect_request: Duration::from_secs(10),
        }
    }
}
