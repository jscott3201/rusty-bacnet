//! A `BACnetClient` on an endpoint's link (#2617): what
//! `await endpoint.client()` returns. It is the standalone class, running
//! on the endpoint session's full client, so every method works the same;
//! only its lifetime differs. The endpoint starts it, its `stop()` ends the
//! client role alone, and the endpoint's close stops it too.

use super::*;

impl BACnetClient {
    /// An empty client for an endpoint on a `transport` link (`"bip"`,
    /// `"sc"` or `"mstp"`), which takes the addresses that link's
    /// standalone client takes. The endpoint fills it with the session's
    /// full client ([`state::ClientState::seat`]) before handing it out.
    /// The start keywords of a standalone client have no use here; they
    /// keep their defaults.
    pub(crate) fn seated(transport: &str) -> Self {
        Self {
            inner: Arc::new(state::ClientState::seated()),
            seated: true,
            transport_type: transport.to_string(),
            interface: std::net::Ipv4Addr::UNSPECIFIED,
            port: 0,
            broadcast_address: String::new(),
            share_port_by_address: false,
            share_wildcard_port: false,
            bip_mode: bacnet_transport::bip::BipMode::default(),
            bip6_foreign: None,
            multicast: None,
            apdu_timeout_ms: 0,
            sc_hub: None,
            sc_vmac: None,
            sc_device_uuid: [0; 16],
            sc_npdu_admission_policy: bacnet_transport::sc::ScNpduAdmissionPolicy::default(),
            sc_ca_cert: None,
            sc_client_cert: None,
            sc_client_key: None,
            sc_heartbeat_interval_ms: None,
            sc_heartbeat_timeout_ms: None,
            reconnect: None,
            sc_failover_hub: None,
            sc_connect_timeout_ms: None,
            ipv6_interface: None,
            ipv6_device_instance: None,
            mstp: crate::mstp_py::MstpOptions {
                serial_port: None,
                baud: 0,
                mac: 0,
                max_master: 0,
                max_info_frames: 0,
                extended_frames: false,
                echo: false,
                echo_grace_ms: None,
                timing: crate::mstp_py::MstpTiming::default(),
            },
            router: None,
            min_request_interval_ms: 0,
            write_policy: bacnet_ops::WritePolicy::default(),
            resolve_scope: bacnet_ops::ResolveScope::Local,
        }
    }

    /// The state the endpoint keeps, to fill this client, ask where it
    /// stands and close it.
    pub(crate) fn state(&self) -> ClientInner {
        Arc::clone(&self.inner)
    }
}
