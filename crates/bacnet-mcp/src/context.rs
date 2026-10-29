//! What every tool call shares: the engine, the policy, the cursors and
//! caches ([`Ctx`]), and one call's budget and cost ([`Call`]).

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use bacnet_client::client::{DeviceEventKind, DiscoveredRouter};
use bacnet_client::discovery::DiscoveredDevice;
use bacnet_ops::engine::Engine;
use bacnet_ops::limits::{Account, TokenBucket};
use bacnet_ops::target::{self, Target};
use bacnet_ops::watch::Hub;
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
    /// The engine; `None` in passive mode, where no client is built.
    pub(crate) engine: Option<Arc<Engine<T>>>,
    pub(crate) policy: Policy,
    /// The one principal of a stdio server: the process's OS user.
    pub(crate) principal: String,
    pub(crate) cursors: Cursors,
    /// The principal's token bucket.
    pub(crate) bucket: Arc<TokenBucket>,
    /// When the last broad Who-Is went.
    pub(crate) broad: Mutex<Option<Instant>>,
    /// Object lists read, by device, for paging without reading again.
    pub(crate) lists: Mutex<HashMap<u32, CachedList>>,
    /// One lock a device, held while its list is read, so two calls on one
    /// device take turns and the second goes on from where the first
    /// stopped.
    pub(crate) list_locks: Mutex<HashMap<u32, Arc<tokio::sync::Mutex<()>>>>,
    /// Calls and requests per device, logged as a summary at shutdown.
    pub(crate) reads: Mutex<BTreeMap<u32, (u64, u64)>>,
    /// Object names and descriptions read by `bacnet_find_object`, by
    /// device, so its later pages don't read them again.
    pub(crate) names: Mutex<HashMap<u32, CachedNames>>,
    /// What Who-Is-Router-To-Network queries found, and when the last one
    /// went; at most one goes a minute.
    pub(crate) routers: Mutex<Routers>,
    /// When the last BBMD survey of `bacnet_network_map` began; at most
    /// one goes a minute.
    pub(crate) bbmd_asked: Mutex<Option<Instant>>,
    /// The watch hub; `None` in passive mode.
    pub(crate) hub: Option<Hub<T>>,
    /// What the principal watches, and the subscriptions waiting for their
    /// streams.
    pub(crate) subs: Arc<Subs>,
    /// Cancelled when the server shuts down gracefully: listen streams
    /// complete.
    pub(crate) closing: CancellationToken,
    /// Listen streams running.
    pub(crate) listening: watch::Sender<usize>,
    /// The discovery table's devices and their networks, kept for the
    /// subscription acknowledgment, which the SDK asks for synchronously.
    pub(crate) known: Mutex<HashMap<u32, u16>>,
    /// Device summaries served to `bacnet://device/{instance}`.
    pub(crate) described: Cache,
    /// Alarm pages served to the alarm resources.
    pub(crate) alarms: Cache,
    /// The write session, the plan grant's budget and the confirmation
    /// sealer (#2068).
    pub(crate) writes: crate::writes::Writes,
    /// Writes and reverts handed to tasks of their own, which a shutdown
    /// waits for.
    pub(crate) handoffs: tokio_util::task::TaskTracker,
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
    /// that read began (not when it last went on).
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
    /// after it was read, a partial one for [`PARTIAL_TTL`] after its read
    /// began.
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

