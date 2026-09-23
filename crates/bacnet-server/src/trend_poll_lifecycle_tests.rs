use super::*;
use bacnet_types::error::Error;
use std::borrow::Cow;
use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Condvar;

#[tokio::test(start_paused = true)]
async fn loop_observes_hundredths_reconciles_changes_and_aborts_sleep() {
    let (db, oid) = database(Arc::new(MutableClock(Mutex::new(Some(frame())))));
    db.write()
        .await
        .get_mut(&oid)
        .unwrap()
        .write_property(
            PropertyIdentifier::LOG_INTERVAL,
            None,
            PropertyValue::Unsigned(1),
            None,
        )
        .unwrap();
    let task = tokio::spawn(crate::trend_log::run(db.clone()));
    tokio::task::yield_now().await;
    assert_eq!(
        snapshot(&*db.read().await, oid)[2],
        PropertyValue::Unsigned(1)
    );
    tokio::time::advance(Duration::from_millis(9)).await;
    tokio::task::yield_now().await;
    assert_eq!(
        snapshot(&*db.read().await, oid)[2],
        PropertyValue::Unsigned(1)
    );
    tokio::time::advance(Duration::from_millis(1)).await;
    tokio::task::yield_now().await;
    assert_eq!(
        snapshot(&*db.read().await, oid)[2],
        PropertyValue::Unsigned(2)
    );
    db.write()
        .await
        .get_mut(&oid)
        .unwrap()
        .write_property(
            PropertyIdentifier::LOG_INTERVAL,
            None,
            PropertyValue::Unsigned(u32::MAX.into()),
            None,
        )
        .unwrap();
    tokio::time::advance(Duration::from_millis(100)).await;
    tokio::task::yield_now().await;
    assert_eq!(
        snapshot(&*db.read().await, oid)[2],
        PropertyValue::Unsigned(3)
    );
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    tokio::time::advance(Duration::from_secs(10)).await;
    assert_eq!(
        snapshot(&*db.read().await, oid)[2],
        PropertyValue::Unsigned(3)
    );
}

struct GateTrend {
    object: Box<dyn BACnetObject>,
    first: AtomicBool,
    selected: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    release: Arc<(Mutex<bool>, Condvar)>,
    appended: Arc<AtomicUsize>,
}
impl BACnetObject for GateTrend {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.object.object_identifier()
    }
    fn object_name(&self) -> &str {
        self.object.object_name()
    }
    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        self.object.property_list()
    }
    fn read_property(
        &self,
        property: PropertyIdentifier,
        index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        if property == PropertyIdentifier::LOG_INTERVAL && self.first.swap(false, Ordering::SeqCst)
        {
            self.selected
                .lock()
                .unwrap()
                .take()
                .unwrap()
                .send(())
                .unwrap();
            let (lock, cv) = &*self.release;
            let mut released = lock.lock().unwrap();
            while !*released {
                released = cv.wait(released).unwrap();
            }
        }
        self.object.read_property(property, index)
    }
    fn write_property(
        &mut self,
        p: PropertyIdentifier,
        i: Option<u32>,
        v: PropertyValue,
        priority: Option<u8>,
    ) -> Result<(), Error> {
        self.object.write_property(p, i, v, priority)
    }
    fn bind_clock_internal(&mut self, clock: Option<Arc<dyn ClockReader>>) {
        self.object.bind_clock_internal(clock);
    }
    fn add_trend_record(
        &mut self,
        record: bacnet_types::constructed::BACnetLogRecord,
    ) -> Result<(), Error> {
        self.object.add_trend_record(record)?;
        self.appended.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn queued_replacement_cannot_interleave_selection_and_append() {
    let (db, oid) = database(Arc::new(MutableClock(Mutex::new(Some(frame())))));
    let (selected_tx, selected_rx) = tokio::sync::oneshot::channel();
    let release = Arc::new((Mutex::new(false), Condvar::new()));
    let appended = Arc::new(AtomicUsize::new(0));
    {
        let mut guard = db.write().await;
        let object = guard.remove(&oid).unwrap();
        guard
            .add(Box::new(GateTrend {
                object,
                first: AtomicBool::new(true),
                selected: Mutex::new(Some(selected_tx)),
                release: release.clone(),
                appended: appended.clone(),
            }))
            .unwrap();
    }
    let polling_db = db.clone();
    let poll = tokio::spawn(async move { poll_trend_logs(&polling_db).await });
    selected_rx.await.unwrap();
    let mut replacement = Box::pin(db.write());
    std::future::poll_fn(|cx| {
        assert!(replacement.as_mut().poll(cx).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
    // The writer is queued before selection may finish. With separate selection
    // and append guards it would acquire the slot before the append guard.
    *release.0.lock().unwrap() = true;
    release.1.notify_one();
    let mut guard = replacement.await;
    assert_eq!(appended.load(Ordering::SeqCst), 1);
    guard
        .add(Box::new(TrendLogObject::new(1, "Replacement", 8).unwrap()))
        .unwrap();
    drop(guard);
    poll.await.unwrap();
    assert_eq!(
        snapshot(&*db.read().await, oid)[2],
        PropertyValue::Unsigned(0)
    );
}
