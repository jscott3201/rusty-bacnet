//! [`BoxedSerial`], a serial port with its type erased.
//!
//! [`SerialPort`]'s methods return `impl Future`, so the trait has no
//! `dyn` form. [`AnyTransport`](crate::any::AnyTransport) needs one port
//! type for every serial backend, so its MS/TP variant runs over a
//! [`BoxedSerial`]: a box around an object-safe mirror of the trait whose
//! methods return boxed futures. Each read, write or drain costs one
//! dynamic call and one future allocation; the port's own I/O dominates
//! both on a real line.

use std::any::Any;
use std::future::Future;
use std::pin::Pin;

use bacnet_types::error::Error;

use super::{Echo, SerialPort};

/// A boxed future a [`DynSerial`] method returns.
type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, Error>> + Send + 'a>>;

/// [`SerialPort`] with boxed futures, so it can sit behind `dyn`.
trait DynSerial: Send + Sync + 'static {
    fn write_boxed<'a>(&'a self, data: &'a [u8]) -> BoxFuture<'a, ()>;
    fn drain_boxed(&self) -> BoxFuture<'_, ()>;
    fn read_boxed<'a>(&'a self, buf: &'a mut [u8]) -> BoxFuture<'a, usize>;
    fn echo_dyn(&self) -> Echo;
}

impl<S: SerialPort> DynSerial for S {
    fn write_boxed<'a>(&'a self, data: &'a [u8]) -> BoxFuture<'a, ()> {
        Box::pin(self.write(data))
    }

    fn drain_boxed(&self) -> BoxFuture<'_, ()> {
        Box::pin(self.drain())
    }

    fn read_boxed<'a>(&'a self, buf: &'a mut [u8]) -> BoxFuture<'a, usize> {
        Box::pin(self.read(buf))
    }

    fn echo_dyn(&self) -> Echo {
        self.echo()
    }
}

/// Any [`SerialPort`], boxed: the port type of
/// [`AnyTransport`](crate::any::AnyTransport)'s MS/TP variant.
///
/// It forwards every call, including [`SerialPort::drain`] and
/// [`SerialPort::echo`], to the port it holds. Converting an
/// [`MstpTransport`](super::MstpTransport) into an `AnyTransport` boxes its
/// port for you; build one directly only to name the variant's type.
///
/// ```
/// use bacnet_transport::any::AnyTransport;
/// use bacnet_transport::mstp::{BoxedSerial, LoopbackSerial, MstpConfig, MstpTransport};
///
/// let (serial, _peer) = LoopbackSerial::pair();
/// let any: AnyTransport = MstpTransport::new(serial, MstpConfig::default()).into();
/// assert!(matches!(any, AnyTransport::Mstp(_)));
///
/// // The same, boxing the port by hand.
/// let (serial, _peer) = LoopbackSerial::pair();
/// let mstp: MstpTransport<BoxedSerial> =
///     MstpTransport::new(BoxedSerial::new(serial), MstpConfig::default());
/// let any = AnyTransport::Mstp(mstp);
/// # drop(any);
/// ```
pub struct BoxedSerial(Box<dyn DynSerial>);

impl BoxedSerial {
    /// Box `port`. A port that is already a `BoxedSerial` is returned as
    /// it is, not boxed a second time.
    pub fn new<S: SerialPort>(port: S) -> Self {
        // Nothing can be moved out through `&mut dyn Any`, so the port sits
        // in an `Option` the downcast can take it from.
        let mut slot = Some(port);
        if let Some(boxed) = (&mut slot as &mut dyn Any).downcast_mut::<Option<Self>>()
            && let Some(boxed) = boxed.take()
        {
            return boxed;
        }
        match slot {
            Some(port) => Self(Box::new(port)),
            None => unreachable!("the slot is only emptied when S is BoxedSerial"),
        }
    }
}

impl SerialPort for BoxedSerial {
    async fn write(&self, data: &[u8]) -> Result<(), Error> {
        self.0.write_boxed(data).await
    }

    async fn drain(&self) -> Result<(), Error> {
        self.0.drain_boxed().await
    }