/// How long an element-by-element read may go on across calls before it
/// starts again.
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
    /// The device the call reached, for the read log.
    device: Mutex<Option<u32>>,
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

    pub(crate) fn reached(&self, device: u32) {
        *self.device.lock().unwrap_or_else(PoisonError::into_inner) = Some(device);
    }

    pub(crate) fn device(&self) -> Option<u32> {
        *self.device.lock().unwrap_or_else(PoisonError::into_inner)
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

/// A device argument: an instance, `dnet:instance` or `dnet:0xMAC`, as a
/// string, or an instance as a number.
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
pub(crate) struct Device {
    pub(crate) instance: u32,
    /// Its network, 0 for the local one.
    pub(crate) network: u16,
    pub(crate) entry: DiscoveredDevice,
}

const DISCOVER_HINT: &str = "call bacnet_discover first (with a range or network that includes it)";

impl<T: TransportPort + 'static> Ctx<T> {
    /// The engine, or a refusal in passive mode.
    pub(crate) fn engine(&self) -> Result<&Engine<T>, ToolError> {
        self.engine.as_deref().ok_or_else(|| {
            ToolError::refused(
                "passive",
                "the server is passive: it builds no client and sends nothing",
            )
        })
    }

    /// The device `arg` names, if the discovery table holds it and the
    /// policy allows it. Targets are numeric: a device is never found by
    /// name, and an address is no device.
    pub(crate) async fn device(&self, call: &Call, arg: &DeviceArg) -> Result<Device, ToolError> {
        let engine = self.engine()?;
        let text = arg.text();
        let unknown = || ToolError::UnknownDevice {
            device: text.clone(),
            hint: DISCOVER_HINT.into(),
        };
        let parsed = target::parse_target(&text).map_err(|e| ToolError::invalid("device", e))?;
        let entry = match parsed {
            Target::Instance(instance) => engine.device(instance).await.ok_or_else(unknown)?,
            Target::Routed { dnet, instance } => engine
                .device(instance)
                .await
                .filter(|found| found.source_network == Some(dnet))
                .ok_or_else(unknown)?,
            Target::RoutedMac { dnet, mac } => engine
                .devices()
                .await
                .into_iter()
                .find(|found| {
                    found.source_network == Some(dnet)
                        && found.source_address.as_deref() == Some(&mac[..])
                })
                .ok_or_else(unknown)?,
            Target::Mac(_) | Target::Bip6(_) => {
                return Err(ToolError::invalid(
                    "device",
                    "a device instance, dnet:instance or dnet:0xMAC, not an address",
                ));
            }
        };
        let device = Device {
            instance: entry.object_identifier.instance_number(),
            network: entry.source_network.unwrap_or(0),
            entry,
        };
        self.allow(device.instance, device.network)?;
        self.remember(&device.entry);
        call.reached(device.instance);
        Ok(device)
    }

    /// Note a device of the discovery table for the subscription check.
    pub(crate) fn remember(&self, found: &DiscoveredDevice) {
        let instance = found.object_identifier.instance_number();
        let network = found.source_network.unwrap_or(0);
        self.known
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(instance, network);
    }

    /// Whether `instance` is a known device the policy lets the server
    /// reach, without waiting for the discovery table.
    pub(crate) fn reach_known(&self, instance: u32) -> bool {
        self.reach_check(instance).is_ok()
    }

    /// [`reach_known`](Self::reach_known), saying why not: the device
    /// isn't in the discovery table, or the policy leaves it out.
    pub(crate) fn reach_check(&self, instance: u32) -> Result<(), ToolError> {
        let network = self
            .known
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&instance)
            .copied();
        let network = network.ok_or_else(|| ToolError::UnknownDevice {
            device: instance.to_string(),
            hint: DISCOVER_HINT.into(),
        })?;
        self.allow(instance, network)
    }

    /// The known devices in reach, in order.
    pub(crate) fn known_in_reach(&self) -> Vec<u32> {
        let known = self.known.lock().unwrap_or_else(PoisonError::into_inner);
        let mut devices: Vec<u32> = known
            .iter()
            .filter(|(instance, network)| self.allow(**instance, **network).is_ok())
            .map(|(instance, _)| *instance)
            .collect();
        devices.sort_unstable();
        devices
    }

    /// Keep [`known`](Self::known) in step with the discovery table until
    /// the server closes: a snapshot, then its events (a snapshot again
    /// after a burst the receiver missed). It also lets `bacnet_watch`
    /// points go once nobody has asked for them in [`TOOL_IDLE`].
    pub(crate) async fn follow_devices(&self) {
        let Some(engine) = &self.engine else {
            return;
        };
        let mut events = engine.device_events();
        let mut resync = true;
        let mut sweep = tokio::time::interval(Duration::from_secs(60));
        loop {
            if resync {
                let devices = engine.devices().await;
                let mut known = self.known.lock().unwrap_or_else(PoisonError::into_inner);
                known.clear();
                for found in &devices {
                    known.insert(
                        found.object_identifier.instance_number(),
                        found.source_network.unwrap_or(0),
                    );
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
                    let instance = event.device.object_identifier.instance_number();
                    self.known
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .remove(&instance);
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

    /// Refuse a device or network the policy leaves out.
    pub(crate) fn allow(&self, instance: u32, network: u16) -> Result<(), ToolError> {
        self.allow_network(network)?;
        if !self.policy.allows_device(instance) {
            return Err(ToolError::refused(
                "device-not-allowed",
                format!("device {instance} is not in this server's [mcp] devices"),
            ));
        }
        Ok(())
    }

    /// Record a finished call in the read log, as a summary line.
    pub(crate) fn log_read(&self, tool: &str, call: &Call, outcome: &str) {
        let requests = call.account.requests();
        let device = call.device();
        tracing::info!(
            target: "bacnet_mcp::reads",
            tool,
            device,
            requests,
            outcome,
            "read call"
        );
        if let Some(device) = device {
            let mut reads = self.reads.lock().unwrap_or_else(PoisonError::into_inner);
            let entry = reads.entry(device).or_default();
            entry.0 += 1;
            entry.1 += u64::from(requests);
        }
    }

    /// Log the read counts per device, at shutdown.
    pub(crate) fn log_summary(&self) {
        let reads = self.reads.lock().unwrap_or_else(PoisonError::into_inner);
        for (device, (calls, requests)) in reads.iter() {
            tracing::info!(
                target: "bacnet_mcp::reads",
                principal = %self.principal,
                device,
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
