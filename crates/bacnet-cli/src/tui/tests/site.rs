//! An in-memory site for the TUI's in-process tests: real `BACnetServer`s
//! and a `BACnetClient` on a hub, with the worker and `update` driven as
//! the UI loop drives them, minus the terminal.
//!
//! The hub gives every port a 6-byte MAC shaped like a BACnet/IP address, so
//! frames read like a real site and stay the same on every run. It can also
//! stand in for a device that refuses ReadPropertyMultiple, and it counts
//! each device's outstanding confirmed requests.
//!
//! The tests run on paused tokio time. Everything here is in memory and on
//! the test's runtime, so the clock only moves when every task is idle: the
//! listen window cannot end while a reply is still being processed, however
//! slow the machine. Reply counts are therefore exact rather than timing-bound.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bacnet_client::client::BACnetClient;
use bacnet_encoding::npdu::decode_npdu;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_server::server::BACnetServer;
use bacnet_transport::port::{ReceivedNpdu, TransportPort};
use bacnet_types::enums::ConfirmedServiceChoice;
use bacnet_types::error::Error;
use bacnet_types::MacAddr;
use bytes::Bytes;
use ratatui::crossterm::event::KeyCode;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use super::lab::{lab_database, lab_identity};
use super::*;
use crate::tui::app::{Link, OpState};
use crate::tui::worker::session::SessionOptions;
use crate::tui::worker::{session, EventSink, EVENT_CHANNEL_CAPACITY};

/// Reject reason: the service is not recognized (Clause 20.1.5).
const UNRECOGNIZED_SERVICE: u8 = 9;
/// Abort reason: the answer would need segmentation (Clause 20.1.6).
const SEGMENTATION_NOT_SUPPORTED: u8 = 4;

/// How a server's port departs from a well-behaved device.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct Quirks {
    /// Reject every ReadPropertyMultiple as an unknown service.
    pub(super) refuse_rpm: bool,
    /// Abort the first ReadPropertyMultiple as too large to send.
    pub(super) abort_first_rpm: bool,
    /// Answer no confirmed request at all.
    pub(super) silent: bool,
    /// Reject SubscribeCOV and SubscribeCOVProperty as unknown services: a
    /// device without COV.
    pub(super) refuse_cov: bool,
    /// After the first SubscribeCOV, swallow this many more, retries
    /// included: a renewal that times out.
    pub(super) lose_cov_after_first: u8,
    /// Answer every ReadPropertyMultiple with an OBJECT / UNKNOWN_OBJECT
    /// error, as a device that fails a whole request for one bad object.
    pub(super) fail_rpm: bool,
    /// Never answer WriteProperty or WritePropertyMultiple: a write still
    /// in flight when the session ends.
    pub(super) swallow_writes: bool,
    /// Fail any ReadProperty or ReadPropertyMultiple that asks for
    /// State_Text with PROPERTY / UNKNOWN_PROPERTY, as a device without it.
    pub(super) refuse_state_text: bool,
    /// Fail any ReadProperty or ReadPropertyMultiple that asks for
    /// Object_Name the same way, as a device that won't give names.
    pub(super) refuse_object_name: bool,
    /// Swallow this many confirmed requests, retries included, then
    /// answer: a device that was away for a while.
    pub(super) swallow_first: u8,
}

struct Port {
    mac: MacAddr,
    inbox: mpsc::Sender<ReceivedNpdu>,
    quirks: Quirks,
    /// SubscribeCOV requests answered so far.
    cov_answered: usize,
}

#[derive(Default)]
struct Ports {
    ports: Vec<Port>,
    /// Confirmed requests outstanding per destination, and the most seen.
    outstanding: HashMap<MacAddr, (usize, usize)>,
    /// Every confirmed request: its destination, service and when it went.
    requests: Vec<(MacAddr, u8, tokio::time::Instant)>,
    /// Frames each port has sent, broadcasts and replies included.
    sent: HashMap<MacAddr, usize>,
}

/// Every port sees every broadcast; a unicast reaches the port with that MAC.
#[derive(Default)]
pub(super) struct Hub {
    inner: Mutex<Ports>,
}

