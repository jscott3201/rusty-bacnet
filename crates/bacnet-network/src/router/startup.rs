//! Sequential startup with retained rollback for failures and cancelled waits.

use super::*;

impl BACnetRouter {
    pub(super) async fn start_dispatch<T: TransportPort + 'static>(
        ports: Vec<RouterPort<T>>,
        track_depth: bool,
        control: Arc<control_policy::ControlGate>,
        network_control: Option<AdmissionSender<ReceivedNetworkControl>>,
        engine_options: bacnet_router_core::router::Options,
    ) -> Result<(Self, mpsc::Receiver<ReceivedApdu>, QueueAdmissionCounters), Error> {
        if ports.is_empty() || ports.len() > PORTS {
            return Err(Error::Encoding(format!(
                "A router needs 1 to {PORTS} ports, not {}",
                ports.len()
            )));
        }
        // Refuse reserved and duplicate network numbers
        {
            let mut seen = std::collections::HashSet::new();
            for port in &ports {
                if matches!(port.network_number, 0 | 0xFFFF) {
                    return Err(Error::Encoding(format!(
                        "A router port can't be on network {}: ports take 1 to 65534",
                        port.network_number
                    )));
                }
                if !seen.insert(port.network_number) {
                    return Err(Error::Encoding(format!(
                        "Duplicate network number {} in router ports",
                        port.network_number
                    )));
                }
            }
        }

        let mut startup = Startup::new(ports);
        let (local_tx, local_rx, counters) = AdmissionReceiver::channel(track_depth);

        // Start each transport, and note what the engine and its hooks need.
        let mut port_receivers = Vec::new();
        let mut port_info = Vec::new();
        let mut configs = Vec::new();
        let mut networks = Vec::new();
        let mut limit_changes = Vec::new();

        let setup = async {
            for index in 0..startup.ports.len() {
                startup.attempted = index + 1;
                let port = &mut startup.ports[index];
                // A routing node may owe its data link more than other nodes:
                // MS/TP turns on extended frames here (Clause 9.3).
                port.transport.prepare_for_routing();
                let rx = port.transport.start().await?;
                port_receivers.push(rx);
                let mac = Mac::from_slice(port.transport.local_mac()).map_err(|_| {
                    Error::Encoding(format!(
                        "Router port on network {} has a MAC longer than {} octets",
                        port.network_number,
                        bacnet_npdu::MAX_MAC_LEN
                    ))
                })?;
                port_info.push(RouterPortInfo {
                    index: index as u8,
                    network_number: port.network_number,
                    mac: MacAddr::from_slice(mac.as_slice()),
                });
                // Read once the transport has started: MS/TP's grows with the
                // extended frames routing turned on, and BACnet/SC's comes from
                // its hub connection.
                configs.push(Engine::port(
                    port.network_number,
                    mac,
                    port.transport.max_npdu(),
                    PortGroups {
                        groups: port.transport.group_destinations(),
                        local_broadcasts: port.transport.local_broadcast_destinations(),
                    },
                ));
                // Taken after that read; the task that follows it applies the
                // value it holds first, so a change in between isn't lost.
                limit_changes.push(port.transport.max_npdu_changes());
                networks.push(port.network_number);
            }
            Engine::new(&configs, tokio_clock(), engine_options)
                .map_err(|error| Error::Encoding(error.to_string()))
        }
        .await;
        let engine = match setup {
            Ok(engine) => engine,
            Err(error) => {
                if let Some(cleanup) = startup.cleanup()
                    && let Err(join_error) = cleanup.await
                {
                    tracing::error!(%join_error, "router startup rollback owner failed");
                }
                return Err(error);
            }
        };
        let ports = std::mem::take(&mut startup.ports);

        let network_control_ingress_sequence = Arc::new(AtomicU64::new(0));
        let shared = Arc::new(Shared::new(engine, control, networks));

        let (send_txs, send_rxs): (Vec<_>, Vec<_>) = (0..ports.len())
            .map(|_| send_queue::channel(send_queue::CAPACITY))
            .unzip();
        let replies = Arc::new(replies::ReplyLifetime::default());
        let outputs = Arc::new(Outputs {
            replies: Arc::clone(&replies),
            send_txs,
            local_tx,
            local_control: local_control::LocalControl::new(
                network_control,
                Arc::clone(&network_control_ingress_sequence),
            ),
        });
        // Senders may return a late rejection through these queues, but
        // must not keep them or the local consumers alive after dispatch.
        let (stop_senders, sender_stop) = tokio::sync::watch::channel(false);
        let sender_tasks = ports
            .into_iter()
            .zip(send_rxs)
            .enumerate()
            .map(|(index, (port, send_rx))| {
                (
                    port.network_number,
                    tokio::spawn(sender::owned(
                        port.transport,
                        // At most 255 ports, checked above.
                        PortId(index as u8),
                        send_rx,
                        Arc::clone(&shared),
                        Arc::downgrade(&outputs),
                        sender_stop.clone(),
                        port.network_number,
                    )),
                )
            })
            .collect();

        // Follow the limits of the ports whose transports publish them.
        let limit_tasks = limit_changes
            .into_iter()
            .enumerate()
            .filter_map(|(index, changes)| {
                // At most 255 ports, checked above.
                let port = PortId(index as u8);
                let task = limits::follow(Arc::clone(&shared), port, changes?);
                Some(tokio::spawn(task))
            })
            .collect();

        // Startup: each port hears its own network number, in a
        // Network-Number-Is (R3, Clause 6.4.20), then, in an I-Am-Router,
        // which networks lie beyond the other ports.
        shared.announce(&outputs.send_txs);

        let dispatch_tasks = port_receivers
            .into_iter()
            .enumerate()
            .map(|(index, rx)| {
                let port = PortDispatch {
                    shared: Arc::clone(&shared),
                    outputs: Arc::clone(&outputs),
                    // At most 255 ports, checked above.
                    port: index as u8,
                };
                tokio::spawn(port.run(rx))
            })
            .collect();

        // Run the engine's sweep at each of its deadlines: it purges stale
        // learned routes, lapsed busy marks, hardened pending slots and
        // discovery entries. `last_used` never extends `last_seen` expiry.
        //
        // The task sleeps until the deadline it last read, and nothing
        // wakes it early. That holds because only the sweep itself
        // (`on_deadline`) moves the engine's `next_deadline`; if the core
        // ever lets another call move it, that call's site must wake this
        // task (a `Notify`, say), or the sweep runs late.
        let aging = Arc::clone(&shared);
        let aging_task = tokio::spawn(async move {
            let clock = aging.engine.clock();
            // A deadline past the host clock's range never comes.
            let wake = |next: Option<Instant>| next.and_then(|at| clock.to_std(at));
            let mut next = wake(aging.engine.lock().next_deadline());
            loop {
                match next {
                    Some(at) => tokio::time::sleep_until(at.into()).await,
                    None => std::future::pending().await,
                }
                next = wake(aging.sweep());
            }
        });

        Ok((
            Self {
                replies,
                outputs: Arc::downgrade(&outputs),
                shared,
                lifecycle: shutdown::Lifecycle::new(shutdown::Workers {
                    dispatch: dispatch_tasks,
                    senders: sender_tasks,
                    aging: aging_task,
                    limits: limit_tasks,
                    stop_senders,
                }),
                network_control_ingress_sequence,
                port_info,
            },
            local_rx,
            counters,
        ))
    }
}

