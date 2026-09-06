//! Discovery rate-limiting, duplicate suppression, and directed response handling.
//!
//! Controls response amplification for unconfirmed discovery services (Who-Is
//! and Who-Has) per ASHRAE 135-2020 Clauses 16.9 and 16.10.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bytes::BytesMut;

use bacnet_encoding::apdu::{encode_apdu, Apdu, UnconfirmedRequest as UnconfirmedRequestPdu};
use bacnet_encoding::npdu::NpduAddress;
use bacnet_network::layer::{NetworkLayer, ReceivedApdu};
use bacnet_objects::database::ObjectDatabase;
use bacnet_services::who_has::{WhoHasObject, WhoHasRequest};
use bacnet_services::who_is::{IAmRequest, WhoIsRequest};
use bacnet_transport::port::TransportPort;
use bacnet_types::enums::{NetworkPriority, ObjectType, UnconfirmedServiceChoice};
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
use bacnet_types::MacAddr;
use tokio::sync::RwLock;

use super::{BACnetServer, IAmBroadcaster, ServerConfig};

/// Configuration policy for discovery rate limiting and duplicate suppression.
#[derive(Debug, Clone)]
pub struct DiscoveryPolicy {
    /// Maximum discovery responses emitted globally per second (default 64).
    pub max_responses_per_sec_global: u32,
    /// Maximum discovery response bytes emitted globally per second (default 65,536).
    pub max_bytes_per_sec_global: usize,
    /// Maximum discovery responses emitted per source per second (default 16).
    pub max_responses_per_sec_per_source: u32,
    /// Maximum discovery response bytes emitted per source per second (default 16,384).
    pub max_bytes_per_sec_per_source: usize,
    /// Maximum burst capacity for global discovery responses (default 64).
    pub global_burst_capacity: u32,
    /// Maximum burst capacity for per-source discovery responses (default 16).
    pub source_burst_capacity: u32,
    /// Headroom reserved exclusively for configured reserved sources (default 8).
    pub reserved_capacity: u32,
    /// Sources permitted to consume from reserved capacity.
    pub reserved_sources: Vec<MacAddr>,
    /// Coalescing window for duplicate discovery request suppression (default 200ms).
    pub coalesce_window: Duration,
    /// Prefer directed unicast responses over network broadcast (default true).
    pub prefer_directed_responses: bool,
    /// Maximum number of distinct discovery sources tracked concurrently (default 256).
    pub max_tracked_sources: usize,
}

impl Default for DiscoveryPolicy {
    fn default() -> Self {
        Self {
            max_responses_per_sec_global: 64,
            max_bytes_per_sec_global: 65_536,
            max_responses_per_sec_per_source: 16,
            max_bytes_per_sec_per_source: 16_384,
            global_burst_capacity: 64,
            source_burst_capacity: 16,
            reserved_capacity: 8,
            reserved_sources: Vec::new(),
            coalesce_window: Duration::from_millis(200),
            prefer_directed_responses: true,
            max_tracked_sources: 256,
        }
    }
}

impl DiscoveryPolicy {
    /// Create an unlimited discovery policy with all rate limits disabled.
    pub fn unlimited() -> Self {
        Self {
            max_responses_per_sec_global: u32::MAX,
            max_bytes_per_sec_global: usize::MAX,
            max_responses_per_sec_per_source: u32::MAX,
            max_bytes_per_sec_per_source: usize::MAX,
            global_burst_capacity: u32::MAX,
            source_burst_capacity: u32::MAX,
            reserved_capacity: 0,
            reserved_sources: Vec::new(),
            coalesce_window: Duration::ZERO,
            prefer_directed_responses: false,
            max_tracked_sources: 256,
        }
    }

    /// Return a sanitized copy with valid burst and capacity bounds.
    pub fn sanitized(&self) -> Self {
        let mut policy = self.clone();
        if policy.max_responses_per_sec_global > 0
            && policy.max_responses_per_sec_global != u32::MAX
        {
            policy.global_burst_capacity = policy.global_burst_capacity.max(1);
        }
        if policy.max_responses_per_sec_per_source > 0
            && policy.max_responses_per_sec_per_source != u32::MAX
        {
            policy.source_burst_capacity = policy.source_burst_capacity.max(1);
        }
        policy.reserved_capacity = policy.reserved_capacity.min(policy.global_burst_capacity);
        policy.max_tracked_sources = policy.max_tracked_sources.max(1);
        policy
    }
}

