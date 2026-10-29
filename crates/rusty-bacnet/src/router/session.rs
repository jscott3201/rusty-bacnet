//! Actual multiport activity belongs to the admitted native session.
use super::application::{Application, Slots};
use super::config::PortConfig;
use super::lifecycle::Session;
use super::records::{PyRouterPortInfo, PyRouterRoute};
use bacnet_network::router::{BACnetRouter, RouterOptions, RouterPort, RouterStopError};
use bacnet_network::router_table::Reachability;
use bacnet_types::error::Error;

pub(super) struct NativeSession {
    configs: Vec<PortConfig>,
    application: Application,
    pub router: Option<BACnetRouter>,
    /// The application ports this start made, until their stacks take them.
    pub slots: Slots,
}

impl NativeSession {
    #[cfg(test)]
    pub(super) fn from_started(router: BACnetRouter) -> Self {
        Self {
            configs: Vec::new(),
            application: Application::None,
            router: Some(router),
            slots: Slots::default(),
        }
    }
    pub fn new(configs: Vec<PortConfig>, application: Application) -> Self {
        Self {
            configs,
            application,
            router: None,
            slots: Slots::default(),
        }
    }
    pub fn ports(&self) -> Vec<PyRouterPortInfo> {
        self.router
            .as_ref()
            .unwrap()
            .ports()
            .iter()
            .map(|port| PyRouterPortInfo {
                index: port.index(),
                network_number: port.network_number(),
                mac: port.local_mac().to_vec(),
            })
            .collect()
    }
    pub fn routes(&self) -> Vec<PyRouterRoute> {
        let router = self.router.as_ref().unwrap();
        let table = router.table();
        let now = router.now();
        // Only owned Rust data crosses this lock boundary; Python conversion
        // happens after both this table guard and lifecycle admission are gone.
        table
            .routes()
            .map(|(network_number, route)| PyRouterRoute {
                network_number,
                port_index: route.port().0,
                direct: route.is_direct(),
                next_hop: (!route.is_direct()).then(|| route.next_hop().as_slice().to_vec()),
                reachability: match table.effective_reachability(network_number, now).unwrap() {
                    Reachability::Reachable => "reachable",
                    Reachability::Busy => "busy",
                    Reachability::Unreachable => "unreachable",
                },
            })
            .collect()
    }
}

impl Session for NativeSession {
    type StartError = Error;
    type StopError = RouterStopError;
    async fn start(&mut self) -> Result<(), Error> {
        let ports = self
            .configs
            .iter()
            .map(|config| RouterPort {
                transport: config.transport(),
                network_number: config.network_number,
            })
            .collect();
        let started = BACnetRouter::start(ports, RouterOptions::new()).await?;
        // The network-control receiver stays internal: none was asked for.
        let (router, slots) = Slots::split(self.application, started);
        self.router = Some(router);
        self.slots = slots;
        Ok(())
    }
    async fn stop(&mut self) -> Result<(), RouterStopError> {
        let stopped = self.router.as_mut().unwrap().stop().await;
        // Ports no stack took end with this start.
        self.slots = Slots::default();
        stopped
    }
}
