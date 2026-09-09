use super::*;

// Borrow the handle in its owning slot through the join. Cancelling the caller
// leaves the (possibly already aborted) handle available for the next cleanup.
pub(super) async fn cancel(slot: &mut Option<JoinHandle<()>>) {
    if let Some(task) = slot.as_mut() {
        task.abort();
        let _ = task.await;
    }
    *slot = None;
}

pub(super) async fn replace(
    timer: &Arc<Mutex<Option<JoinHandle<()>>>>,
    comm_state: &Arc<AtomicU8>,
    service_data: &[u8],
    password: &Option<String>,
    policy: DccPolicy,
) -> Result<dcc_outcomes::DccMetadata, handlers::device_mgmt::DccFailure> {
    // Decode and validate once, retaining only non-secret proposed state and
    // metadata. Never change live state before a cancellable await.
    let (mode, duration, proposed) =
        handlers::device_mgmt::validate_dcc(service_data, password, policy)?;
    let mut slot = timer.lock().await;
    cancel(&mut slot).await;
    // Replacement, expiry, and shutdown share this linearization boundary.
    // No suspension between the live commit and installing the new owner.
    comm_state.store(proposed, Ordering::Release);
    if let Some(minutes) = duration {
        let owner = Arc::downgrade(timer);
        let comm = Arc::clone(comm_state);
        *slot = Some(tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(minutes as u64 * 60)).await;
            if let Some(owner) = owner.upgrade() {
                // An old task waiting here can be aborted and joined while a
                // replacement holds the slot; it never joins or removes itself.
                let _slot = owner.lock().await;
                comm.store(0, Ordering::Release);
                debug!(
                    "DCC timer expired after {} min, state reverted to ENABLE",
                    minutes
                );
            }
        }));
    }
    Ok(dcc_outcomes::DccMetadata {
        mode: Some(mode.to_raw()),
        duration,
    })
}