/// Operational counters for discovery requests and responses.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct DiscoveryCounters {
    /// Total Who-Is requests received.
    pub who_is_received: u64,
    /// Total Who-Has requests received.
    pub who_has_received: u64,
    /// Total I-Am responses sent.
    pub i_am_sent: u64,
    /// Total I-Have responses sent.
    pub i_have_sent: u64,
    /// Total response bytes sent for discovery.
    pub response_bytes_sent: u64,
    /// Total duplicate or cached requests coalesced without sending responses.
    pub requests_coalesced: u64,
    /// Responses throttled by per-source rate or burst limits.
    pub responses_throttled_source: u64,
    /// Responses throttled by global rate or reserved capacity limits.
    pub responses_throttled_global: u64,
    /// Total directed unicast discovery responses sent.
    pub directed_responses_sent: u64,
}

#[derive(Debug, Default)]
pub(crate) struct AtomicDiscoveryCounters {
    pub(crate) who_is_received: AtomicU64,
    pub(crate) who_has_received: AtomicU64,
    pub(crate) i_am_sent: AtomicU64,
    pub(crate) i_have_sent: AtomicU64,
    pub(crate) response_bytes_sent: AtomicU64,
    pub(crate) requests_coalesced: AtomicU64,
    pub(crate) responses_throttled_source: AtomicU64,
    pub(crate) responses_throttled_global: AtomicU64,
    pub(crate) directed_responses_sent: AtomicU64,
}

impl AtomicDiscoveryCounters {
    pub(crate) fn snapshot(&self) -> DiscoveryCounters {
        DiscoveryCounters {
            who_is_received: self.who_is_received.load(Ordering::Relaxed),
            who_has_received: self.who_has_received.load(Ordering::Relaxed),
            i_am_sent: self.i_am_sent.load(Ordering::Relaxed),
            i_have_sent: self.i_have_sent.load(Ordering::Relaxed),
            response_bytes_sent: self.response_bytes_sent.load(Ordering::Relaxed),
            requests_coalesced: self.requests_coalesced.load(Ordering::Relaxed),
            responses_throttled_source: self.responses_throttled_source.load(Ordering::Relaxed),
            responses_throttled_global: self.responses_throttled_global.load(Ordering::Relaxed),
            directed_responses_sent: self.directed_responses_sent.load(Ordering::Relaxed),
        }
    }

    #[inline]
    pub(crate) fn inc_who_is(&self) {
        self.who_is_received.fetch_add(1, Ordering::Relaxed);
    }

    #[inline]
    pub(crate) fn inc_who_has(&self) {
        self.who_has_received.fetch_add(1, Ordering::Relaxed);
    }

    #[inline]
    pub(crate) fn inc_coalesced(&self) {
        self.requests_coalesced.fetch_add(1, Ordering::Relaxed);
    }

    #[inline]
    pub(crate) fn inc_throttled_source(&self) {
        self.responses_throttled_source
            .fetch_add(1, Ordering::Relaxed);
    }

