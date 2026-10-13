//! [`Arrived`]: what the link glue hands the router for one received NPDU.

use crate::router::Frame;
use crate::{Mac, PortId};

/// An NPDU a port's link core delivered, with its link source held by
/// value, so the [`Frame`] the router reads can borrow it.
///
/// The B/IP and MS/TP ports build one from what their link core delivers;
/// pass its [`frame`](Self::frame) to
/// [`Router::receive`](crate::Router::receive).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Arrived<'d> {
    /// The port it came in on.
    pub port: PortId,
    /// The link-layer source: a B/IP address's six octets, which for a
    /// Forwarded-NPDU are its origin's, or an MS/TP station's one.
    pub source: Mac,
    /// Whether it reached more than this node: a link broadcast.
    pub link_group: bool,
    /// The NPDU.
    pub npdu: &'d [u8],
}

impl Arrived<'_> {
    /// The frame for [`Router::receive`](crate::Router::receive).
    #[must_use]
    pub fn frame(&self) -> Frame<'_> {
        Frame {
            port: self.port,
            source: self.source.as_slice(),
            link_group: self.link_group,
            npdu: self.npdu,
        }
    }
}