    async fn read(&self, buf: &mut [u8]) -> Result<usize, Error> {
        self.0.read_boxed(buf).await
    }

    fn echo(&self) -> Echo {
        self.0.echo_dyn()
    }
}

impl std::fmt::Debug for BoxedSerial {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BoxedSerial").finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mstp::LoopbackSerial;

    /// Every call reaches the boxed port: writes cross to the peer, reads
    /// come back, and the declared echo and drain are the port's own.
    #[tokio::test]
    async fn forwards_every_call_to_the_boxed_port() {
        let grace = std::time::Duration::from_millis(3);
        let (a, b) = LoopbackSerial::pair();
        let a = BoxedSerial::new(a.with_echo(Echo::Full { grace }));
        let b = BoxedSerial::new(b);
        assert_eq!(a.echo(), Echo::Full { grace });
        assert_eq!(b.echo(), Echo::None);

        a.write(&[1, 2, 3]).await.unwrap();
        a.drain().await.unwrap();
        let mut buf = [0; 8];
        // The port hears its own write first, then the peer gets it.
        assert_eq!(a.read(&mut buf).await.unwrap(), 3);
        assert_eq!(b.read(&mut buf).await.unwrap(), 3);
        assert_eq!(&buf[..3], &[1, 2, 3]);
    }

    /// A port without a drain keeps the trait's "unsupported" default
    /// through the box, rather than claiming completion.
    #[tokio::test]
    async fn keeps_a_missing_drain_unsupported() {
        struct NoDrain;
        impl SerialPort for NoDrain {
            async fn write(&self, _data: &[u8]) -> Result<(), Error> {
                Ok(())
            }
            async fn read(&self, _buf: &mut [u8]) -> Result<usize, Error> {
                Ok(0)
            }
        }
        let port = BoxedSerial::new(NoDrain);
        assert!(matches!(
            port.drain().await,
            Err(Error::Transport(e)) if e.kind() == std::io::ErrorKind::Unsupported
        ));
    }

    /// Converting a started transport into an `AnyTransport` carries it
    /// over whole: the router's extended frames, the running task's queues,
    /// and the diagnostics handle and frame tap taken before the move.
    #[tokio::test(start_paused = true)]
    async fn conversion_carries_a_started_transport_over() {
        use crate::any::AnyTransport;
        use crate::mstp::{MstpConfig, MstpTransport};
        use crate::port::TransportPort;

        let (serial, _peer) = LoopbackSerial::pair();
        let mut mstp = MstpTransport::new(serial, MstpConfig::default());
        mstp.prepare_for_routing();
        let diagnostics = mstp.diagnostics();
        let mut frames = mstp.frame_tap().subscribe();
        let _npdus = mstp.start().await.unwrap();
        let mut any = AnyTransport::from(mstp);
        assert_eq!(any.max_npdu(), bacnet_npdu::NpduLimits::both(1497));
        any.send_broadcast(&[1, 0]).await.unwrap();
        assert!(any.send_broadcast(&[0; 1498]).await.is_err());
        assert_eq!(diagnostics.snapshot().outbound_oversize, 1);
        // Alone on the line, the station polls for a master, heard by the
        // tap taken before the move and by one taken from the wrapper.
        let mut later = any.frame_tap().expect("MS/TP has a tap").subscribe();
        let soon = std::time::Duration::from_secs(5);
        assert!(matches!(
            tokio::time::timeout(soon, frames.recv()).await,
            Ok(Some(_))
        ));
        assert!(matches!(
            tokio::time::timeout(soon, later.recv()).await,
            Ok(Some(_))
        ));
        any.stop().await.unwrap();
    }

    /// Boxing a `BoxedSerial` again hands back the same box.
    #[test]
    fn does_not_box_twice() {
        let (a, _b) = LoopbackSerial::pair();
        let once = BoxedSerial::new(a);
        let inner = std::ptr::from_ref::<dyn DynSerial>(&*once.0).cast::<()>();
        let twice = BoxedSerial::new(once);
        let again = std::ptr::from_ref::<dyn DynSerial>(&*twice.0).cast::<()>();
        assert_eq!(inner, again);
    }
}