impl Hub {
    /// The most confirmed requests that were ever outstanding at once to
    /// the port with `mac`.
    pub(super) fn most_outstanding(&self, mac: &[u8]) -> usize {
        let inner = self.inner.lock().unwrap();
        inner
            .outstanding
            .get(&MacAddr::from_slice(mac))
            .map_or(0, |(_, most)| *most)
    }

    /// Every frame the port with `mac` has put on the hub.
    pub(super) fn frames_from(&self, mac: &[u8]) -> usize {
        let inner = self.inner.lock().unwrap();
        inner
            .sent
            .get(&MacAddr::from_slice(mac))
            .copied()
            .unwrap_or(0)
    }

    /// The services of the confirmed requests sent to `mac`, in order,
    /// retries included.
    pub(super) fn services_to(&self, mac: &[u8]) -> Vec<u8> {
        let inner = self.inner.lock().unwrap();
        inner
            .requests
            .iter()
            .filter(|(to, ..)| to.as_slice() == mac)
            .map(|(_, service, _)| *service)
            .collect()
    }

    /// The confirmed requests sent to `mac`: each one's service and when
    /// it went, retries included.
    pub(super) fn requests_to(&self, mac: &[u8]) -> Vec<(u8, tokio::time::Instant)> {
        let inner = self.inner.lock().unwrap();
        inner
            .requests
            .iter()
            .filter(|(to, ..)| to.as_slice() == mac)
            .map(|(_, service, at)| (*service, *at))
            .collect()
    }
}

/// The APDU's PDU type and, for an unsegmented confirmed request, its
/// invoke id and service.
fn apdu_kind(npdu: &[u8]) -> Option<(u8, Option<(u8, u8)>)> {
    let decoded = decode_npdu(Bytes::copy_from_slice(npdu)).ok()?;
    if decoded.is_network_message() {
        return None;
    }
    let apdu = decoded.payload;
    let pdu_type = apdu.first()? >> 4;
    let request = (pdu_type == 0 && apdu[0] & 0x08 == 0)
        .then(|| Some((*apdu.get(2)?, *apdu.get(3)?)))
        .flatten();
    Some((pdu_type, request))
}

pub(super) struct HubPort {
    hub: Arc<Hub>,
    mac: MacAddr,
    rx: Option<mpsc::Receiver<ReceivedNpdu>>,
}

impl HubPort {
    pub(super) fn join(hub: &Arc<Hub>, ip: [u8; 4], quirks: Quirks) -> Self {
        let mac = MacAddr::from_slice(&[ip[0], ip[1], ip[2], ip[3], 0xBA, 0xC0]);
        let (inbox, rx) = mpsc::channel(256);
        hub.inner.lock().unwrap().ports.push(Port {
            mac: mac.clone(),
            inbox,
            quirks,
            cov_answered: 0,
        });
        Self {
            hub: Arc::clone(hub),
            mac,
            rx: Some(rx),
        }
    }

