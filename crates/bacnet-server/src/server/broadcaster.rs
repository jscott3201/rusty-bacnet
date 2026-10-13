//! Local sends share the server's admission seal and joined request lifetime.
use super::*;
use std::sync::{Mutex as SyncMutex, Weak};

pub(super) struct BroadcasterState<T: TransportPort> {
    network: SyncMutex<Option<Arc<NetworkLayer<T>>>>,
    requests: Weak<request_tasks::RequestTasks>,
    config: Arc<ServerConfig>,
    /// Weak, so a server dropped without `stop()` is left holding the only
    /// handle it hands off the runtime (#1409).
    db: Weak<RwLock<ObjectDatabase>>,
    /// The server's DeviceCommunicationControl state, read before each send.
    comm_state: Arc<CommState>,
    capacity: Arc<Semaphore>,
}

pub(super) fn stopped() -> Error {
    Error::Encoding("server is stopping or stopped".into())
}

impl<T: TransportPort> BroadcasterState<T> {
    pub(super) fn new(
        network: &Arc<NetworkLayer<T>>,
        requests: &Arc<request_tasks::RequestTasks>,
        config: &Arc<ServerConfig>,
        db: &Arc<RwLock<ObjectDatabase>>,
        comm_state: &Arc<CommState>,
    ) -> Arc<Self> {
        Arc::new(Self {
            network: SyncMutex::new(Some(Arc::clone(network))),
            requests: Arc::downgrade(requests),
            config: Arc::clone(config),
            db: Arc::downgrade(db),
            comm_state: Arc::clone(comm_state),
            capacity: Arc::new(Semaphore::new(32)),
        })
    }

    pub(super) fn seal(&self) {
        // Serializes with clone + registration, including callers that already
        // upgraded a weak handle on another thread. Never held across await.
        self.network.lock().unwrap().take();
        self.capacity.close();
    }

    pub(super) fn is_open(&self) -> bool {
        self.network.lock().unwrap().is_some()
    }
}

impl<T: TransportPort + 'static> BroadcasterState<T> {
    fn admit(
        &self,
        limiter: Option<Arc<DiscoveryLimiter>>,
    ) -> Result<oneshot::Receiver<Result<(), Error>>, Error> {
        let slot = self.network.lock().unwrap();
        let network = Arc::clone(slot.as_ref().ok_or_else(stopped)?);
        let permit = Arc::clone(&self.capacity)
            .try_acquire_owned()
            .map_err(|_| Error::Encoding("server I-Am broadcast capacity exhausted".into()))?;
        let requests = self.requests.upgrade().ok_or_else(stopped)?;
        let config = Arc::clone(&self.config);
        let db = self.db.upgrade().ok_or_else(stopped)?;
        let comm_state = Arc::clone(&self.comm_state);
        let (tx, rx) = oneshot::channel();
        requests.spawn(async move {
            let _permit = permit;
            let result = discovery::broadcast_i_am_from(
                &config,
                &db,
                &network,
                &comm_state,
                limiter.as_ref(),
            )
            .await;
            let _ = tx.send(result);
        });
        Ok(rx)
    }

    pub(super) async fn send(&self, limiter: Option<Arc<DiscoveryLimiter>>) -> Result<(), Error> {
        self.admit(limiter)?.await.map_err(|_| stopped())?
    }
}

impl<T: TransportPort + 'static> IAmBroadcaster<T> {
    /// Send through the running server; overload or shutdown returns an error,
    /// and so does DeviceCommunicationControl restricting initiation (see
    /// [`BACnetServer::broadcast_i_am`]). Cancelling this waiter does not
    /// cancel an already admitted send.
    pub async fn broadcast_i_am(&self) -> Result<(), Error> {
        let completion = self.state.upgrade().ok_or_else(stopped)?.admit(None)?;
        completion.await.map_err(|_| stopped())?
    }
}

impl<T: TransportPort + 'static> BACnetServer<T> {
    #[cfg(test)]
    pub(super) fn test_network(&self) -> &Arc<NetworkLayer<T>> {
        self.network
            .as_ref()
            .expect("test requires retained network")
    }

    pub(super) fn active_network(&self) -> Result<&Arc<NetworkLayer<T>>, Error> {
        if !self.broadcaster.is_open() {
            return Err(stopped());
        }
        self.network.as_ref().ok_or_else(stopped)
    }
}