struct Startup<T: TransportPort + 'static> {
    ports: Vec<RouterPort<T>>,
    attempted: usize,
}

impl<T: TransportPort + 'static> Startup<T> {
    fn new(ports: Vec<RouterPort<T>>) -> Self {
        Self {
            ports,
            attempted: 0,
        }
    }

    fn cleanup(&mut self) -> Option<tokio::task::JoinHandle<()>> {
        if self.ports.is_empty() {
            return None;
        }
        let cleanup = Rollback {
            ports: std::mem::take(&mut self.ports),
            attempted: self.attempted,
            stopped: vec![false; self.attempted],
        };
        match tokio::runtime::Handle::try_current() {
            Ok(runtime) => Some(runtime.spawn(cleanup.run())),
            Err(_) => {
                drop(cleanup);
                None
            }
        }
    }
}

impl<T: TransportPort + 'static> Drop for Startup<T> {
    fn drop(&mut self) {
        self.cleanup();
    }
}

struct Rollback<T: TransportPort> {
    ports: Vec<RouterPort<T>>,
    attempted: usize,
    stopped: Vec<bool>,
}

impl<T: TransportPort> Rollback<T> {
    async fn run(mut self) {
        for index in 0..self.attempted {
            let port = &mut self.ports[index];
            match shutdown::observe(port.transport.stop()).await {
                Ok(Ok(())) => self.stopped[index] = true,
                Ok(Err(error)) => tracing::error!(
                    network_number = port.network_number,
                    ?error,
                    "router startup rollback transport stop failed"
                ),
                Err(detail) => {
                    tracing::error!(network_number = port.network_number, %detail, "router startup rollback transport stop panicked")
                }
            }
        }
    }
}

impl<T: TransportPort> Drop for Rollback<T> {
    fn drop(&mut self) {
        for (index, port) in self.ports.iter_mut().take(self.attempted).enumerate() {
            if !self.stopped[index] {
                port.transport.abort();
            }
        }
    }
}