    fn deliver(&self, npdu: &[u8], to: Option<&[u8]>) {
        let kind = apdu_kind(npdu);
        let mut inner = self.hub.inner.lock().unwrap();
        *inner.sent.entry(self.mac.clone()).or_default() += 1;
        if let (Some(to), Some((pdu_type, request))) = (to, kind) {
            let to = MacAddr::from_slice(to);
            match (pdu_type, request) {
                (0, Some((invoke_id, service))) => {
                    inner
                        .requests
                        .push((to.clone(), service, tokio::time::Instant::now()));
                    let rpm = service == ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE.to_raw();
                    let cov = [
                        ConfirmedServiceChoice::SUBSCRIBE_COV.to_raw(),
                        ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY.to_raw(),
                    ]
                    .contains(&service);
                    let Some(port) = inner.ports.iter_mut().find(|port| port.mac == to) else {
                        return;
                    };
                    let write = [
                        ConfirmedServiceChoice::WRITE_PROPERTY.to_raw(),
                        ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE.to_raw(),
                    ]
                    .contains(&service);
                    if port.quirks.silent || (write && port.quirks.swallow_writes) {
                        return;
                    }
                    if port.quirks.swallow_first > 0 {
                        port.quirks.swallow_first -= 1;
                        return;
                    }
                    if service == ConfirmedServiceChoice::SUBSCRIBE_COV.to_raw() {
                        if port.cov_answered > 0 && port.quirks.lose_cov_after_first > 0 {
                            port.quirks.lose_cov_after_first -= 1;
                            return;
                        }
                        port.cov_answered += 1;
                    }
                    // Answer for the device, with no NPDU options.
                    let read = rpm || service == ConfirmedServiceChoice::READ_PROPERTY.to_raw();
                    // State_Text (110) as an RPM [0] or an RP [1] property.
                    let state_text = [[0x09, 110], [0x19, 110]]
                        .iter()
                        .any(|tag| npdu.windows(2).any(|pair| pair == tag));
                    // Object_Name (77), likewise.
                    let object_name = [[0x09, 77], [0x19, 77]]
                        .iter()
                        .any(|tag| npdu.windows(2).any(|pair| pair == tag));
                    let refused = (port.quirks.refuse_state_text && state_text)
                        || (port.quirks.refuse_object_name && object_name);
                    let answer =
                        if (rpm && port.quirks.refuse_rpm) || (cov && port.quirks.refuse_cov) {
                            Some(vec![0x01, 0x00, 0x60, invoke_id, UNRECOGNIZED_SERVICE])
                        } else if read && refused {
                            // Error PDU: class PROPERTY (2), code UNKNOWN_PROPERTY (32).
                            Some(vec![
                                0x01, 0x00, 0x50, invoke_id, service, 0x91, 2, 0x91, 32,
                            ])
                        } else if rpm && port.quirks.abort_first_rpm {
                            port.quirks.abort_first_rpm = false;
                            Some(vec![
                                0x01,
                                0x00,
                                0x71,
                                invoke_id,
                                SEGMENTATION_NOT_SUPPORTED,
                            ])
                        } else if rpm && port.quirks.fail_rpm {
                            // Error PDU: class OBJECT (1), code UNKNOWN_OBJECT (31).
                            Some(vec![
                                0x01, 0x00, 0x50, invoke_id, service, 0x91, 1, 0x91, 31,
                            ])
                        } else {
                            None
                        };
                    if let Some(answer) = answer {
                        let back = ReceivedNpdu::unverified(
                            Bytes::copy_from_slice(&answer),
                            to,
                            false,
                            Vec::new(),
                            None,
                        );
                        if let Some(me) = inner.ports.iter().find(|port| port.mac == self.mac) {
                            let _ = me.inbox.try_send(back);
                        }
                        return;
                    }
                    let (now, most) = inner.outstanding.entry(to).or_default();
                    *now += 1;
                    *most = (*most).max(*now);
                }
                // A reply ends the request outstanding at its sender.
                (2 | 3 | 5 | 6 | 7, _) => {
                    if let Some((now, _)) = inner.outstanding.get_mut(&self.mac) {
                        *now = now.saturating_sub(1);
                    }
                }
                _ => {}
            }
        }
        let targets: Vec<_> = inner
            .ports
            .iter()
            .filter(|port| port.mac != self.mac && to.is_none_or(|to| port.mac.as_slice() == to))
            .map(|port| port.inbox.clone())
            .collect();
        drop(inner);
        for tx in targets {
            let npdu = ReceivedNpdu::unverified(
                Bytes::copy_from_slice(npdu),
                self.mac.clone(),
                to.is_none(),
                Vec::new(),
                None,
            );
            let _ = tx.try_send(npdu);
        }
    }
}

impl TransportPort for HubPort {
    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        self.rx
            .take()
            .ok_or_else(|| Error::Encoding("hub port already started".into()))
    }

    async fn stop(&mut self) -> Result<(), Error> {
        Ok(())
    }

    async fn send_unicast(&self, npdu: &[u8], mac: &[u8]) -> Result<(), Error> {
        self.deliver(npdu, Some(mac));
        Ok(())
    }

    async fn send_broadcast(&self, npdu: &[u8]) -> Result<(), Error> {
        self.deliver(npdu, None);
        Ok(())
    }

    fn local_mac(&self) -> &[u8] {
        &self.mac
    }

    fn local_receive_apdu_capacity(&self) -> u16 {
        1476
    }
}

pub(super) async fn bounded<T>(future: impl Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(60), future)
        .await
        .expect("in-process TUI test exceeded its deadline")
}

