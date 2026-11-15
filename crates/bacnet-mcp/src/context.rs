//! What every tool call shares: the client, the policy, the cursors and
//! caches ([`Ctx`]), and one call's budget and cost ([`Call`]).

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use bacnet_client::client::{DeviceEventKind, DiscoveredRouter};
use bacnet_client::discovery::DiscoveredDevice;
use bacnet_ops::limits::{Account, TokenBucket};
use bacnet_ops::target::{self, Target};
use bacnet_ops::watch::Point;
use bacnet_ops::watch::hub::Hub;
use bacnet_ops::{Client, DeviceAddress, OpsError};
use bacnet_transport::port::TransportPort;
use bacnet_types::primitives::ObjectIdentifier;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::watch;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use crate::cursor::Cursors;
use crate::errors::ToolError;
use crate::listen::{Subs, TOOL_IDLE};
use crate::policy::Policy;
use crate::resources::Cache;

/// The server's state, shared by every call.
pub(crate) struct Ctx<T: TransportPort + 'static> {
    /// The client; `None` in passive mode, where no client is built.
    pub(crate) client: Option<Client<T>>,
    pub(crate) policy: Policy,
    /// The one principal of a stdio server: the process's OS user.
    pub(crate) principal: String,
    pub(crate) cursors: Cursors,
    /// The principal's token bucket.
    pub(crate) bucket: Arc<TokenBucket>,
    /// When the principal's last broad Who-Is went.
    pub(crate) broad: Arc<Mutex<Option<Instant>>>,
    /// Object lists read, by the device's address, for paging without
    /// reading again.
    pub(crate) lists: Mutex<HashMap<DeviceAddress, CachedList>>,
    /// One lock a device, held while its list is read, so two calls on one
    /// device take turns and the second goes on from where the first
    /// stopped.
    pub(crate) list_locks: Mutex<HashMap<DeviceAddress, Arc<tokio::sync::Mutex<()>>>>,
    /// Calls and requests per device, by its address and with its
    /// instance, logged as a summary at shutdown.
    pub(crate) reads: Mutex<BTreeMap<DeviceAddress, (u32, u64, u64)>>,
    /// Object names and descriptions read by `bacnet_find_object`, by the
    /// device's address, so its later pages don't read them again.
    pub(crate) names: Mutex<HashMap<DeviceAddress, CachedNames>>,
    /// What Who-Is-Router-To-Network queries found, and when the last one
    /// went; at most one goes a minute for the principal.
    pub(crate) routers: Arc<Mutex<Routers>>,
    /// When the last BBMD survey of `bacnet_network_map` began; at most
    /// one goes a minute for the principal.
    pub(crate) bbmd_asked: Arc<Mutex<Option<Instant>>>,
    /// The watch hub; `None` in passive mode.
    pub(crate) hub: Option<Hub<T>>,
    /// Whether the session shuts the hub down when it ends: not when the
    /// TUI's listener shares it among its sessions (#2185).
    pub(crate) owns_hub: bool,
    /// How long the client has to open the session, for the TUI's
    /// listener; `None` on stdio.
    pub(crate) initialize: Option<Duration>,
    /// What the principal watches, and the subscriptions waiting for their
    /// streams.
    pub(crate) subs: Arc<Subs>,
    /// Cancelled when the server shuts down gracefully: listen streams
    /// complete.
    pub(crate) closing: CancellationToken,
    /// Listen streams running.
    pub(crate) listening: watch::Sender<usize>,
    /// The discovery table's devices, one per address with the instance it
    /// claims, kept for the subscription acknowledgment, which the SDK asks
    /// for synchronously. Two devices sharing an instance are two entries
    /// (#2560).
    pub(crate) known: Mutex<HashMap<DeviceAddress, u32>>,
    /// Device summaries served to `bacnet://device/{instance}`, by the
    /// address the instance named when read.
    pub(crate) described: Cache,
    /// Alarm pages served to the alarm resources, by address as above.
    pub(crate) alarms: Cache,
    /// The write session, the plan grant's budget and the confirmation
    /// sealer (#2068).
    pub(crate) writes: crate::writes::Writes,
    /// Writes and reverts handed to tasks of their own, which a shutdown
    /// waits for.
    pub(crate) handoffs: tokio_util::task::TaskTracker,
    /// A request's panic, which ends the session (#2203).
    pub(crate) panics: crate::contain::Panics,
    /// On a session attached to the TUI that offers writes, the TUI's
    /// approval queue and the desk it lends (#2185).
    pub(crate) attached: Option<crate::tools::write_attached::Attached<T>>,
    /// On a session attached to the TUI, where its calls go for the agent
    /// pane and where the TUI's armed state is read (#2185).
    pub(crate) pane: Option<crate::board::Pane>,
}

