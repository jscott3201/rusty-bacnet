//! Custom reader instrumentation keeps the native point's atomic install hook.
use super::fixture::*;
use super::*;
use std::borrow::Cow;
use std::sync::atomic::AtomicUsize;

pub(super) struct Boundary {
    pub inner: Box<dyn BACnetObject>,
    pub overrides: Vec<(P, PropertyValue)>,
    pub supports: bool,
    pub mode_reads: Arc<AtomicUsize>,
    pub on_mode: Option<Arc<dyn Fn() + Send + Sync>>,
    pub panic_event: Option<Arc<AtomicBool>>,
}

impl Boundary {
    pub(super) fn wrap(db: &mut ObjectDatabase, oid: ObjectIdentifier) -> Self {
        Self {
            inner: db.remove(&oid).unwrap().unwrap(),
            overrides: vec![],
            supports: true,
            mode_reads: Arc::new(AtomicUsize::new(0)),
            on_mode: None,
            panic_event: None,
        }
    }
}

impl BACnetObject for Boundary {
    fn supports_cov(&self) -> bool {
        self.inner.supports_cov()
    }
    fn supports_subscribe_cov_property(&self) -> bool {
        self.inner.supports_subscribe_cov_property()
    }
    fn supports_cov_property(&self, p: P) -> bool {
        self.inner.supports_cov_property(p)
    }
    fn cov_reported_properties(&self) -> &'static [bacnet_objects::traits::CovReportedProperty] {
        self.inner.cov_reported_properties()
    }
    fn object_identifier(&self) -> ObjectIdentifier {
        self.inner.object_identifier()
    }
    fn object_name(&self) -> &str {
        self.inner.object_name()
    }
    fn read_property(&self, p: P, index: Option<u32>) -> Result<PropertyValue, Error> {
        if p == P::AUTHORIZATION_MODE {
            self.mode_reads.fetch_add(1, Ordering::SeqCst);
            if let Some(callback) = &self.on_mode {
                callback();
            }
        }
        if let Some((_, value)) = self.overrides.iter().find(|(property, _)| *property == p) {
            return Ok(value.clone());
        }
        self.inner.read_property(p, index)
    }
    fn write_property(
        &mut self,
        p: P,
        index: Option<u32>,
        value: PropertyValue,
        priority: Option<u8>,
    ) -> Result<(), Error> {
        self.inner.write_property(p, index, value, priority)
    }
    fn property_list(&self) -> Cow<'static, [P]> {
        let mut properties = self.inner.property_list().into_owned();
        for (p, _) in &self.overrides {
            if !properties.contains(p) {
                properties.push(*p);
            }
        }
        Cow::Owned(properties)
    }
    fn bind_clock_internal(&mut self, clock: Option<Arc<dyn bacnet_objects::clock::ClockReader>>) {
        self.inner.bind_clock_internal(clock);
    }
    fn report_access_input_internal(&mut self, input: AccessControlInput) -> Result<(), Error> {
        self.inner.report_access_input_internal(input)
    }
    fn supports_access_event_batch_internal(&self) -> bool {
        self.supports && self.inner.supports_access_event_batch_internal()
    }
    fn record_access_event_batch_internal(
        &mut self,
        reports: &[AccessEventReport],
    ) -> Result<Vec<CommittedAccessEvent>, Error> {
        self.inner.record_access_event_batch_internal(reports)
    }
    fn evaluate_intrinsic_reporting(&mut self) -> Option<bacnet_objects::event::TransitionOutcome> {
        if self
            .panic_event
            .as_ref()
            .is_some_and(|flag| flag.swap(false, Ordering::SeqCst))
        {
            panic!("authorization post-commit test panic");
        }
        self.inner.evaluate_intrinsic_reporting()
    }
}

pub(super) fn override_values(
    db: &mut ObjectDatabase,
    oid: ObjectIdentifier,
    overrides: Vec<(P, PropertyValue)>,
) {
    let mut wrapper = Boundary::wrap(db, oid);
    wrapper.overrides = overrides;
    db.add(Box::new(wrapper)).unwrap();
}
