//! Cumulative `u64` totals over a portable core's drained `u32` counts.
//!
//! The cores of `bacnet-router-core` count as the link cores do: `u32`
//! counts that saturate between takes and start again from zero when
//! taken. The hosted [`BACnetRouter`](crate::router::BACnetRouter) and
//! [`NetworkLayer`](crate::layer::NetworkLayer) take them after each call
//! that may have counted, add them to these totals, and read the totals
//! without a lock, so their users see counts since start.

/// Declares a totals type: one `AtomicU64` per named field of a core's
/// counts, `add` for the counts a take returned, and `load` for a copy as
/// the hosted counters type. Both types must name every field: the sizes
/// are checked at compile time, so a count a core adds fails the build
/// until it is listed here and totalled.
///
/// Each total saturates at `u64::MAX`. Additions release and loads
/// acquire, so a reader who sees a total also sees what the call that
/// counted it did before its counts were added.
macro_rules! totals {
    (
        $(#[$meta:meta])*
        $name:ident: $counts:ty => $totals:ty { $($field:ident),+ $(,)? }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Default)]
        pub(crate) struct $name {
            $($field: std::sync::atomic::AtomicU64,)+
        }

        impl $name {
            /// How many counts it keeps.
            const FIELDS: usize = [$(stringify!($field)),+].len();

            /// Add the counts a take returned to the totals, saturating.
            pub(crate) fn add(&self, counts: &$counts) {
                use std::sync::atomic::Ordering;
                $(
                    if counts.$field != 0 {
                        let added = u64::from(counts.$field);
                        let _ = self.$field.try_update(
                            Ordering::Release,
                            Ordering::Relaxed,
                            |total| Some(total.saturating_add(added)),
                        );
                    }
                )+
            }

            /// The totals, as the hosted counters.
            pub(crate) fn load(&self) -> $totals {
                use std::sync::atomic::Ordering;
                let mut totals = <$totals>::default();
                $(totals.$field = self.$field.load(Ordering::Acquire);)+
                totals
            }
        }

        // The core's counts are all `u32`s and the totals all `u64`s, so
        // their sizes say how many there are.
        const _: () = assert!(
            std::mem::size_of::<$counts>() == $name::FIELDS * std::mem::size_of::<u32>(),
            "total every count the core keeps"
        );
        const _: () = assert!(
            std::mem::size_of::<$totals>() == $name::FIELDS * std::mem::size_of::<u64>(),
            "publish every total"
        );
    };
}

pub(crate) use totals;
