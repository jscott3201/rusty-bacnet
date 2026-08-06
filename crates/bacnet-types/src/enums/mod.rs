//! BACnet enumeration types per ASHRAE 135-2020.
//!
//! Uses newtype wrappers (e.g. `ObjectType(u32)`) with associated constants
//! rather than Rust enums so that vendor-proprietary values pass through
//! without panicking. Every type provides `from_raw` / `to_raw` for
//! wire-level conversion and a human-readable `Display` impl.

#[cfg(not(feature = "std"))]
use alloc::format;

// ---------------------------------------------------------------------------
// Macro to reduce boilerplate for newtype enum wrappers
// ---------------------------------------------------------------------------

/// Generates a newtype wrapper struct with associated constants, `from_raw`,
/// `to_raw`, `Display`, and optional `Debug` that shows the symbolic name.
macro_rules! bacnet_enum {
    (
        $(#[$meta:meta])*
        $vis:vis struct $Name:ident($inner:ty);
        $(
            $(#[$vmeta:meta])*
            const $VARIANT:ident = $val:expr;
        )*
    ) => {
        $(#[$meta])*
        #[derive(Clone, Copy, PartialEq, Eq, Hash)]
        $vis struct $Name($inner);

        impl $Name {
            $(
                $(#[$vmeta])*
                pub const $VARIANT: Self = Self($val);
            )*

            /// Create from a raw wire value.
            #[inline]
            pub const fn from_raw(value: $inner) -> Self {
                Self(value)
            }

            /// Return the raw wire value.
            #[inline]
            pub const fn to_raw(self) -> $inner {
                self.0
            }

            /// All named constants as (name, value) pairs.
            pub const ALL_NAMED: &[(&str, Self)] = &[
                $( (stringify!($VARIANT), Self($val)), )*
            ];
        }

        impl core::fmt::Debug for $Name {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                match self.0 {
                    $( $val => f.write_str(concat!(stringify!($Name), "::", stringify!($VARIANT))), )*
                    other => write!(f, "{}({})", stringify!($Name), other),
                }
            }
        }

        impl core::fmt::Display for $Name {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                match self.0 {
                    $( $val => f.write_str(stringify!($VARIANT)), )*
                    other => write!(f, "{}", other),
                }
            }
        }

        impl core::str::FromStr for $Name {
            type Err = $crate::enums::ParseEnumError;

            /// Parse a symbolic constant name, ignoring `_`/`-` separators and
            /// ASCII case, so every common spelling of a name is accepted
            /// (`ANALOG_INPUT`, `analog-input`, `AnalogInput`, `analogInput`, ...).
            ///
            /// Raw numeric values are *not* accepted; use
            /// [`from_raw`](Self::from_raw) for those.
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Self::ALL_NAMED
                    .iter()
                    .find(|(name, _)| $crate::enums::names_eq(name, s))
                    .map(|&(_, value)| value)
                    .ok_or($crate::enums::ParseEnumError {
                        type_name: stringify!($Name),
                    })
            }
        }
    };
}

// ---------------------------------------------------------------------------
// Name parsing
// ---------------------------------------------------------------------------

/// Returned by the `FromStr` impls of the BACnet enum newtypes when a string
/// does not name any known constant of that type.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ParseEnumError {
    type_name: &'static str,
}

impl ParseEnumError {
    /// The name of the enum type that failed to parse, e.g. `"ObjectType"`.
    #[inline]
    pub const fn type_name(&self) -> &'static str {
        self.type_name
    }
}

impl core::fmt::Display for ParseEnumError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "not a known {} name", self.type_name)
    }
}

#[cfg(feature = "std")]
impl std::error::Error for ParseEnumError {}

/// Compare two enum names for equality, ignoring `_`/`-` separators and ASCII
/// case. Dropping separators and folding case collapses every supported spelling
/// (lowercase, UPPERCASE, PascalCase, camelCase, snake_case,
/// SCREAMING_SNAKE_CASE, kebab-case, SCREAMING-KEBAB-CASE) onto the same form,
/// and comparing lazily keeps this allocation-free.
fn names_eq(a: &str, b: &str) -> bool {
    fn significant(s: &str) -> impl Iterator<Item = char> + '_ {
        s.chars()
            .filter(|c| *c != '_' && *c != '-')
            .map(|c| c.to_ascii_lowercase())
    }

    significant(a).eq(significant(b))
}

mod object_type;
pub use object_type::*;
mod property_id;
pub use property_id::*;
mod protocol;
pub use protocol::*;
mod network;
pub use network::*;
mod bvll;
pub use bvll::*;
mod object_level;
pub use object_level::*;
mod network_port;
pub use network_port::*;
mod life_safety;
pub use life_safety::*;
mod scheduling;
pub use scheduling::*;
mod access;
pub use access::*;
mod lighting;
pub use lighting::*;
mod lift;
pub use lift::*;
mod misc;
pub use misc::*;
mod audit;
pub use audit::*;
mod units;
pub use units::*;
mod resolve;
pub use resolve::*;

#[cfg(test)]
mod tests;