/// A device with only its Device object, vendor 0.
fn bare_device(instance: u32) -> (bacnet_objects::DeviceIdentity, ObjectDatabase) {
    let identity = bacnet_objects::DeviceIdentity::new(instance, 0)
        .expect("valid Device identity")
        .with_name(format!("Lab device {instance}"));
    let mut db = ObjectDatabase::new();
    db.add(Box::new(
        DeviceObject::new(DeviceConfig {
            identity: identity.clone(),
            ..Default::default()
        })
        .unwrap(),
    ))
    .unwrap();
    (identity, db)
}

/// One server on the hub.
pub(super) struct Server {
    /// Last octet of its 10.0.0.x address.
    pub(super) octet: u8,
    /// Its device instance.
    pub(super) instance: u32,
    /// Its objects; a bare Device object when `None`.
    pub(super) database: Option<ObjectDatabase>,
    /// How its port misbehaves.
    pub(super) quirks: Quirks,
    /// Deny every write by mutation policy (SERVICE_REQUEST_DENIED).
    pub(super) deny_writes: bool,
    /// Its Max_APDU_Length_Accepted, when not the default.
    pub(super) max_apdu: Option<u16>,
}

impl Server {
    /// A device with only its Device object.
    pub(super) fn bare(octet: u8, instance: u32) -> Self {
        Self {
            octet,
            instance,
            database: None,
            quirks: Quirks::default(),
            deny_writes: false,
            max_apdu: None,
        }
    }

    /// The lab device.
    pub(super) fn lab(octet: u8, instance: u32) -> Self {
        Self {
            database: Some(lab_database(instance)),
            ..Self::bare(octet, instance)
        }
    }
}

/// The client's limits.
pub(super) struct ClientLimits {
    /// Max APDU length accepted.
    pub(super) max_apdu: u16,
    /// Whether it takes segmented replies.
    pub(super) segmented: bool,
}

impl Default for ClientLimits {
    fn default() -> Self {
        Self {
            max_apdu: 1476,
            segmented: true,
        }
    }
}

/// A site: servers on the hub, the client's worker, and the UI state.
pub(super) struct Site {
    pub(super) hub: Arc<Hub>,
    servers: Vec<BACnetServer<HubPort>>,
    pub(super) app: App,
    events: mpsc::Receiver<WorkerEvent>,
    commands: Option<mpsc::Sender<Command>>,
    worker: Option<JoinHandle<()>>,
}

impl Site {
    /// Bare servers `(last IP octet, device instance)`, and a connected
    /// client at 10.0.0.1.
    pub(super) async fn start(devices: &[(u8, u32)]) -> Self {
        let servers = devices
            .iter()
            .map(|&(octet, instance)| Server::bare(octet, instance))
            .collect();
        Self::start_with(servers, ClientLimits::default()).await
    }

    /// `servers` and a connected client at 10.0.0.1 with `limits`.
    pub(super) async fn start_with(servers: Vec<Server>, limits: ClientLimits) -> Self {
        Self::start_with_options(servers, limits, SessionOptions::default()).await
    }

    /// `servers` and a connected client at 10.0.0.1 with `limits`, its
    /// session run with `options`.
    pub(super) async fn start_with_options(
        servers: Vec<Server>,
        limits: ClientLimits,
        options: SessionOptions,
    ) -> Self {
        let hub = Arc::new(Hub::default());
        let mut started = Vec::new();
        for server in servers {
            let (identity, database) = match server.database {
                Some(database) => (lab_identity(server.instance), database),
                None => bare_device(server.instance),
            };
            let identity = match server.max_apdu {
                Some(max_apdu) => identity.with_max_apdu(max_apdu).unwrap(),
                None => identity,
            };

            let policy = if server.deny_writes {
                bacnet_server::mutation::MutationPolicy::DenyAll
            } else {
                bacnet_server::mutation::MutationPolicy::Permissive
            };
            let builder = BACnetServer::generic_builder()
                .device(identity)
                .transport(HubPort::join(&hub, [10, 0, 0, server.octet], server.quirks))
                .database(database)
                .mutation_policy(policy);
            started.push(bounded(Box::pin(builder.build())).await.unwrap());
        }
        let client = BACnetClient::generic_builder()
            .transport(HubPort::join(&hub, [10, 0, 0, 1], Quirks::default()))
            .apdu_timeout_ms(2_000)
            .max_apdu_length(limits.max_apdu)
            .segmented_response_accepted(limits.segmented);
        let client = bounded(Box::pin(client.build())).await.unwrap();

        let (sink, events) = EventSink::channel(EVENT_CHANNEL_CAPACITY);
        let (commands, command_rx) = mpsc::channel(16);
        let worker = tokio::spawn(Box::pin(session::serve(
            client,
            AddressStyle::Bip,
            sink,
            command_rx,
            options,
        )));
        let mut site = Self {
            hub,
            servers: started,
            app: new_app(None),
            events,
            commands: Some(commands),
            worker: Some(worker),
        };
        site.pump_until(|app| matches!(app.link, Link::Up { .. }))
            .await;
        site
    }

