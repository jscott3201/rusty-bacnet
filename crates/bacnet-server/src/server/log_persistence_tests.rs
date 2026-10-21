//! A durable Trend Log under the running server (#1541): the once-a-second
//! operation pass commits it when its interval runs out, and `stop()`
//! commits it once more and waits for that commit.

use super::*;
use crate::server::cov_notifications_tests::recording_transport;
use crate::server::test_transport::TestTransport;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::log_persistence::{LogCommitCadence, LogPersistence, LogSnapshot};
use bacnet_objects::traits::BACnetObject;
use bacnet_objects::trend::TrendLogObject;
use bacnet_types::constructed::{BACnetLogRecord, LogDatum};
use bacnet_types::primitives::{Date, Time};
use std::sync::{Condvar, Mutex as StdMutex};

/// Storage that keeps the Total_Record_Count of each commit.
#[derive(Default)]
struct Commits {
    totals: StdMutex<Vec<u32>>,
    landed: Condvar,
}

impl Commits {
    fn totals(&self) -> Vec<u32> {
        self.totals.lock().unwrap().clone()
    }

    /// Block until `count` commits have landed. The writer thread runs
    /// them, so this waits on it off the runtime; the bound only turns a
    /// missing commit into a failure.
    fn wait_for(&self, count: usize) {
        let totals = self.totals.lock().unwrap();
        let (totals, timeout) = self
            .landed
            .wait_timeout_while(totals, Duration::from_secs(10), |totals| {
                totals.len() < count
            })
            .unwrap();
        assert!(
            !timeout.timed_out(),
            "{count} commits expected, got {totals:?}"
        );
    }
}

impl LogPersistence<BACnetLogRecord> for Commits {
    fn load(
        &self,
        _expected: ObjectIdentifier,
    ) -> Result<Option<LogSnapshot<BACnetLogRecord>>, Error> {
        Ok(None)
    }

    fn commit(&self, snapshot: &LogSnapshot<BACnetLogRecord>) -> Result<(), Error> {
        self.totals
            .lock()
            .unwrap()
            .push(snapshot.total_record_count);
        self.landed.notify_all();
        Ok(())
    }
}

fn record(hour: u8) -> BACnetLogRecord {
    BACnetLogRecord {
        date: Date {
            year: 126,
            month: 10,
            day: 20,
            day_of_week: 2,
        },
        time: Time {
            hour,
            minute: 0,
            second: 0,
            hundredths: 0,
        },
        log_datum: LogDatum::RealValue(f32::from(hour)),
        status_flags: None,
    }
}

async fn settle() {
    for _ in 0..16 {
        tokio::task::yield_now().await;
    }
}

#[tokio::test(start_paused = true)]
async fn the_server_commits_a_durable_log_on_its_interval_and_at_stop() {
    let storage = Arc::new(Commits::default());
    let mut log = TrendLogObject::with_persistence(1, "TL-1", 50, storage.clone()).unwrap();
    log.set_commit_cadence(LogCommitCadence {
        records: 1000,
        interval: Duration::from_secs(60),
    });
    log.add_record(record(1)).unwrap();
    let oid = log.object_identifier();
    let identity = bacnet_objects::DeviceIdentity::new(100, 0).unwrap();
    let device = DeviceObject::new(DeviceConfig {
        identity: identity.clone().with_name("Durable-log-device"),
        ..DeviceConfig::default()
    })
    .unwrap();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(device)).unwrap();
    db.add(Box::new(log)).unwrap();
    let (transport, _sent) = recording_transport();
    let config = ServerConfig {
        device: identity,
        enable_event_enrollment: false,
        ..ServerConfig::default()
    };
    let mut server: BACnetServer<TestTransport> =
        BACnetServer::start(config, db, transport).await.unwrap();
    settle().await;

    // The pass at start began the interval; half of it commits nothing.
    tokio::time::advance(Duration::from_secs(30)).await;
    settle().await;
    assert!(storage.totals().is_empty());
    tokio::time::advance(Duration::from_secs(31)).await;
    settle().await;
    let waiting = storage.clone();
    tokio::task::spawn_blocking(move || waiting.wait_for(1))
        .await
        .unwrap();
    assert_eq!(storage.totals(), vec![1]);

    // One more record, well short of the count and the interval: stop
    // commits it, and has waited for that commit when it returns.
    server
        .database()
        .write()
        .await
        .get_mut(&oid)
        .unwrap()
        .add_trend_record(record(2))
        .unwrap();
    server.stop().await.unwrap();
    assert_eq!(storage.totals(), vec![1, 2]);
}
