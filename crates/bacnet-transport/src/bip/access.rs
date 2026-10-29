//! Reaching the B/IP transport behind a transport that may carry BACnet/IP.

use bacnet_types::error::Error;

use super::BipTransport;

/// A transport that may carry BACnet/IP, and so can lend its [`BipTransport`].
///
/// BBMD management (Annex J) and the B/IP counters only exist on a
/// [`BipTransport`]. This trait lets code that holds some other transport type
/// reach them when the data link underneath is B/IP. `BACnetClient`'s BBMD
/// helpers take any transport with it, so the same calls work on a client over
/// [`BipTransport`] and on one over [`AnyTransport`](crate::any::AnyTransport).
///
/// A wrapper transport, such as a decorator around a `BipTransport`, can
/// implement it by delegating to the transport it wraps.
///
/// ```
/// use std::net::Ipv4Addr;
/// use bacnet_transport::any::AnyTransport;
/// use bacnet_transport::bip::{AsBip, BipTransport};
/// use bacnet_transport::loopback::LoopbackTransport;
/// use bacnet_types::data_link::DataLink;
/// use bacnet_types::error::Error;
///
/// let bip = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
/// let any: AnyTransport = bip.into();
/// assert_eq!(any.as_bip()?.management_counters().read_bdt_responses, 0);
///
/// let (loopback, _peer) = LoopbackTransport::pair(vec![1], vec![2]);
/// let any: AnyTransport = loopback.into();
/// assert!(matches!(
///     any.as_bip(),
///     Err(Error::UnsupportedTransport {
///         required: DataLink::Bip,
///         actual: DataLink::Loopback,
///     })
/// ));
/// # Ok::<(), Error>(())
/// ```
pub trait AsBip {
    /// The B/IP transport underneath this one.
    ///
    /// # Errors
    ///
    /// [`Error::UnsupportedTransport`] naming this transport's data link when
    /// it is not BACnet/IP.
    fn as_bip(&self) -> Result<&BipTransport, Error>;
}

impl AsBip for BipTransport {
    /// Always this transport.
    fn as_bip(&self) -> Result<&BipTransport, Error> {
        Ok(self)
    }
}
