//! Observation of the session's own link, without another socket or reader.

use super::EndpointSession;
use bacnet_transport::port::TransportPort;
use bacnet_transport::tap::FrameTapReceiver;

impl<T: TransportPort + 'static> EndpointSession<T> {
    /// Subscribe to raw frames on a B/IP-builder session's transport.
    ///
    /// Returns `None` outside the running state or when the builder did not
    /// supply a tap. Each receiver has its own bounded queue; full queues
    /// discard new frames and count them in [`FrameTapReceiver::dropped`].
    /// The receiver does not keep the transport alive. As with the transport's
    /// tap, stopping alone does not end it: drop it when observation is over.
    pub fn frames(&self, capacity: usize) -> Option<FrameTapReceiver> {
        self.is_running()
            .then(|| {
                self.frame_tap
                    .as_ref()
                    .map(|tap| tap.subscribe_with_capacity(capacity))
            })
            .flatten()
    }
}
