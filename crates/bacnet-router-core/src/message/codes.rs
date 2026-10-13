//! The wire codes of network-layer messages: [`MessageType`] and
//! [`RejectReason`].

/// An open set of wire codes with named constants, the shape the codecs
/// give `BvlcFunction` and `FrameType`: any raw value converts, and the
/// names cover the values the standard defines. The codecs keep their own
/// copies, since no crate here depends on another for it; CI checks that
/// the copies match (`scripts/ci/check-named-codes.py`).
macro_rules! named_codes {
    (
        $(#[$meta:meta])*
        pub struct $Name:ident($inner:ty);
        $(
            $(#[$vmeta:meta])*
            const $VARIANT:ident = $val:expr;
        )*
    ) => {
        $(#[$meta])*
        #[derive(Clone, Copy, PartialEq, Eq, Hash)]
        pub struct $Name($inner);

        impl $Name {
            $(
                $(#[$vmeta])*
                #[doc = ""]
                #[doc = concat!("Wire value `", stringify!($val), "`.")]
                pub const $VARIANT: Self = Self($val);
            )*

            /// Every named constant, with its name, in the order declared.
            pub const ALL_NAMED: &[(&str, Self)] = &[
                $( (stringify!($VARIANT), Self($val)), )*
            ];

            /// Wrap a raw wire value, named or not.
            #[inline]
            #[must_use]
            pub const fn from_raw(value: $inner) -> Self {
                Self(value)
            }

            /// The raw wire value.
            #[inline]
            #[must_use]
            pub const fn to_raw(self) -> $inner {
                self.0
            }

            /// The constant's name, or `None` for a value the standard
            /// doesn't define.
            #[must_use]
            pub const fn name(self) -> Option<&'static str> {
                match self.0 {
                    $( $val => Some(stringify!($VARIANT)), )*
                    _ => None,
                }
            }

            /// Whether the standard defines this value.
            #[must_use]
            pub const fn is_defined(self) -> bool {
                self.name().is_some()
            }
        }

        /// `Name::CONSTANT` for a named value, `Name(raw)` otherwise.
        impl core::fmt::Debug for $Name {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                match self.name() {
                    Some(name) => write!(f, "{}::{}", stringify!($Name), name),
                    None => write!(f, "{}({})", stringify!($Name), self.0),
                }
            }
        }

        /// The constant's name, or the raw value in decimal.
        impl core::fmt::Display for $Name {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                match self.name() {
                    Some(name) => f.write_str(name),
                    None => write!(f, "{}", self.0),
                }
            }
        }
    };
}

named_codes! {
    /// A network-layer message type: the octet after an NPDU's addresses
    /// that says which message it carries (Clause 6.2.4).
    ///
    /// The set is open, so [`Npdu::parse`](crate::npdu::Npdu::parse) hands
    /// on every octet as it arrived, and a router matches the constants with
    /// a `_` arm for the rest. The constants name the types a router reads
    /// and sends, grouped by what a router does with them and by name within
    /// a group, and [`ALL_NAMED`](Self::ALL_NAMED) lists them that way. Of
    /// the other octets, 0x80 and up are vendor proprietary
    /// ([`is_proprietary`](Self::is_proprietary)), 0x0A to 0x11 are the
    /// network-security types the standard no longer defines
    /// ([`is_removed_security`](Self::is_removed_security)), and the rest
    /// are reserved. Two message types are equal when their octets are.
    ///
    /// ```
    /// use bacnet_router_core::message::MessageType;
    ///
    /// assert_eq!(MessageType::from_raw(0x13), MessageType::NETWORK_NUMBER_IS);
    /// assert_eq!(MessageType::NETWORK_NUMBER_IS.name(), Some("NETWORK_NUMBER_IS"));
    ///
    /// // A proprietary type has no name, but keeps its octet.
    /// let proprietary = MessageType::from_raw(0x80);
    /// assert!(proprietary.is_proprietary() && !proprietary.is_defined());
    /// assert_eq!(format!("{proprietary:?}"), "MessageType(128)");
    /// ```
    pub struct MessageType(u8);

    // Routing discovery: finding the router that reaches a network, and
    // the refusal a router sends when it can't route a message.
    /// A router's list of what it reaches; this router learns from it.
    const I_AM_ROUTER_TO_NETWORK = 0x01;
    /// A route on offer; kept only where the table has none.
    const I_COULD_BE_ROUTER_TO_NETWORK = 0x02;
    /// A refusal with its reason, carried back to whoever sent the message.
    const REJECT_MESSAGE_TO_NETWORK = 0x03;
    /// A route query, answered here or searched for beyond.
    const WHO_IS_ROUTER_TO_NETWORK = 0x00;

    // Busy and available: flow control between routers.
    /// Lifts a hold on traffic.
    const ROUTER_AVAILABLE_TO_NETWORK = 0x05;
    /// Puts a hold on traffic, which lapses on its own.
    const ROUTER_BUSY_TO_NETWORK = 0x04;

    // The routing table: updates, queries and their answers.
    /// A table update, or a query with a count of 0.
    const INITIALIZE_ROUTING_TABLE = 0x06;
    /// What comes back for one.
    const INITIALIZE_ROUTING_TABLE_ACK = 0x07;

    // Connections over point-to-point links, which this stack doesn't open.
    /// Counted and otherwise ignored here.
    const DISCONNECT_CONNECTION_TO_NETWORK = 0x09;
    /// Logged and otherwise ignored here.
    const ESTABLISH_CONNECTION_TO_NETWORK = 0x08;

    // The network number of the local network.
    /// The number, and whether it was configured.
    const NETWORK_NUMBER_IS = 0x13;
    /// The question that number answers.
    const WHAT_IS_NETWORK_NUMBER = 0x12;
}

impl MessageType {
    /// Whether this is a vendor-proprietary type, 0x80 and up, which an NPDU
    /// follows with a two-octet vendor ID.
    #[must_use]
    pub const fn is_proprietary(self) -> bool {
        self.to_raw() >= 0x80
    }

    /// Whether this is one of the network-security types, 0x0A to 0x11,
    /// that the standard no longer defines. A router doesn't reject them.
    #[must_use]
    pub const fn is_removed_security(self) -> bool {
        matches!(self.to_raw(), 0x0A..=0x11)
    }
}

named_codes! {
    /// The reason a Reject-Message-To-Network gives (Clause 6.4.4).
    ///
    /// The set is open, so a reject read off the wire keeps whatever octet
    /// it carried, and a `match` on the constants needs a `_` arm. The
    /// constants are the reasons the standard defines: 5 has been
    /// withdrawn, and 7 and up are unassigned. Two reasons are equal when
    /// their octets are.
    ///
    /// ```
    /// use bacnet_router_core::message::RejectReason;
    ///
    /// assert_eq!(RejectReason::from_raw(4), RejectReason::MESSAGE_TOO_LONG);
    /// assert_eq!(format!("{}", RejectReason::ROUTER_BUSY), "ROUTER_BUSY");
    /// assert_eq!(format!("{:?}", RejectReason::from_raw(5)), "RejectReason(5)");
    /// ```
    pub struct RejectReason(u8);

    /// Some other error.
    const OTHER = 0;
    /// No route to the network, after looking for one.
    const NOT_DIRECTLY_CONNECTED = 1;
    /// The router is busy for that network for now.
    const ROUTER_BUSY = 2;
    /// A message type the router doesn't know.
    const UNKNOWN_MESSAGE_TYPE = 3;
    /// Too long to route to that network.
    const MESSAGE_TOO_LONG = 4;
    /// A DLEN or SLEN no data link uses.
    const ADDRESSING_ERROR = 6;
}