/// The points a principal watches, each with the number of its sessions
/// holding it: a stdio server's one session, or all of a listener's (#2185).
pub(crate) type Points = Arc<Mutex<HashMap<Point, usize>>>;

/// One session fewer holding `point` among the principal's `points`.
pub(crate) fn let_go(points: &Points, point: &Point) {
    let mut points = points.lock().unwrap_or_else(PoisonError::into_inner);
    if points.get_mut(point).is_some_and(|n| {
        *n = n.saturating_sub(1);
        *n == 0
    }) {
        points.remove(point);
    }
}

/// What a principal's sessions share beyond the hub and the bucket: the
/// minute gates on broadcasts and surveys, and the points its cap counts.
/// A stdio server has its own; a listener's sessions share one (#2185).
#[derive(Default, Clone)]
pub(crate) struct Gates {
    /// When the last broad Who-Is went.
    pub(crate) broad: Arc<Mutex<Option<Instant>>>,
    /// Router queries' findings and the last one's claim.
    pub(crate) routers: Arc<Mutex<Routers>>,
    /// When the last BBMD survey began.
    pub(crate) bbmd_asked: Arc<Mutex<Option<Instant>>>,
    pub(crate) points: Points,
}

/// Counts a listen stream while it lives.
pub(crate) struct Listening<'a>(&'a watch::Sender<usize>);

impl Drop for Listening<'_> {
    fn drop(&mut self) {
        self.0.send_modify(|n| *n = n.saturating_sub(1));
    }
}

/// The routers known from Who-Is-Router-To-Network queries.
#[derive(Debug, Clone, Default)]
pub(crate) struct Routers {
    /// When the last query went: a claim taken before it is sent, which
    /// holds off the next for a minute whether or not it finished.
    pub(crate) asked: Option<Instant>,
    /// When the last query that finished went, and the routers it heard
    /// (at most [`ROUTER_CAP`]).
    pub(crate) heard: Option<(Instant, Vec<DiscoveredRouter>)>,
}

/// The most routers kept, the size of a router table.
pub(crate) const ROUTER_CAP: usize = 256;

/// One device's object names and descriptions as read.
#[derive(Debug, Clone)]
pub(crate) struct CachedNames {
    /// When the first was read; the whole entry goes after [`LIST_TTL`].
    pub(crate) at: Instant,
    pub(crate) objects: HashMap<ObjectIdentifier, Names>,
}

/// An object's name and description, kept already cleaned and capped
/// ([`crate::untrusted::clean`]): the raw device text is never stored.
#[derive(Debug, Clone, Default)]
pub(crate) struct Names {
    pub(crate) name: Option<Cleaned>,
    /// `None` until read; then the description, if the device gave one.
    pub(crate) description: Option<Option<Cleaned>>,
}

/// Device text after [`crate::untrusted::clean`], kept long enough to
/// match on, and whether that cap cut it.
#[derive(Debug, Clone)]
pub(crate) struct Cleaned {
    pub(crate) text: String,
    pub(crate) truncated: bool,
}

/// A device's Object_List as read so far.
#[derive(Debug, Clone)]
pub(crate) struct CachedList {
    /// When it was read whole, or, for one read element by element, when
    /// that read last went on: when it completed, once it has.
    pub(crate) at: Instant,
    /// The entries read, in device order.
    pub(crate) objects: Arc<Vec<ObjectIdentifier>>,
    /// Read element by element: the element count, and the next element to
    /// read (past `total` once complete).
    pub(crate) indexed: Option<(u32, u32)>,
    /// Elements the device answered with an error, skipped.
    pub(crate) gaps: Vec<u32>,
}

