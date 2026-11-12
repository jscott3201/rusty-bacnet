//! Custom reader instrumentation keeps the native point's atomic install hook.
use super::fixture::*;
use super::*;
use bacnet_objects::property_metadata::PropertyMetadata;
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
    /// Take `oid` out of `db` to wrap it. The removal is a real one, so a
    /// Schedule wrapped this way would give up its held slots (#2264); only
    /// Access Points are wrapped, and the wrapper lends no `ScheduleRunner`.
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
    fn cov(&self) -> Option<&dyn bacnet_objects::traits::CovReporting> {
        self.inner.cov()
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
    fn property_metadata(&self) -> Cow<'_, [PropertyMetadata]> {
        let mut rows = self.inner.property_metadata().into_owned();
        for (p, _) in &self.overrides {
            if !rows.iter().any(|row| row.property_identifier == *p) {
                rows.push(PropertyMetadata::new(
                    *p,
                    bacnet_objects::property_metadata::PropertyConformance::Optional,
                    None,
                    bacnet_objects::property_metadata::PropertyWriteCapability::ReadOnly,
                ));
            }
        }
        Cow::Owned(rows)
    }
    fn timed(&mut self) -> Option<&mut dyn bacnet_objects::traits::Timed> {
        self.inner.timed()
    }
    fn local_input(&mut self) -> Option<&mut dyn bacnet_objects::traits::LocalInput> {
        self.inner.local_input()
    }
    fn access_events(&mut self) -> Option<&mut dyn bacnet_objects::traits::AccessEvents> {
        if self.supports {
            self.inner.access_events()
        } else {
            None
        }
    }
    fn event_reporting(&self) -> Option<&dyn bacnet_objects::traits::EventReporting> {
        Some(self)
    }
    fn event_reporting_mut(&mut self) -> Option<&mut dyn bacnet_objects::traits::EventReporting> {
        Some(self)
    }
}

/// Only the evaluation reaches the wrapped object, as before the capability
/// existed: every other event hook keeps its default.
impl bacnet_objects::traits::EventReporting for Boundary {
    fn evaluate_intrinsic_reporting(
        &mut self,
        now: bacnet_time::Instant,
    ) -> Option<bacnet_objects::event::TransitionOutcome> {
        if self
            .panic_event
            .as_ref()
            .is_some_and(|flag| flag.swap(false, Ordering::SeqCst))
        {
            panic!("authorization post-commit test panic");
        }
        self.inner
            .event_reporting_mut()
            .and_then(|reporting| reporting.evaluate_intrinsic_reporting(now))
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