    /// Hand commands to the worker, as the UI loop does.
    pub(super) async fn send(&self, commands: Vec<Command>) {
        for command in commands {
            self.commands.as_ref().unwrap().send(command).await.unwrap();
        }
    }

    /// Press a key, as the UI loop does, and hand its commands on.
    pub(super) async fn press(&mut self, code: KeyCode) {
        let commands = press(&mut self.app, code);
        self.send(commands).await;
    }

    /// Apply worker events until `done` holds.
    pub(super) async fn pump_until(&mut self, done: impl Fn(&App) -> bool) {
        bounded(async {
            while !done(&self.app) {
                let event = self.events.recv().await.expect("worker stopped");
                let commands = worker(&mut self.app, event);
                self.send(commands).await;
            }
        })
        .await;
    }

    /// Apply worker events until one satisfies `done`, which sees each first.
    pub(super) async fn pump_events_until(&mut self, mut done: impl FnMut(&WorkerEvent) -> bool) {
        bounded(async {
            loop {
                let event = self.events.recv().await.expect("worker stopped");
                let finished = done(&event);
                let commands = worker(&mut self.app, event);
                self.send(commands).await;
                if finished {
                    return;
                }
            }
        })
        .await;
    }

    /// Fill in the Who-Is form through the keymap, with a 2 s listen window,
    /// send it (confirming the warning if one appears), wait out the window,
    /// and then until the table holds at least `rows` devices.
    pub(super) async fn who_is(&mut self, range: &str, rows: usize) {
        press(&mut self.app, KeyCode::Char('d'));
        press(&mut self.app, KeyCode::Tab);
        type_text(&mut self.app, range);
        press(&mut self.app, KeyCode::Tab);
        press(&mut self.app, KeyCode::Backspace);
        type_text(&mut self.app, "2");
        let mut commands = press(&mut self.app, KeyCode::Enter);
        if commands.is_empty() {
            commands = press(&mut self.app, KeyCode::Enter);
        }
        assert_eq!(commands.len(), 1, "{commands:?}");
        self.send(commands).await;
        self.pump_until(|app| app.op.as_ref().is_some_and(|op| !op.running()))
            .await;
        assert_eq!(self.app.op.as_ref().unwrap().state, OpState::Done);
        // With paused time every reply is in before the window ends; this is
        // a second guard that costs nothing when it already holds.
        self.pump_until(|app| app.devices.len() >= rows).await;
    }

    /// The server at index `i` of those the site started with.
    pub(super) fn server(&self, i: usize) -> &BACnetServer<HubPort> {
        &self.servers[i]
    }

    /// Apply worker events, and hand on their commands, for `span` of
    /// (paused) time.
    pub(super) async fn run_for(&mut self, span: Duration) {
        let end = tokio::time::Instant::now() + span;
        loop {
            tokio::select! {
                event = self.events.recv() => {
                    let commands = worker(&mut self.app, event.expect("worker stopped"));
                    self.send(commands).await;
                }
                () = tokio::time::sleep_until(end) => return,
            }
        }
    }

    /// Quit as the UI does: drop the command sender and wait for the
    /// worker, which cancels the watch list's subscriptions and stops the
    /// client. The servers keep running.
    pub(super) async fn quit(&mut self) {
        self.commands = None;
        if let Some(worker) = self.worker.take() {
            bounded(worker).await.unwrap();
        }
    }

    pub(super) async fn stop(mut self) {
        // Dropping the command sender stops the worker, which stops the client.
        self.quit().await;
        for server in &mut self.servers {
            bounded(server.stop()).await.unwrap();
        }
    }
}