impl CachedList {
    /// Whether every entry has been read.
    pub(crate) fn complete(&self) -> bool {
        self.indexed.is_none_or(|(total, next)| next > total)
    }

    /// Whether it may still be used: a complete list for [`LIST_TTL`]
    /// after it was read (or its element-by-element read completed), a
    /// partial one for [`PARTIAL_TTL`] after its read last went on.
    pub(crate) fn fresh(&self) -> bool {
        let ttl = if self.complete() {
            LIST_TTL
        } else {
            PARTIAL_TTL
        };
        self.at.elapsed() < ttl
    }
}

/// How long an object list read for one page serves the next ones.
pub(crate) const LIST_TTL: Duration = Duration::from_secs(300);

/// How long an element-by-element read is kept after it last went on,
/// for the next call to go on from; past it, the read starts again.
pub(crate) const PARTIAL_TTL: Duration = Duration::from_secs(600);

/// The most devices whose lists are kept; past it the oldest goes.
pub(crate) const LIST_CAP: usize = 64;

/// One tool call's budget and tally.
pub(crate) struct Call {
    pub(crate) started: Instant,
    /// Past this, a call that pages returns what it has.
    pub(crate) deadline: Instant,
    pub(crate) account: Arc<Account>,
    timeouts: AtomicU32,
    /// The device the call reached, by address and instance, for the read
    /// log.
    device: Mutex<Option<(DeviceAddress, u32)>>,
}

impl Call {
    pub(crate) fn new(budget: Duration, account: Arc<Account>) -> Self {
        let started = Instant::now();
        Self {
            started,
            deadline: started + budget,
            account,
            timeouts: AtomicU32::new(0),
            device: Mutex::new(None),
        }
    }

    /// True once the call's budget is spent.
    pub(crate) fn out_of_time(&self) -> bool {
        Instant::now() >= self.deadline
    }

    /// Count a request that timed out.
    pub(crate) fn timed_out(&self) {
        self.timeouts.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn reached(&self, address: &DeviceAddress, instance: u32) {
        *self.device.lock().unwrap_or_else(PoisonError::into_inner) =
            Some((address.clone(), instance));
    }

    pub(crate) fn device(&self) -> Option<(DeviceAddress, u32)> {
        self.device
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// The `cost` block every result carries.
    pub(crate) fn cost(&self) -> Value {
        let ms = |d: Duration| u64::try_from(d.as_millis()).unwrap_or(u64::MAX);
        json!({
            "requests": self.account.requests(),
            "elapsed_ms": ms(self.started.elapsed()),
            "throttled_ms": ms(self.account.throttled()),
            "timeouts": self.timeouts.load(Ordering::Relaxed),
        })
    }
}

/// A device argument: an instance, `dnet:instance`, or an address (`ip:port`,
/// `dnet:0xMAC` and the target grammar's other station forms), as a string,
/// or an instance as a number.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub(crate) enum DeviceArg {
    Number(u32),
    Text(String),
}

impl DeviceArg {
    pub(crate) fn text(&self) -> String {
        match self {
            Self::Number(n) => n.to_string(),
            Self::Text(text) => text.clone(),
        }
    }
}

/// A device the policy lets the call reach, from the discovery table.
pub(crate) struct Device<T: TransportPort + 'static> {
    pub(crate) instance: u32,
    /// Where it answers: what the caches key on (#2560).
    pub(crate) address: DeviceAddress,
    /// Its network, 0 for the local one.
    pub(crate) network: u16,
    pub(crate) entry: DiscoveredDevice,
    /// The handle the call reads through, pinned to `entry`'s address
    /// however the call named the device: a request never follows the
    /// instance to an address the policy didn't check (#2560).
    pub(crate) handle: bacnet_ops::Device<T>,
}

