//! [`WritePolicy`]: the priorities a [`Client`](crate::Client)'s writes may
//! use, and [`WriteDenied`], what it refused.

use std::fmt;

use super::Priority;
use crate::label::{kebab, object_label};
use crate::reading::PropertyRef;

/// Which command priorities a [`Client`](crate::Client)'s
/// [`write`](crate::Device::write) and [`relinquish`](crate::Device::relinquish)
/// may use. The check is local and costs no traffic: a refused call fails
/// with [`OpsError::Denied`](crate::OpsError::Denied) and sends nothing.
///
/// The default refuses four priorities: 1 and 2, life safety, and 5,
/// critical equipment control, which Table 19-1 sets apart for those
/// systems, and 6, which a binary object's minimum on and off time logic
/// keeps for itself (Clause 19.2.3). A program writing at one of them is
/// nearly always a mistake, so one that means it allows each it uses, once:
///
/// ```
/// use bacnet_ops::{Priority, WritePolicy};
///
/// let policy = WritePolicy::default();
/// assert!(!policy.allows(Priority::MANUAL_LIFE_SAFETY));
/// assert!(policy.allows(Priority::MANUAL_OPERATOR));
/// let life_safety = policy.allow_priorities([Priority::MANUAL_LIFE_SAFETY]);
/// assert!(life_safety.allows(Priority::MANUAL_LIFE_SAFETY));
/// assert!(!life_safety.allows(Priority::MINIMUM_ON_OFF));
/// ```
///
/// How far it reaches: it checks the priority a WriteProperty carries as
/// its own parameter, and nothing else. A priority that travels inside a
/// value goes unchecked: the priority field of a BACnetLightingCommand
/// written to a Lighting Output's Lighting_Command or to a Channel
/// (Clauses 12.54 and 12.53), a Lighting_Command_Default_Priority, the
/// Priority_For_Writing of a Schedule, Loop, Access Point or Staging object
/// (Clauses 19.2.5 to 19.2.8), and the actions of a Command object (Clause
/// 19.2.4): the policy doesn't look inside values. A write with no priority to a property that may be
/// commandable ([`Commandability::Optional`](super::Commandability::Optional))
/// carries none, so no priority is checked, though the device takes it at
/// 16 if the property is commandable: a policy that refused 16 would not
/// see it.
///
/// Nothing else is guarded: no read before a write, no rule about
/// Out_Of_Service. The low-level `BACnetClient` has no policy at all, and
/// the `bacnet` tool's writes go through it under the tool's own tiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WritePolicy {
    /// One bit a priority, bit `n - 1` for priority `n`: set when refused.
    refused: u16,
}

impl WritePolicy {
    /// The priorities the default refuses: 1, 2, 5 and 6.
    pub const GUARDED: [Priority; 4] = [
        Priority::MANUAL_LIFE_SAFETY,
        Priority::AUTOMATIC_LIFE_SAFETY,
        Priority::CRITICAL_EQUIPMENT,
        Priority::MINIMUM_ON_OFF,
    ];

    /// The same policy, allowing `priorities` as well.
    #[must_use]
    pub fn allow_priorities(self, priorities: impl IntoIterator<Item = Priority>) -> Self {
        let allowed = priorities
            .into_iter()
            .fold(0, |bits, priority| bits | bit(priority));
        Self {
            refused: self.refused & !allowed,
        }
    }

    /// Whether a write or relinquish at `priority` may go.
    pub fn allows(self, priority: Priority) -> bool {
        self.refused & bit(priority) == 0
    }

    /// Refuse a write (or relinquish) of `property` at `priority` unless
    /// the policy allows it.
    pub(crate) fn check(
        self,
        property: PropertyRef,
        priority: Priority,
        relinquish: bool,
    ) -> Result<(), WriteDenied> {
        if self.allows(priority) {
            return Ok(());
        }
        Err(WriteDenied {
            property,
            priority,
            relinquish,
        })
    }
}

impl Default for WritePolicy {
    /// Refusing [`GUARDED`](Self::GUARDED).
    fn default() -> Self {
        Self {
            refused: Self::GUARDED
                .iter()
                .fold(0, |bits, priority| bits | bit(*priority)),
        }
    }
}

fn bit(priority: Priority) -> u16 {
    1 << priority.index()
}

/// A write or relinquish a [`WritePolicy`] refused, so nothing was sent:
/// what it named, and the priority to allow for it to go.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct WriteDenied {
    /// The property it named.
    pub property: PropertyRef,
    /// The priority refused: allow it with
    /// [`WritePolicy::allow_priorities`].
    pub priority: Priority,
    /// It was a relinquish rather than a write.
    pub relinquish: bool,
}

impl fmt::Display for WriteDenied {
    /// `not sent: the write policy refuses priority 1 (life safety,
    /// manual) for the write of analog-output 1 present-value; allow it
    /// with WritePolicy::allow_priorities`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let what = if self.relinquish {
            "relinquish"
        } else {
            "write"
        };
        let PropertyRef {
            object,
            property,
            index,
        } = self.property;
        let index = index.map(|i| format!("[{i}]")).unwrap_or_default();
        write!(
            f,
            "not sent: the write policy refuses priority {} ({}) for the {what} of {} {}{index}; \
             allow it with WritePolicy::allow_priorities",
            self.priority,
            self.priority.purpose(),
            object_label(object),
            kebab(property),
        )
    }
}

impl std::error::Error for WriteDenied {}

#[cfg(test)]
mod tests {
    use bacnet_types::enums::{ObjectType, PropertyIdentifier};
    use bacnet_types::primitives::ObjectIdentifier;

    use super::*;

    fn pv() -> PropertyRef {
        let ao = ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 1).unwrap();
        PropertyRef::new(ao, PropertyIdentifier::PRESENT_VALUE)
    }

    /// The default refuses 1, 2, 5 and 6 and nothing else; allowing one
    /// opens that one alone.
    #[test]
    fn the_default_guards_four_priorities() {
        let policy = WritePolicy::default();
        let refused: Vec<u8> = Priority::all()
            .filter(|p| !policy.allows(*p))
            .map(Priority::get)
            .collect();
        assert_eq!(refused, [1, 2, 5, 6]);
        let five = policy.allow_priorities([Priority::CRITICAL_EQUIPMENT]);
        let refused: Vec<u8> = Priority::all()
            .filter(|p| !five.allows(*p))
            .map(Priority::get)
            .collect();
        assert_eq!(refused, [1, 2, 6]);
        let open = policy.allow_priorities(WritePolicy::GUARDED);
        assert!(Priority::all().all(|p| open.allows(p)));
        // Allowing what is already allowed changes nothing.
        assert_eq!(policy.allow_priorities([Priority::LOWEST]), policy);
    }

    #[test]
    fn a_refusal_names_the_target_and_the_way_to_allow_it() {
        let denied = WritePolicy::default()
            .check(pv(), Priority::MINIMUM_ON_OFF, true)
            .unwrap_err();
        assert_eq!(denied.priority, Priority::MINIMUM_ON_OFF);
        let text = denied.to_string();
        assert!(
            text.contains("priority 6 (minimum on/off time)")
                && text.contains("relinquish of analog-output 1 present-value")
                && text.contains("allow_priorities"),
            "{text}"
        );
        assert!(
            WritePolicy::default()
                .check(pv(), Priority::MANUAL_OPERATOR, false)
                .is_ok()
        );
    }
}