    #[inline]
    pub(crate) fn inc_throttled_global(&self) {
        self.responses_throttled_global
            .fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_sent(&self, bytes: usize, directed: bool, is_who_is: bool) {
        if is_who_is {
            self.i_am_sent.fetch_add(1, Ordering::Relaxed);
        } else {
            self.i_have_sent.fetch_add(1, Ordering::Relaxed);
        }
        self.response_bytes_sent
            .fetch_add(bytes as u64, Ordering::Relaxed);
        if directed {
            self.directed_responses_sent.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// Target of a Who-Has query.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum WhoHasTarget {
    Id(ObjectIdentifier),
    Name(String),
}

/// Network-aware identity of a discovery requester.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct SourceKey {
    pub(crate) network: u16,
    pub(crate) mac: MacAddr,
}

impl SourceKey {
    pub(crate) fn from_received(received: &ReceivedApdu) -> Self {
        if let Some(ref net) = received.source_network {
            if (1..=0xFFFE).contains(&net.network) && !net.mac_address.is_empty() {
                return Self {
                    network: net.network,
                    mac: MacAddr::from_slice(&net.mac_address),
                };
            }
        }
        Self {
            network: 0,
            mac: received.source_mac.clone(),
        }
    }
}

#[derive(Debug)]
struct SourceState {
    tokens: f64,
    byte_tokens: f64,
    last_refill: Instant,
    last_activity: Instant,
    last_who_is: Option<Instant>,
    last_who_has: HashMap<WhoHasTarget, Instant>,
}

fn refill_bucket(
    tokens: &mut f64,
    byte_tokens: &mut f64,
    last_refill: &mut Instant,
    rate: u32,
    byte_rate: usize,
    burst: u32,
    now: Instant,
) {
    if rate == u32::MAX {
        *tokens = burst as f64;
        *byte_tokens = byte_rate as f64;
        *last_refill = now;
        return;
    }
    let elapsed = now.saturating_duration_since(*last_refill).as_secs_f64();
    if elapsed > 0.0 {
        *tokens = (*tokens + elapsed * (rate as f64)).min(burst as f64);
        *byte_tokens = (*byte_tokens + elapsed * (byte_rate as f64)).min(byte_rate as f64);
        *last_refill = now;
    }
}

impl SourceState {
    fn refill(&mut self, policy: &DiscoveryPolicy, now: Instant) {
        refill_bucket(
            &mut self.tokens,
            &mut self.byte_tokens,
            &mut self.last_refill,
            policy.max_responses_per_sec_per_source,
            policy.max_bytes_per_sec_per_source,
            policy.source_burst_capacity,
            now,
        );
    }

    fn has_capacity(&self, policy: &DiscoveryPolicy, bytes: usize) -> bool {
        if policy.max_responses_per_sec_per_source == u32::MAX {
            return true;
        }
        self.byte_tokens >= bytes as f64 && self.tokens >= 1.0
    }

    fn deduct(&mut self, policy: &DiscoveryPolicy, bytes: usize) {
        if policy.max_responses_per_sec_per_source != u32::MAX {
            self.tokens -= 1.0;
            self.byte_tokens -= bytes as f64;
        }
    }
}

#[derive(Debug)]
struct DiscoveryState {
    global_tokens: f64,
    global_byte_tokens: f64,
    global_last_refill: Instant,
    last_broadcast_who_is: Option<Instant>,
    last_broadcast_who_has: HashMap<WhoHasTarget, Instant>,
    negative_who_has: HashMap<WhoHasTarget, Instant>,
    sources: HashMap<SourceKey, SourceState>,
}

impl DiscoveryState {
    fn refill_global(&mut self, policy: &DiscoveryPolicy, now: Instant) {
        refill_bucket(
            &mut self.global_tokens,
            &mut self.global_byte_tokens,
            &mut self.global_last_refill,
            policy.max_responses_per_sec_global,
            policy.max_bytes_per_sec_global,
            policy.global_burst_capacity,
            now,
        );
    }

    fn has_global_capacity(
        &self,
        policy: &DiscoveryPolicy,
        is_reserved: bool,
        bytes: usize,
    ) -> bool {
        if policy.max_responses_per_sec_global == u32::MAX {
            return true;
        }
        if self.global_byte_tokens < bytes as f64 {
            return false;
        }
        let required = if is_reserved {
            1.0
        } else {
            1.0 + policy.reserved_capacity as f64
        };
        self.global_tokens >= required
    }

    fn deduct_global(&mut self, policy: &DiscoveryPolicy, bytes: usize) {
        if policy.max_responses_per_sec_global != u32::MAX {
            self.global_tokens -= 1.0;
            self.global_byte_tokens -= bytes as f64;
        }
    }

    fn get_or_create_source_mut(
        &mut self,
        policy: &DiscoveryPolicy,
        key: &SourceKey,
        now: Instant,
    ) -> &mut SourceState {
        ensure_source_capacity(self, policy, now);
        let src = self
            .sources
            .entry(key.clone())
            .or_insert_with(|| SourceState {
                tokens: policy.source_burst_capacity as f64,
                byte_tokens: policy.max_bytes_per_sec_per_source as f64,
                last_refill: now,
                last_activity: now,
                last_who_is: None,
                last_who_has: HashMap::new(),
            });
        src.refill(policy, now);
        src
    }

    fn is_coalesced_who_has(
        &self,
        policy: &DiscoveryPolicy,
        target: &WhoHasTarget,
        key: &SourceKey,
        is_broadcast: bool,
        now: Instant,
    ) -> bool {
        if policy.coalesce_window.is_zero() {
            return false;
        }
        let is_recent = |opt: Option<&Instant>| {
            opt.is_some_and(|ts| now.saturating_duration_since(*ts) < policy.coalesce_window)
        };
        is_recent(self.negative_who_has.get(target))
            || (is_broadcast && is_recent(self.last_broadcast_who_has.get(target)))
            || self
                .sources
                .get(key)
                .is_some_and(|s| is_recent(s.last_who_has.get(target)))
    }
}

#[derive(Debug)]
struct AtomicDeviceInstance(AtomicU64);

impl AtomicDeviceInstance {
    const PRESENT_BIT: u64 = 0x1_0000_0000;

    fn new(opt: Option<u32>) -> Self {
        Self(AtomicU64::new(match opt {
            Some(i) => Self::PRESENT_BIT | (i as u64),
            None => 0,
        }))
    }

    fn load(&self, order: Ordering) -> Option<u32> {
        let val = self.0.load(order);
        if val & Self::PRESENT_BIT != 0 {
            Some(val as u32)
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PreCheckDecision {
    Admit,
    OutOfRange,
    Coalesced,
    ThrottledSource,
    ThrottledGlobal,
    DecodeError,
}

/// Thread-safe rate limiter, duplicate coalescer, and negative query cache.
#[derive(Debug)]
pub(crate) struct DiscoveryLimiter {
    policy: DiscoveryPolicy,
    counters: AtomicDiscoveryCounters,
    device_instance: AtomicDeviceInstance,
    state: Mutex<DiscoveryState>,
}

impl DiscoveryLimiter {
    pub(crate) fn new(policy: DiscoveryPolicy, device_instance: Option<u32>) -> Self {
        let now = Instant::now();
        let state = DiscoveryState {
            global_tokens: policy.global_burst_capacity as f64,
            global_byte_tokens: policy.max_bytes_per_sec_global as f64,
            global_last_refill: now,
            last_broadcast_who_is: None,
            last_broadcast_who_has: HashMap::new(),
            negative_who_has: HashMap::new(),
            sources: HashMap::new(),
        };
        Self {
            policy,
            counters: AtomicDiscoveryCounters::default(),
            device_instance: AtomicDeviceInstance::new(device_instance),
            state: Mutex::new(state),
        }
    }

    pub(crate) fn counters(&self) -> DiscoveryCounters {
        self.counters.snapshot()
    }

    #[allow(dead_code)]
    pub(crate) fn policy(&self) -> &DiscoveryPolicy {
        &self.policy
    }

    fn check_instance_in_range(&self, low: Option<u32>, high: Option<u32>) -> bool {
        match (self.device_instance.load(Ordering::Acquire), low, high) {
            (Some(inst), Some(low), Some(high)) => inst >= low && inst <= high,
            (Some(_), _, _) => true,
            (None, _, _) => false,
        }
    }

    pub(crate) fn pre_check_who_is(
        &self,
        req_bytes: &[u8],
        received: &ReceivedApdu,
        now: Instant,
    ) -> PreCheckDecision {
        let who_is = match WhoIsRequest::decode(req_bytes) {
            Ok(w) => w,
            Err(_) => return PreCheckDecision::DecodeError,
        };

        self.counters.inc_who_is();

        if !self.check_instance_in_range(who_is.low_limit, who_is.high_limit) {
            return PreCheckDecision::OutOfRange;
        }

        let is_broadcast = received.is_group || received.link_layer_group;
        let source_key = SourceKey::from_received(received);
        let mut state = self.state.lock().unwrap();

        // 1. Coalescing check
        if !self.policy.coalesce_window.is_zero() {
            let is_recent = |opt: Option<Instant>| {
                opt.is_some_and(|ts| {
                    now.saturating_duration_since(ts) < self.policy.coalesce_window
                })
            };
            if (is_broadcast && is_recent(state.last_broadcast_who_is))
                || state
                    .sources
                    .get(&source_key)
                    .is_some_and(|s| is_recent(s.last_who_is))
            {
                self.counters.inc_coalesced();
                return PreCheckDecision::Coalesced;
            }
        }

        // 2. Refill & Rate limits
        state.refill_global(&self.policy, now);
        let is_reserved = is_source_reserved(&self.policy, &source_key, received);
        const ESTIMATED_I_AM_BYTES: usize = 64;

        if !state.has_global_capacity(&self.policy, is_reserved, ESTIMATED_I_AM_BYTES) {
            self.counters.inc_throttled_global();
            return PreCheckDecision::ThrottledGlobal;
        }

        let policy = self.policy.clone();
        let src = state.get_or_create_source_mut(&policy, &source_key, now);
        if !src.has_capacity(&policy, ESTIMATED_I_AM_BYTES) {
            self.counters.inc_throttled_source();
            return PreCheckDecision::ThrottledSource;
        }

        state.deduct_global(&policy, ESTIMATED_I_AM_BYTES);
        let src = state.get_or_create_source_mut(&policy, &source_key, now);
        src.deduct(&policy, ESTIMATED_I_AM_BYTES);
        src.last_activity = now;
        src.last_who_is = Some(now);
        if is_broadcast && !self.policy.prefer_directed_responses {
            state.last_broadcast_who_is = Some(now);
        }

        PreCheckDecision::Admit
    }

    pub(crate) fn record_i_am_sent(
        &self,
        actual_bytes: usize,
        directed: bool,
        source_mac: &MacAddr,
        source_network: Option<&NpduAddress>,
        now: Instant,
    ) {
        self.counters.record_sent(actual_bytes, directed, true);

        const ESTIMATED_I_AM_BYTES: usize = 64;
        let delta = (actual_bytes as f64) - (ESTIMATED_I_AM_BYTES as f64);
        let mut state = self.state.lock().unwrap();
        if delta != 0.0 {
            if self.policy.max_responses_per_sec_global != u32::MAX {
                state.global_byte_tokens = (state.global_byte_tokens - delta).max(0.0);
            }
            let key = SourceKey {
                network: source_network.map(|n| n.network).unwrap_or(0),
                mac: source_mac.clone(),
            };
            if let Some(src) = state.sources.get_mut(&key) {
                if self.policy.max_responses_per_sec_per_source != u32::MAX {
                    src.byte_tokens = (src.byte_tokens - delta).max(0.0);
                }
            }
        }
        if !directed {
            state.last_broadcast_who_is = Some(now);
        }
    }

    pub(crate) fn pre_check_who_has(
        &self,
        req_bytes: &[u8],
        received: &ReceivedApdu,
        now: Instant,
    ) -> PreCheckDecision {
        let who_has = match WhoHasRequest::decode(req_bytes) {
            Ok(w) => w,
            Err(_) => return PreCheckDecision::DecodeError,
        };

        self.counters.inc_who_has();

        if !self.check_instance_in_range(who_has.low_limit, who_has.high_limit) {
            return PreCheckDecision::OutOfRange;
        }

        let target = match &who_has.object {
            WhoHasObject::Identifier(oid) => WhoHasTarget::Id(*oid),
            WhoHasObject::Name(name) => WhoHasTarget::Name(name.clone()),
        };

        let is_broadcast = received.is_group || received.link_layer_group;
        let source_key = SourceKey::from_received(received);
        let mut state = self.state.lock().unwrap();

        if state.is_coalesced_who_has(&self.policy, &target, &source_key, is_broadcast, now) {
            self.counters.inc_coalesced();
            return PreCheckDecision::Coalesced;
        }

        state.refill_global(&self.policy, now);
        let is_reserved = is_source_reserved(&self.policy, &source_key, received);
        const ESTIMATED_I_HAVE_BYTES: usize = 64;

        if !state.has_global_capacity(&self.policy, is_reserved, ESTIMATED_I_HAVE_BYTES) {
            self.counters.inc_throttled_global();
            return PreCheckDecision::ThrottledGlobal;
        }

        let policy = self.policy.clone();
        let src = state.get_or_create_source_mut(&policy, &source_key, now);
        if !src.has_capacity(&policy, ESTIMATED_I_HAVE_BYTES) {
            self.counters.inc_throttled_source();
            return PreCheckDecision::ThrottledSource;
        }

        src.last_activity = now;
        PreCheckDecision::Admit
    }

    pub(crate) fn is_negative_who_has(&self, target: &WhoHasTarget, now: Instant) -> bool {
        if self.policy.coalesce_window.is_zero() {
            return false;
        }
        let state = self.state.lock().unwrap();
        if let Some(ts) = state.negative_who_has.get(target) {
            now.saturating_duration_since(*ts) < self.policy.coalesce_window
        } else {
            false
        }
    }

    pub(crate) fn record_negative_who_has(&self, target: WhoHasTarget, now: Instant) {
        let mut state = self.state.lock().unwrap();
        if state.negative_who_has.len() > 512 {
            let window = self.policy.coalesce_window;
            state
                .negative_who_has
                .retain(|_, ts| now.saturating_duration_since(*ts) < window);
        }
        state.negative_who_has.insert(target, now);
    }

    pub(crate) fn try_consume_who_has_response(
        &self,
        target: &WhoHasTarget,
        received: &ReceivedApdu,
        response_bytes: usize,
        now: Instant,
    ) -> bool {
        let source_key = SourceKey::from_received(received);
        let mut state = self.state.lock().unwrap();
        let is_broadcast = received.is_group || received.link_layer_group;

        if state.is_coalesced_who_has(&self.policy, target, &source_key, is_broadcast, now) {
            self.counters.inc_coalesced();
            return false;
        }

        state.refill_global(&self.policy, now);
        let is_reserved = is_source_reserved(&self.policy, &source_key, received);

        if !state.has_global_capacity(&self.policy, is_reserved, response_bytes) {
            self.counters.inc_throttled_global();
            return false;
        }

        let policy = self.policy.clone();
        let src = state.get_or_create_source_mut(&policy, &source_key, now);
        if !src.has_capacity(&policy, response_bytes) {
            self.counters.inc_throttled_source();
            return false;
        }

        state.deduct_global(&policy, response_bytes);
        let src = state.get_or_create_source_mut(&policy, &source_key, now);
        src.deduct(&policy, response_bytes);
        src.last_activity = now;
        src.last_who_has.insert(target.clone(), now);
        if is_broadcast && !self.policy.prefer_directed_responses {
            state.last_broadcast_who_has.insert(target.clone(), now);
        }

        true
    }

    pub(crate) fn record_i_have_sent(
        &self,
        actual_bytes: usize,
        directed: bool,
        target: &WhoHasTarget,
        _source_mac: &MacAddr,
        _source_network: Option<&NpduAddress>,
        now: Instant,
    ) {
        self.counters.record_sent(actual_bytes, directed, false);

        if !directed {
            let mut state = self.state.lock().unwrap();
            state.last_broadcast_who_has.insert(target.clone(), now);
        }
    }
}

fn is_source_reserved(
    policy: &DiscoveryPolicy,
    source_key: &SourceKey,
    received: &ReceivedApdu,
) -> bool {
    policy
        .reserved_sources
        .iter()
        .any(|r| r == &source_key.mac || r == &received.source_mac)
}

fn ensure_source_capacity(state: &mut DiscoveryState, policy: &DiscoveryPolicy, now: Instant) {
    if state.sources.len() < policy.max_tracked_sources {
        return;
    }
    state.sources.retain(|_, src| {
        now.saturating_duration_since(src.last_activity) < Duration::from_secs(10)
    });
    while state.sources.len() >= policy.max_tracked_sources {
        let oldest = state
            .sources
            .iter()
            .min_by_key(|(_, src)| src.last_activity)
            .map(|(k, _)| k.clone());
        if let Some(key) = oldest {
            state.sources.remove(&key);
        } else {
            break;
        }
    }
}

pub(crate) async fn broadcast_i_am_from<T: TransportPort + 'static>(
    config: &ServerConfig,
    db: &Arc<RwLock<ObjectDatabase>>,
    network: &Arc<NetworkLayer<T>>,
    limiter: Option<&Arc<DiscoveryLimiter>>,
) -> Result<(), Error> {
    let device_oid = db
        .read()
        .await
        .list_objects()
        .into_iter()
        .find(|oid| oid.object_type() == ObjectType::DEVICE)
        .ok_or_else(|| Error::Encoding("no Device object in database".into()))?;

    let mut service_buf = BytesMut::new();
    IAmRequest {
        object_identifier: device_oid,
        max_apdu_length: config.max_apdu_length,
        segmentation_supported: config.segmentation_supported,
        vendor_id: config.vendor_id,
    }
    .encode(&mut service_buf);

    let mut buf = BytesMut::new();
    encode_apdu(
        &mut buf,
        &Apdu::UnconfirmedRequest(UnconfirmedRequestPdu {
            service_choice: UnconfirmedServiceChoice::I_AM,
            service_request: service_buf.freeze(),
        }),
    )?;

    if let Some(limiter) = limiter {
        limiter.record_i_am_sent(buf.len(), false, &MacAddr::new(), None, Instant::now());
    }

    network
        .broadcast_apdu(&buf, false, NetworkPriority::NORMAL)
        .await
}

impl<T: TransportPort + 'static> BACnetServer<T> {
    /// Broadcast an I-Am for this server's Device object using the bound transport socket.
    pub async fn broadcast_i_am(&self) -> Result<(), Error> {
        broadcast_i_am_from(
            &self.config,
            &self.db,
            &self.network,
            Some(&self.discovery_limiter),
        )
        .await
    }
}

impl<T: TransportPort + 'static> IAmBroadcaster<T> {
    /// Broadcast an I-Am for this server's Device object using the bound transport socket.
    pub async fn broadcast_i_am(&self) -> Result<(), Error> {
        broadcast_i_am_from(&self.config, &self.db, &self.network, None).await
    }
}
