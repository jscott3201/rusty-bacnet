//! The macro that gives an object type its reporting persistence, compiled
//! with the `event-reporting` feature; see the [parent module](super).

/// Implement reporting persistence for `$object`, which holds a
/// [`ReportingStore`](super::ReportingStore) in `reporting_store`. It gets
/// `with_reporting_persistence`, `written_reporting`,
/// `clear_written_reporting` and `wait_for_reporting_saves`, and
/// [`DurableWrites`](crate::durable::DurableWrites) staging its reporting
/// settings; its `durable_writes` returns `self.durable_capability()`, and
/// its `write_property` offers each write to [`admit`](super::admit) once the
/// array index is checked.
///
/// - `impl_reporting_persistence!(T)`: an object that holds the kept rows in
///   the usual fields (`event_detector`, `event_detection_enable`,
///   `event_history`) and its Tags storage in `profile`. It also gets the
///   three setters, [`KeepsReporting`](super::KeepsReporting), and staging
///   of its Tags beside its reporting settings. Its algorithm parameters
///   are its detector's
///   ([`AlgorithmDetector`](super::parameters::AlgorithmDetector)), and a
///   staged write of one is checked on a detector of that type.
/// - `impl_reporting_persistence!(T, states: field)`: the same, for an
///   object whose Number_Of_States is in `field`, which bounds the
///   Alarm_Values entries it takes.
/// - `impl_reporting_persistence!(T, without_tags)`: an object that
///   implements `KeepsReporting` itself, has no Tags storage, and guards
///   its own setters with [`setter_gives_way`](super::setter_gives_way).
macro_rules! impl_reporting_persistence {
    ($object:ty $(, states: $states:ident)?) => {
        $crate::write_config::reporting::impl_reporting_persistence!(@builder $object);

        impl $object {
            /// Set Notification_Class. On an object keeping its reporting
            /// settings, a value a write saved wins, as for the next two.
            pub fn set_notification_class(&mut self, class: u32) {
                if !self.kept_over_setter(bacnet_types::enums::PropertyIdentifier::NOTIFICATION_CLASS) {
                    self.event_detector.notification_class = class;
                }
            }

            /// Set Event_Enable.
            pub fn set_event_enable(&mut self, enable: bacnet_types::bitstring::EventTransitionBits) {
                if !self.kept_over_setter(bacnet_types::enums::PropertyIdentifier::EVENT_ENABLE) {
                    self.event_detector.event_enable = enable;
                }
            }

            /// Set Notify_Type.
            pub fn set_notify_type(&mut self, notify_type: bacnet_types::enums::NotifyType) {
                if !self.kept_over_setter(bacnet_types::enums::PropertyIdentifier::NOTIFY_TYPE) {
                    self.event_detector.notify_type = notify_type;
                }
            }

            /// Set the description string. On an object keeping its
            /// reporting settings, a Description a write saved wins
            /// (#2364).
            pub fn set_description(&mut self, desc: impl Into<String>) {
                if !self.kept_over_setter(bacnet_types::enums::PropertyIdentifier::DESCRIPTION) {
                    self.description = desc.into();
                }
            }

            fn kept_over_setter(&self, property: bacnet_types::enums::PropertyIdentifier) -> bool {
                $crate::write_config::reporting::setter_gives_way(&self.reporting_store, self.oid, property)
            }

            /// The staging this object lends: its Tags and its reporting
            /// settings, whichever it keeps.
            #[cfg(feature = "durable-writes")]
            fn durable_capability(&mut self) -> Option<&mut dyn $crate::durable::DurableWrites> {
                let keeps = self.profile.capability().is_some() || self.reporting_store.attached();
                keeps.then_some(self as _)
            }
        }

        impl $crate::write_config::reporting::KeepsReporting for $object {
            fn reporting_store(&mut self) -> &mut $crate::write_config::reporting::ReportingStore {
                &mut self.reporting_store
            }

            fn reporting_store_ref(&self) -> &$crate::write_config::reporting::ReportingStore {
                &self.reporting_store
            }

            fn reporting_rows(&self) -> $crate::write_config::reporting::ReportingRows {
                $crate::write_config::reporting::ReportingRows {
                    notification_class: self.event_detector.notification_class,
                    event_enable: self.event_detector.event_enable,
                    notify_type: self.event_detector.notify_type,
                    event_detection_enable: self.event_detection_enable,
                    options: self.event_history.options.clone(),
                    description: self.description.clone(),
                    algorithm: $crate::write_config::reporting::parameters::AlgorithmDetector::parameters(
                        &self.event_detector,
                    ),
                }
            }

            fn restore_reporting_rows(&mut self, saved: &$crate::write_config::ReportingWriteConfig) {
                $crate::write_config::reporting::RowFields {
                    notification_class: &mut self.event_detector.notification_class,
                    event_enable: &mut self.event_detector.event_enable,
                    notify_type: &mut self.event_detector.notify_type,
                    event_detection_enable: &mut self.event_detection_enable,
                    options: &mut self.event_history.options,
                    description: &mut self.description,
                }
                .restore(saved);
                $crate::write_config::reporting::parameters::AlgorithmDetector::restore_parameters(
                    &mut self.event_detector,
                    &saved.algorithm,
                );
            }

            fn parameter_after(
                &self,
                states: Option<u32>,
                rows: &mut $crate::write_config::AlgorithmWriteConfig,
                write: &$crate::durable::PendingWrite,
            ) -> Result<(), bacnet_types::error::Error> {
                // The detector's type picks the family's write path.
                $crate::write_config::reporting::parameters::after(
                    &self.event_detector,
                    states,
                    rows,
                    write,
                )
            }

            $(fn number_of_states(&self) -> Option<u32> {
                Some(self.$states)
            })?
        }

        impl $crate::durable::DurableWrites for $object {
            fn stage_write(
                &mut self,
                property: bacnet_types::enums::PropertyIdentifier,
                array_index: Option<u32>,
                value: &bacnet_types::primitives::PropertyValue,
            ) -> $crate::durable::StageStep {
                self.stage_writes(&[$crate::durable::PendingWrite {
                    property,
                    array_index,
                    value: value.clone(),
                }])
            }

            fn stage_writes(
                &mut self,
                writes: &[$crate::durable::PendingWrite],
            ) -> $crate::durable::StageStep {
                let tags = $crate::durable::DurableWrites::stage_writes(&mut self.profile, writes);
                if matches!(tags, $crate::durable::StageStep::Busy(_)) {
                    return tags;
                }
                let reporting = $crate::write_config::reporting::stage_writes(self, writes);
                $crate::write_config::reporting::joined(tags, reporting, |tags| {
                    $crate::durable::DurableWrites::release_staged_write(&mut self.profile, tags);
                })
            }

            fn release_staged_write(&mut self, staged: &$crate::durable::SaveWait) {
                $crate::durable::DurableWrites::release_staged_write(&mut self.profile, staged);
                $crate::write_config::reporting::release(self, staged);
            }

            fn has_staged_write(&self) -> bool {
                $crate::durable::DurableWrites::has_staged_write(&self.profile)
                    || $crate::write_config::reporting::has_staged(&self.reporting_store)
            }

            fn settle_forgotten_writes(&mut self) -> Option<$crate::durable::SaveWait> {
                let tags = $crate::durable::DurableWrites::settle_forgotten_writes(&mut self.profile);
                match (tags, $crate::write_config::reporting::settle(self)) {
                    (Some(tags), Some(reporting)) => Some(tags.and(reporting)),
                    (tags, reporting) => tags.or(reporting),
                }
            }

            fn expire_staged(&mut self, now: core::time::Duration) {
                $crate::durable::DurableWrites::expire_staged(&mut self.profile, now);
                $crate::write_config::reporting::expire(self, now);
            }
        }
    };

    ($object:ty, without_tags) => {
        $crate::write_config::reporting::impl_reporting_persistence!(@builder $object);

        impl $object {
            /// The staging this object lends, once it keeps its reporting
            /// settings.
            #[cfg(feature = "durable-writes")]
            fn durable_capability(&mut self) -> Option<&mut dyn $crate::durable::DurableWrites> {
                self.reporting_store.attached().then_some(self as _)
            }
        }

        impl $crate::durable::DurableWrites for $object {
            fn stage_write(
                &mut self,
                property: bacnet_types::enums::PropertyIdentifier,
                array_index: Option<u32>,
                value: &bacnet_types::primitives::PropertyValue,
            ) -> $crate::durable::StageStep {
                self.stage_writes(&[$crate::durable::PendingWrite {
                    property,
                    array_index,
                    value: value.clone(),
                }])
            }

            fn stage_writes(
                &mut self,
                writes: &[$crate::durable::PendingWrite],
            ) -> $crate::durable::StageStep {
                $crate::write_config::reporting::stage_writes(self, writes)
            }

            fn release_staged_write(&mut self, staged: &$crate::durable::SaveWait) {
                $crate::write_config::reporting::release(self, staged);
            }

            fn has_staged_write(&self) -> bool {
                $crate::write_config::reporting::has_staged(&self.reporting_store)
            }

            fn settle_forgotten_writes(&mut self) -> Option<$crate::durable::SaveWait> {
                $crate::write_config::reporting::settle(self)
            }

            fn expire_staged(&mut self, now: core::time::Duration) {
                $crate::write_config::reporting::expire(self, now);
            }
        }
    };

    (@builder $object:ty) => {
        impl $object {
            /// Keep the reporting settings peers write (Notification_Class,
            /// Event_Enable, Notify_Type, Event_Detection_Enable,
            /// Event_Message_Texts_Config and the Event_Algorithm_Inhibit
            /// pair) in `persistence`, and a written Description (#2364),
            /// serving what is saved there for this object, if anything
            /// (#2331). An analog, binary or multi-state Input, Output or
            /// Value keeps its event algorithm parameters there too, and
            /// its detector starts with the saved ones: Time_Delay and
            /// Time_Delay_Normal on all nine, High_Limit, Low_Limit,
            /// Deadband and Limit_Enable on the analog ones, Alarm_Value on
            /// a Binary Input or Value, and Alarm_Values on a Multi-state
            /// Input or Value.
            ///
            /// A write of one of them is then saved before the object serves
            /// it, and refused with DEVICE / OPERATIONAL_PROBLEM, the old
            /// value kept, when the save fails; a Description past 16 KiB is
            /// refused with RESOURCES / NO_SPACE_TO_WRITE_PROPERTY. A row a
            /// write saved wins over its setter until
            /// [`clear_written_reporting`](Self::clear_written_reporting).
            /// Fails when storage can't be loaded, holds another kind of
            /// configuration, or holds what no write leaves: a value the
            /// object refuses on the wire, or an algorithm parameter its
            /// type doesn't keep. One wire rule doesn't fail it: saved
            /// Alarm_Values naming a state past this object's
            /// Number_Of_States are restored as written. The state count
            /// and State_Text aren't kept, so a peer that added a state and
            /// listed it, or an application that now builds the object with
            /// fewer states, leaves such a list. The object serves it, and
            /// a write of the same list is still refused. A Multi-state
            /// Input goes on alarming on the entries within its states. A
            /// Multi-state Value reports CONFIGURATION_ERROR and is in
            /// FAULT, raising no OFFNORMAL on any listed state, until a
            /// list within its states is written or the states are there
            /// again.
            pub fn with_reporting_persistence(
                self,
                persistence: alloc::sync::Arc<dyn $crate::write_config::WriteConfigPersistence>,
            ) -> Result<Self, bacnet_types::error::Error> {
                $crate::write_config::reporting::attach(self, persistence)
            }

            /// The reporting settings, Description and event algorithm
            /// parameters storage keeps because a write set them, now or
            /// before a restart; all `None` without storage.
            pub fn written_reporting(&self) -> $crate::write_config::ReportingWriteConfig {
                $crate::write_config::reporting::written(self)
            }

            /// Drop every reporting setting a write saved, a saved
            /// Description and every saved event algorithm parameter, so
            /// the setters apply again, and at the next start what the
            /// application configures. The values served stay until a
            /// setter or a write changes them. Saves before it returns; a
            /// save that fails keeps the written settings and returns
            /// storage's error. Does nothing without storage. Call it before
            /// the object joins the database, as the setters are: it waits
            /// for its save.
            pub fn clear_written_reporting(&mut self) -> Result<(), bacnet_types::error::Error> {
                $crate::write_config::reporting::clear(self)
            }

            /// Block until the reporting saves queued so far have run.
            /// Dropping the object waits for them too.
            pub fn wait_for_reporting_saves(&self) {
                self.reporting_store.wait_idle();
            }
        }
    };
}

pub(crate) use impl_reporting_persistence;
