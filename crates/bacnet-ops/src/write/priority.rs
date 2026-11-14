//! [`Priority`]: one of the sixteen command priorities (Clause 19.2).

use std::fmt;
use std::str::FromStr;

use crate::error::OpsError;

/// A command priority, 1 (the highest) to 16 (the lowest).
///
/// A write at a priority puts its value in that slot of a commandable
/// property's Priority_Array, and the property follows the lowest-numbered
/// slot that holds one (Clause 19.2.1). Table 19-1 gives five of them a
/// standard use ([`purpose`](Self::purpose)); the site assigns the rest.
///
/// ```
/// use bacnet_ops::Priority;
///
/// let p8 = Priority::new(8).unwrap();
/// assert_eq!(p8, Priority::MANUAL_OPERATOR);
/// assert_eq!((p8.get(), p8.index()), (8, 7));
/// assert_eq!("16".parse::<Priority>().unwrap(), Priority::LOWEST);
/// assert!(Priority::new(0).is_err() && Priority::new(17).is_err());
/// assert_eq!(Priority::MINIMUM_ON_OFF.purpose(), "minimum on/off time");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Priority(u8);

impl Priority {
    /// How many there are: the length of a Priority_Array.
    pub const COUNT: usize = 16;

    /// 1, manual life safety (Table 19-1).
    pub const MANUAL_LIFE_SAFETY: Self = Self(1);

    /// 2, automatic life safety (Table 19-1).
    pub const AUTOMATIC_LIFE_SAFETY: Self = Self(2);

    /// 5, critical equipment control (Table 19-1).
    pub const CRITICAL_EQUIPMENT: Self = Self(5);

    /// 6, which a binary object's own minimum on/off timing logic writes
    /// (Clause 19.2.3): nothing else should use it.
    pub const MINIMUM_ON_OFF: Self = Self(6);

    /// 8, manual operator (Table 19-1).
    pub const MANUAL_OPERATOR: Self = Self(8);

    /// 16, the lowest: the one a commandable property takes a write at
    /// when the write names none (Clause 19.2.1).
    pub const LOWEST: Self = Self(16);

    /// Priority `level`.
    ///
    /// # Errors
    ///
    /// [`OpsError::Parse`] (`what` is `"priority"`) for a level outside 1
    /// to 16.
    pub fn new(level: u8) -> Result<Self, OpsError> {
        if (1..=16).contains(&level) {
            Ok(Self(level))
        } else {
            Err(OpsError::parse(
                "priority",
                &level.to_string(),
                format!("priority must be 1-16, got {level}"),
            ))
        }
    }

    /// The level, 1 to 16.
    pub const fn get(self) -> u8 {
        self.0
    }

    /// Its slot's position in a Priority_Array, from 0.
    pub const fn index(self) -> usize {
        self.0 as usize - 1
    }

    /// The priority of the slot at `index` in a Priority_Array, from 0;
    /// `None` past the sixteenth.
    pub fn at_index(index: usize) -> Option<Self> {
        u8::try_from(index + 1)
            .ok()
            .and_then(|level| Self::new(level).ok())
    }

    /// Every priority, 1 first.
    pub fn all() -> impl Iterator<Item = Self> {
        (1..=16).map(Self)
    }

    /// What Table 19-1 assigns this priority to, in a few words: five are
    /// assigned, and the rest are `site defined`.
    pub fn purpose(self) -> &'static str {
        match self.0 {
            1 => "life safety, manual",
            2 => "life safety, automatic",
            5 => "critical equipment",
            6 => "minimum on/off time",
            8 => "operator, manual",
            _ => "site defined",
        }
    }
}

impl TryFrom<u8> for Priority {
    type Error = OpsError;

    fn try_from(level: u8) -> Result<Self, OpsError> {
        Self::new(level)
    }
}

impl From<Priority> for u8 {
    fn from(priority: Priority) -> Self {
        priority.0
    }
}

impl FromStr for Priority {
    type Err = OpsError;

    /// A level as decimal digits, `8`.
    fn from_str(text: &str) -> Result<Self, OpsError> {
        match text.trim().parse::<u8>() {
            Ok(level) => Self::new(level),
            Err(_) => Err(OpsError::parse(
                "priority",
                text,
                format!("invalid priority: '{text}'"),
            )),
        }
    }
}

impl fmt::Display for Priority {
    /// The level, `8`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// The one priority a write names: the one its value text gives after an
/// `@` (`named`), the one the call gives (`given`), or both when they are
/// the same.
///
/// # Errors
///
/// [`OpsError::Parse`] (`what` is `"priority"`) when they differ: which was
/// meant is unknown, so neither is picked.
pub fn one_priority(
    named: Option<Priority>,
    given: Option<Priority>,
) -> Result<Option<Priority>, OpsError> {
    match (named, given) {
        (Some(named), Some(given)) if named != given => Err(OpsError::parse(
            "priority",
            &format!("@{named}"),
            format!(
                "the value names priority {named} and the priority argument {given}: give one, or the same in both"
            ),
        )),
        (named, given) => Ok(named.or(given)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_outside_one_to_sixteen_are_refused() {
        for level in [0, 17, 255] {
            let refused = Priority::new(level).unwrap_err();
            assert_eq!(
                refused.to_string(),
                format!("priority must be 1-16, got {level}")
            );
        }
        let all: Vec<u8> = Priority::all().map(Priority::get).collect();
        assert_eq!(all, (1..=16).collect::<Vec<_>>());
        assert_eq!(Priority::all().count(), Priority::COUNT);
        assert!(" 7 ".parse::<Priority>().is_ok());
        assert_eq!(
            "x".parse::<Priority>().unwrap_err().to_string(),
            "invalid priority: 'x'"
        );
        assert_eq!(Priority::at_index(15), Some(Priority::LOWEST));
        assert_eq!(Priority::at_index(16), None);
    }

    /// The labels the TUI's priority pane shows, from Table 19-1.
    #[test]
    fn five_have_a_standard_purpose() {
        let named: Vec<(u8, &str)> = Priority::all()
            .filter(|p| p.purpose() != "site defined")
            .map(|p| (p.get(), p.purpose()))
            .collect();
        assert_eq!(named.len(), 5, "{named:?}");
        assert_eq!(Priority::CRITICAL_EQUIPMENT.purpose(), "critical equipment");
    }

    /// A conflict between the text's priority and the call's is refused;
    /// the same in both, or either alone, is that one.
    #[test]
    fn a_conflict_is_refused_never_picked() {
        let (p8, p10) = (Priority::MANUAL_OPERATOR, Priority::new(10).unwrap());
        assert_eq!(one_priority(Some(p8), None).unwrap(), Some(p8));
        assert_eq!(one_priority(None, Some(p10)).unwrap(), Some(p10));
        assert_eq!(one_priority(Some(p8), Some(p8)).unwrap(), Some(p8));
        assert_eq!(one_priority(None, None).unwrap(), None);
        let refused = one_priority(Some(p8), Some(p10)).unwrap_err();
        assert!(
            matches!(
                &refused,
                OpsError::Parse {
                    what: "priority",
                    ..
                }
            ),
            "{refused:?}"
        );
        assert!(refused.to_string().contains("priority 8"), "{refused}");
    }
}