/// A handle pinned to `entry`, a row of the discovery table, for a call
/// that names devices by instance: an instance another row claims too is
/// refused as ambiguous, naming both, and the handle never follows the
/// instance elsewhere (#2560). Sends nothing.
pub(crate) async fn pinned<T: TransportPort + 'static>(
    client: &Client<T>,
    entry: &DiscoveredDevice,
) -> Result<bacnet_ops::Device<T>, OpsError> {
    let instance = entry.object_identifier.instance_number();
    client.low_level().get_device(instance).await?;
    client.device(entry).await
}

const DISCOVER_HINT: &str = "call bacnet_discover first (with a range or network that includes it)";

/// The ambiguity error for `instance`, claimed by every row of `claims`.
fn ambiguous(instance: u32, claims: &[DiscoveredDevice]) -> OpsError {
    OpsError::Ambiguous {
        instance,
        candidates: claims.iter().filter_map(DeviceAddress::of).collect(),
    }
}

impl<T: TransportPort + 'static> Ctx<T> {
    /// The client, or a refusal in passive mode.
    pub(crate) fn client(&self) -> Result<&Client<T>, ToolError> {
        self.client.as_ref().ok_or_else(|| {
            ToolError::refused(
                "passive",
                "the server is passive: it builds no client and sends nothing",
            )
        })
    }

    /// The device `arg` names, if the discovery table holds it and the
    /// policy allows it, with the handle the call reads through. Targets
    /// are numeric: a device is never found by name. An instance names the
    /// one address claiming it, `dnet:instance` the one claiming it on that
    /// network, and an address (#2560) the device at it. Nothing is sent to
    /// find one (the server's client looks for no device), and an instance
    /// two addresses claim is refused, naming both. The handle is pinned to
    /// the row the policy checks, so no request in the call goes elsewhere.
    pub(crate) async fn device(
        &self,
        call: &Call,
        arg: &DeviceArg,
    ) -> Result<Device<T>, ToolError> {
        let client = self.client()?;
        let text = arg.text();
        let unknown = || ToolError::UnknownDevice {
            device: text.clone(),
            hint: DISCOVER_HINT.into(),
        };
        // Named as written: `7`, `2:7` or an address.
        let failed = |error: OpsError| match error {
            OpsError::UnknownDevice { .. } => unknown(),
            OpsError::Ambiguous { candidates, .. } => ToolError::ambiguous(&text, &candidates),
            OpsError::Parse { reason, .. } => ToolError::invalid("device", reason),
            error => ToolError::Failed(format!("{text}: {error}")),
        };
        let parsed = target::parse_target(&text).map_err(|e| ToolError::invalid("device", e))?;
        let entry = match parsed {
            Target::Instance(instance) => match client.low_level().get_device(instance).await {
                Ok(Some(entry)) => entry,
                Ok(None) => return Err(unknown()),
                Err(error) => return Err(failed(error.into())),
            },
            Target::Routed { dnet, instance } => {
                let claims = self.claims(instance, Some(dnet)).await;
                match claims.as_slice() {
                    [] => return Err(unknown()),
                    [entry] => entry.clone(),
                    several => return Err(failed(ambiguous(instance, several))),
                }
            }
            Target::Mac(_) | Target::RoutedMac { .. } => {
                // A handle from an address sends nothing on this client,
                // which never looks for a device; the table says what is
                // there.
                let handle = client.device(text.as_str()).await.map_err(failed)?;
                let at = handle.address();
                client
                    .low_level()
                    .get_device_at(at.network(), at.mac())
                    .await
                    .ok_or_else(unknown)?
            }
            Target::Bip6(_) => {
                return Err(ToolError::invalid(
                    "device",
                    "a B/IPv6 node is named by its VMAC (bip6:0x...) or its instance",
                ));
            }
        };
        let handle = client.device(&entry).await.map_err(failed)?;
        let address = handle.address();
        let device = Device {
            instance: entry.object_identifier.instance_number(),
            network: address.network(),
            address,
            entry,
            handle,
        };
        self.allow(&device.address, device.instance)?;
        self.remember(&device.entry);
        call.reached(&device.address, device.instance);
        Ok(device)
    }

    /// The rows of the discovery table claiming `instance`, on `network`
    /// when given (the client's own network, once it knows the number, is
    /// the local one, as its handles key it), ordered by address.
    pub(crate) async fn claims(
        &self,
        instance: u32,
        network: Option<u16>,
    ) -> Vec<DiscoveredDevice> {
        let Some(client) = &self.client else {
            return Vec::new();
        };
        let own = client.low_level().network_number();
        let local = |network: u16| if Some(network) == own { 0 } else { network };
        let mut claims: Vec<DiscoveredDevice> = client
            .low_level()
            .discovered_devices()
            .await
            .into_iter()
            .filter(|row| row.object_identifier.instance_number() == instance)
            .filter(|row| {
                network.is_none_or(|n| local(row.source_network.unwrap_or(0)) == local(n))
            })
            .collect();
        // One station per address in its local form: a routed row on this
        // network the table hasn't folded into the local one yet (#2572) is
        // no second claim. The sort is stable, so the local row is kept.
        let at = |row: &DiscoveredDevice| DeviceAddress::of(row).map(|a| a.localized(own));
        claims.sort_by_key(at);
        claims.dedup_by_key(|row| at(row));
        claims
    }

    /// Note a device of the discovery table for the subscription check: its
    /// address claims its instance now.
    pub(crate) fn remember(&self, found: &DiscoveredDevice) {
        if let Some(address) = DeviceAddress::of(found) {
            let instance = found.object_identifier.instance_number();
            self.known
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(address, instance);
        }
    }

    /// Whether `instance` names one known device the policy lets the server
    /// reach, without waiting for the discovery table.
    pub(crate) fn reach_known(&self, instance: u32) -> bool {
        self.reach_check(instance).is_ok()
    }

    /// [`reach_known`](Self::reach_known), saying why not: no address in
    /// the discovery table claims the instance, two or more do (a resource
    /// names a device by its instance alone), or the policy leaves out the
    /// one that does.
    pub(crate) fn reach_check(&self, instance: u32) -> Result<(), ToolError> {
        let claims: Vec<DeviceAddress> = {
            let known = self.known.lock().unwrap_or_else(PoisonError::into_inner);
            let mut claims: Vec<DeviceAddress> = known
                .iter()
                .filter(|(_, claimed)| **claimed == instance)
                .map(|(address, _)| address.clone())
                .collect();
            claims.sort();
            claims
        };
        match claims.as_slice() {
            [] => Err(ToolError::UnknownDevice {
                device: instance.to_string(),
                hint: DISCOVER_HINT.into(),
            }),
            [address] => self.allow(address, instance),
            several => {
                let list: Vec<String> = several.iter().map(ToString::to_string).collect();
                Err(ToolError::refused(
                    "ambiguous-device",
                    format!(
                        "device {instance} is claimed by {}; a resource names one device",
                        list.join(" and ")
                    ),
                ))
            }
        }
    }

    /// The one known address claiming `instance`; `None` when none or two
    /// or more do.
    pub(crate) fn known_at(&self, instance: u32) -> Option<DeviceAddress> {
        let known = self.known.lock().unwrap_or_else(PoisonError::into_inner);
        let mut claims = known
            .iter()
            .filter(|(_, claimed)| **claimed == instance)
            .map(|(address, _)| address);
        match (claims.next(), claims.next()) {
            (Some(address), None) => Some(address.clone()),
            _ => None,
        }
    }

    /// The known instances in reach, in order: each claimed by one address
    /// the policy allows.
    pub(crate) fn known_in_reach(&self) -> Vec<u32> {
        let mut instances: Vec<u32> = self
            .known
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .values()
            .copied()
            .collect();
        instances.sort_unstable();
        instances.dedup();
        instances.retain(|instance| self.reach_check(*instance).is_ok());
        instances
    }

    /// Keep [`known`](Self::known) in step with the discovery table until
    /// the server closes: a snapshot, then its events (a snapshot again
    /// after a burst the receiver missed), one entry per address. It also
    /// lets `bacnet_watch` points go once nobody has asked for them in
    /// [`TOOL_IDLE`].
    pub(crate) async fn follow_devices(&self) {
        let Some(client) = &self.client else {
            return;
        };
        let mut events = client.device_events();
        let mut resync = true;
        let mut sweep = tokio::time::interval(Duration::from_secs(60));
        loop {
            if resync {
                let devices = client.devices().await;
                self.known
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .clear();
                for found in &devices {
                    self.remember(found);
                }
                resync = false;
            }
            let event = tokio::select! {
                () = self.closing.cancelled() => return,
                event = events.recv() => event,
                _ = sweep.tick() => {
                    self.subs.expire_tool(TOOL_IDLE);
                    // A stream acknowledged but never begun (its
                    // acknowledgment failed, or the client cancelled first)
                    // leaves a reservation no later one may come to clear.
                    self.subs.prune();
                    continue;
                }
            };
            match event {
                Ok(event) if event.kind == DeviceEventKind::Lost => {
                    if let Some(address) = DeviceAddress::of(&event.device) {
                        self.known
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .remove(&address);
                    }
                }
                Ok(event) => self.remember(&event.device),
                Err(RecvError::Lagged(_)) => resync = true,
                Err(RecvError::Closed) => return,
            }
        }
    }

    /// Count a listen stream until the guard drops, unless `max` run
    /// already.
    pub(crate) fn listening(&self, max: usize) -> Option<Listening<'_>> {
        let mut taken = false;
        self.listening.send_if_modified(|n| {
            taken = *n < max;
            if taken {
                *n += 1;
            }
            taken
        });
        taken.then(|| Listening(&self.listening))
    }

    /// Refuse a network the policy leaves out.
    pub(crate) fn allow_network(&self, network: u16) -> Result<(), ToolError> {
        if self.policy.allows_network(network) {
            return Ok(());
        }
        Err(ToolError::refused(
            "network-not-allowed",
            format!("network {network} is not in this server's [mcp] networks"),
        ))
    }

    /// Refuse a device the policy leaves out: its address's network, or
    /// its instance.
    pub(crate) fn allow(&self, address: &DeviceAddress, instance: u32) -> Result<(), ToolError> {
        self.allow_network(address.network())?;
        if !self.policy.allows_device(instance) {
            return Err(ToolError::refused(
                "device-not-allowed",
                format!("device {instance} is not in this server's [mcp] devices"),
            ));
        }
        Ok(())
    }

    /// A new account for one call: its requests take the principal's
    /// tokens, and the request-rate meter counts them as an agent's.
    pub(crate) fn account(&self) -> Arc<Account> {
        Arc::new(Account::agent(Some(Arc::clone(&self.bucket))))
    }

    /// Record a finished call in the read log, as a summary line.
    pub(crate) fn log_read(&self, tool: &str, call: &Call, outcome: &str) {
        let requests = call.account.requests();
        let reached = call.device();
        let device = reached.as_ref().map(|(_, instance)| *instance);
        let address = reached.as_ref().map(|(address, _)| address.to_string());
        tracing::info!(
            target: "bacnet_mcp::reads",
            tool,
            device,
            address,
            requests,
            outcome,
            "read call"
        );
        if let Some((address, instance)) = reached {
            let mut reads = self.reads.lock().unwrap_or_else(PoisonError::into_inner);
            let entry = reads.entry(address).or_insert((instance, 0, 0));
            entry.0 = instance;
            entry.1 += 1;
            entry.2 += u64::from(requests);
        }
    }

    /// Log the read counts per device, at shutdown.
    pub(crate) fn log_summary(&self) {
        let reads = self.reads.lock().unwrap_or_else(PoisonError::into_inner);
        for (address, (device, calls, requests)) in reads.iter() {
            tracing::info!(
                target: "bacnet_mcp::reads",
                principal = %self.principal,
                device,
                address = %address,
                calls,
                requests,
                "read summary"
            );
        }
    }

    /// The `limits` block every result carries.
    pub(crate) fn limits(&self, page: Option<usize>) -> Value {
        let rates = &self.policy.rates;
        let mut limits = json!({
            "rate_per_s": rates.global,
            "principal_rate_per_s": rates.principal,
            "per_device_outstanding": 1,
        });
        if let Some(page) = page {
            limits["page"] = json!(page);
        }
        limits
    }
}
