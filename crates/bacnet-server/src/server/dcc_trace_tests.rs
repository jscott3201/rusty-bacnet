use super::*;
use std::collections::BTreeMap;
use tracing::{span, Event, Metadata, Subscriber};

#[derive(Clone)]
pub(in crate::server) struct Capture(pub Arc<std::sync::Mutex<Vec<BTreeMap<String, String>>>>);

impl Default for Capture {
    fn default() -> Self {
        // tracing-core 0.1.36's single-dispatch callsite registration consults
        // the current thread. Other parallel tests have no local subscriber and
        // can otherwise cache "never" for our shared cold event callsite. Keep a
        // second, disabled dispatcher registered (NOT installed as a default)
        // so registration consults all live dispatchers. It retains no events.
        static REGISTRATION_PEER: std::sync::OnceLock<tracing::Dispatch> =
            std::sync::OnceLock::new();
        REGISTRATION_PEER
            .get_or_init(|| tracing::Dispatch::new(tracing::subscriber::NoSubscriber::default()));
        Self(Arc::default())
    }
}

impl Subscriber for Capture {
    fn register_callsite(&self, _: &'static Metadata<'static>) -> tracing::subscriber::Interest {
        // Tests install many different scoped dispatches concurrently. Always
        // consult this dispatch's filter rather than caching its answer globally.
        tracing::subscriber::Interest::sometimes()
    }
    fn max_level_hint(&self) -> Option<tracing::metadata::LevelFilter> {
        Some(tracing::metadata::LevelFilter::DEBUG)
    }
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.target() == "bacnet_server::dcc_outcome"
    }
    fn new_span(&self, _: &span::Attributes<'_>) -> span::Id {
        span::Id::from_u64(1)
    }
    fn record(&self, _: &span::Id, _: &span::Record<'_>) {}
    fn record_follows_from(&self, _: &span::Id, _: &span::Id) {}
    fn event(&self, event: &Event<'_>) {
        if !self.enabled(event.metadata()) {
            return;
        }
        assert_eq!(*event.metadata().level(), tracing::Level::DEBUG);
        struct Visitor(BTreeMap<String, String>);
        impl tracing::field::Visit for Visitor {
            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                self.0.insert(field.name().into(), format!("{value:?}"));
            }
            fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
                self.0.insert(field.name().into(), value.into());
            }
        }
        let mut visitor = Visitor(BTreeMap::new());
        event.record(&mut visitor);
        self.0.lock().unwrap().push(visitor.0);
    }
    fn enter(&self, _: &span::Id) {}
    fn exit(&self, _: &span::Id) {}
}

#[test]
fn dcc_outcomes_bound_claimed_addresses() {
    use dcc_outcomes::{DccMetadata, DccOutcome, DccOutcomes};
    let counters = DccOutcomes::default();
    let capture = Capture::default();
    for length in [0, 32, 33, 255, 65536] {
        let mac = vec![0xab; length];
        let route = NpduAddress {
            network: 7,
            mac_address: MacAddr::from_slice(&mac),
        };
        for source in [None, Some(&route)] {
            tracing::subscriber::with_default(capture.clone(), || {
                counters.record(
                    DccOutcome::Accepted,
                    DccMetadata::default(),
                    42,
                    &mac,
                    source,
                );
            });
            let records = capture.0.lock().unwrap();
            let event = records.last().unwrap();
            assert_eq!(event["claimed_source_mac"], "ab".repeat(length.min(32)));
            assert_eq!(event["source_mac_truncated"], (length > 32).to_string());
            assert_eq!(
                event["claimed_sadr"],
                if source.is_some() {
                    "ab".repeat(length.min(32))
                } else {
                    String::new()
                }
            );
            assert_eq!(
                event["sadr_truncated"],
                (source.is_some() && length > 32).to_string()
            );
            assert_eq!(
                event["source_kind"],
                if source.is_some() {
                    "claimed_routed"
                } else {
                    "claimed_direct"
                }
            );
            assert_eq!(
                event.get("claimed_snet"),
                source.map(|_| "7".to_owned()).as_ref()
            );
        }
    }
    assert_eq!(counters.snapshot().accepted_total, 10);
    assert_eq!(capture.0.lock().unwrap().len(), 10);
}

#[test]
fn dcc_outcomes_concurrent_quiescent_totals_and_events() {
    let counters = dcc_outcomes::DccOutcomes::default();
    let capture = Capture::default();
    let barrier = std::sync::Barrier::new(5);
    std::thread::scope(|scope| {
        for _ in 0..5 {
            let counters = &counters;
            let barrier = &barrier;
            let capture = capture.clone();
            scope.spawn(move || {
                tracing::subscriber::with_default(capture, || {
                    barrier.wait();
                    for _ in 0..100 {
                        counters.record(
                            dcc_outcomes::DccOutcome::Accepted,
                            dcc_outcomes::DccMetadata::default(),
                            1,
                            &[],
                            None,
                        );
                    }
                });
            });
        }
    });
    assert_eq!(counters.snapshot().accepted_total, 500);
    assert_eq!(capture.0.lock().unwrap().len(), 500);
}
