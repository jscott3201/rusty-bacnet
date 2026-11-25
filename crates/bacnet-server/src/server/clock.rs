use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use bacnet_objects::clock::{ClockFrame, ClockReader};
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::durable::{SaveWait, StageStep};
use bacnet_transport::port::TransportPort;
use bacnet_types::error::{DecodeError, DecodingKind, Error, Refusal};
use bacnet_types::primitives::ObjectIdentifier;
use bacnet_types::primitives::{Date, Time};
use bacnet_types::sync::MutexExt;

use tokio::sync::{OwnedRwLockWriteGuard, RwLock};

use super::{
    BACnetServer, BipServerBuilder, ServerBuilder, ServerConfig, StartError, broadcaster,
    time_sync_policy,
};

const HUNDREDTHS_PER_SECOND: i128 = 100;
const HUNDREDTHS_PER_MINUTE: i128 = 60 * HUNDREDTHS_PER_SECOND;
const HUNDREDTHS_PER_DAY: i128 = 24 * 60 * HUNDREDTHS_PER_MINUTE;

/// Validated civil-time settings for the bundled server clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ClockConfig {
    utc_offset_minutes: i16,
    daylight_savings_status: bool,
}

impl ClockConfig {
    /// Configure signed minutes west of UTC and daylight-saving status.
    pub fn new(utc_offset_minutes: i16, daylight_savings_status: bool) -> Result<Self, Error> {
        if !(-1440..=1440).contains(&utc_offset_minutes) || utc_offset_minutes % 15 != 0 {
            // Owned text: the offset may be negative.
            return Err(Error::InvalidConfig(Refusal::text(format!(
                "UTC offset {utc_offset_minutes} must be within -1440..=1440 minutes in 15-minute increments"
            ))));
        }
        Ok(Self {
            utc_offset_minutes,
            daylight_savings_status,
        })
    }

    /// Return the configured signed minutes west of UTC.
    pub fn utc_offset_minutes(self) -> i16 {
        self.utc_offset_minutes
    }

    /// Return whether the one-hour daylight-saving adjustment is active.
    pub fn daylight_savings_status(self) -> bool {
        self.daylight_savings_status
    }
}

impl<T: TransportPort + 'static> BACnetServer<T> {
    pub(super) async fn start_with_clock_mode(
        config: ServerConfig,
        db: ObjectDatabase,
        transport: T,
        clock_config: Option<ClockConfig>,
    ) -> Result<Self, StartError> {
        Self::start_with_clock_mode_and_bindings(config, db, transport, clock_config, Vec::new())
            .await
    }

    /// Start with the default system-UTC Device clock.
    ///
    /// The Staging plans due at start-up have written their targets by the
    /// time it returns. Their event notifications may still be on their way
    /// (a `stop()` at once drops those not yet sent), and it never waits on a
    /// recipient for them, a silent one included. Their COV fanout has run
    /// by then only when they made no event notification; otherwise it
    /// follows the notifications, waiting for them at most the APDU timeout
    /// (#2573). The server serves requests by then, so a task of its own
    /// makes those writes while `start` waits for it: a start left unpolled
    /// holds up none of those requests (#2781). So a request the server
    /// already has may be answered before those writes are made: on a
    /// multi-thread runtime, as before, and now on a current-thread one too.
    /// Dropped part way, the start drops the server, and that task with it
    /// once it next yields, after the write it is making at most.
    ///
    /// A refusal, a transport that won't start among them, hands back the
    /// database ([`StartError`]).
    pub async fn start(
        config: ServerConfig,
        db: ObjectDatabase,
        transport: T,
    ) -> Result<Self, StartError> {
        Self::start_with_clock(config, db, transport, ClockConfig::default()).await
    }

    /// Start with a validated Device clock configuration. Start-up Staging
    /// plans go as for [`Self::start`].
    pub async fn start_with_clock(
        config: ServerConfig,
        db: ObjectDatabase,
        transport: T,
        clock_config: ClockConfig,
    ) -> Result<Self, StartError> {
        Self::start_with_clock_mode(config, db, transport, Some(clock_config)).await
    }

    /// Start without a Device wall clock or time-synchronization execution.
    /// Start-up Staging plans go as for [`Self::start`].
    pub async fn start_clockless(
        config: ServerConfig,
        db: ObjectDatabase,
        transport: T,
    ) -> Result<Self, StartError> {
        Self::start_with_clock_mode(config, db, transport, None).await
    }

    /// The zone the Device clock keeps local time in: the Device's
    /// UTC_Offset and Daylight_Savings_Status. `None` on a clockless server.
    pub fn clock_zone(&self) -> Option<ClockConfig> {
        self.clock.as_ref().map(|clock| clock.zone())
    }

    /// Change the Device clock's zone while the server runs: daylight saving
    /// starts or ends, or the host corrects its UTC offset.
    ///
    /// The clock keeps its UTC instant, and a clock a TimeSynchronization
    /// set stays set. So Local_Date and Local_Time move by the difference
    /// between the zones, an hour for daylight saving alone, and the
    /// Device's UTC_Offset and Daylight_Savings_Status read the new pair.
    /// Both properties are read-only to a peer unless the server was built
    /// with [`clock_zone_writable`](ServerBuilder::clock_zone_writable); a
    /// peer's write then moves the clock the same way.
    ///
    /// The pair is replaced as one value under the clock's lock, which every
    /// clock read takes, so a read sees the old offset with the old
    /// daylight-saving status or the new with the new. The change waits for
    /// the database's write guard too, so no request sees it part-way: a
    /// ReadPropertyMultiple of both properties answers with one pair. Don't
    /// call it while holding a guard on [`database`](Self::database). The
    /// guard is never waited for in the caller's task, and a call dropped
    /// part way is as [`write_local`](Self::write_local)'s Cancellation section
    /// describes (#2772): one dropped after its task took the guard may
    /// still change the zone.
    ///
    /// Everything that judges local time reads this clock when it judges:
    /// the next Schedule pass, a Calendar's Present_Value, a Notification
    /// Class recipient's days and time window, a trend log's aligned
    /// interval and each new timestamp. Nothing is recalculated at the
    /// change, and timestamps already recorded stay as they were, as after a
    /// TimeSynchronization that moved local time by the same amount.
    ///
    /// The newest change wins (#2395). With
    /// [`clock_zone_writable`](ServerBuilder::clock_zone_writable) on, a
    /// half a peer wrote that the Device's Tags storage keeps is older than
    /// this call: it is dropped from storage, which is saved first with the
    /// database guard dropped, and `zone` applies in full. A save that fails
    /// is returned and changes nothing: the clock keeps its zone and storage
    /// its halves. A peer's later write wins again. With the opt-in off a
    /// half held stays in storage, unapplied.
    ///
    /// Refused, changing nothing, on a clockless server, and when the local
    /// date in `zone` would fall outside 1900 to 2154. It changes the clock
    /// the server installed at start: a database whose clock reader the
    /// application replaced keeps reading that reader.
    pub async fn set_clock_zone(&self, zone: ClockConfig) -> Result<(), Error> {
        let clock = Arc::clone(
            self.clock
                .as_ref()
                .ok_or_else(|| Error::unsupported("Device clock is disabled"))?,
        );
        let device = crate::local_device::initiating_device(&self.config.device);
        let db = Arc::clone(&self.db);
        let take = async move {
            let staged = stage_zone_reset(&db, device).await;
            (db.write_owned().await, staged)
        };
        let change = move |(mut requests, staged): (
            OwnedRwLockWriteGuard<ObjectDatabase>,
            Option<SaveWait>,
        )| {
            let mut authority = requests
                .get_mut(&device)
                .and_then(|device| device.device_authority_internal());
            let result = clock.check(zone).and_then(|()| match authority.as_mut() {
                Some(authority) => authority.reset_written_zone(),
                None => Ok(()),
            });
            if let (Some(staged), Some(authority)) = (&staged, authority.as_mut()) {
                authority.release_staged(staged);
            }
            result?;
            clock.set_zone(zone)
        };
        // Neither the stage nor the guard is waited for in the caller's
        // task (#2772).
        self.request_tasks
            .for_caller(take, change)
            .await
            // `None`: no change; its runtime dropped the task before it committed.
            .unwrap_or_else(|| Err(broadcaster::stopped()))
    }
}

/// Stage the drop of the zone halves the Device's storage keeps, and wait
/// for its save with the database guard dropped: the wait to release it
/// with, or `None` when there is nothing to drop.
async fn stage_zone_reset(
    db: &RwLock<ObjectDatabase>,
    device: ObjectIdentifier,
) -> Option<SaveWait> {
    loop {
        let step = db
            .write()
            .await
            .get_mut(&device)
            .and_then(|device| device.device_authority_internal())
            .map(|mut device| device.stage_zone_reset())?;
        match step {
            StageStep::Staged(wait) => {
                wait.clone().await;
                return Some(wait);
            }
            StageStep::Busy(wait) => wait.await,
            StageStep::Skip => return None,
        }
    }
}

impl<T: TransportPort + 'static> ServerBuilder<T> {
    /// Start the Device clock in `zone`: its UTC_Offset and
    /// Daylight_Savings_Status. The default is offset 0 without daylight
    /// saving. The first Schedule pass, which runs as the server starts,
    /// already judges local time in it;
    /// [`set_clock_zone`](BACnetServer::set_clock_zone) changes it later.
    /// With [`clock_zone_writable`](Self::clock_zone_writable) on, a half a
    /// peer wrote that the Device's storage keeps wins over it (#2395),
    /// until the host's next `set_clock_zone`; with it off, the server logs
    /// a warning naming the half it leaves unapplied.
    pub fn clock_zone(mut self, zone: ClockConfig) -> Self {
        self.clock_zone = zone;
        self
    }

    /// Let peers write the Device's UTC_Offset and Daylight_Savings_Status
    /// (#2395), as [`ServerConfig::clock_zone_writable`] does. Off by
    /// default: both are then read-only on the wire.
    pub fn clock_zone_writable(mut self, writable: bool) -> Self {
        self.config.clock_zone_writable = writable;
        self
    }
}

impl BipServerBuilder {
    /// Start the Device clock in `zone`; see [`ServerBuilder::clock_zone`].
    pub fn clock_zone(mut self, zone: ClockConfig) -> Self {
        self.clock_zone = zone;
        self
    }

    /// Let peers write the zone; see [`ServerBuilder::clock_zone_writable`].
    pub fn clock_zone_writable(mut self, writable: bool) -> Self {
        self.config.clock_zone_writable = writable;
        self
    }
}

#[cfg(feature = "sc-tls")]
impl super::ScServerBuilder {
    /// Start the Device clock in `zone`; see [`ServerBuilder::clock_zone`].
    pub fn clock_zone(mut self, zone: ClockConfig) -> Self {
        self.clock_zone = zone;
        self
    }

    /// Let peers write the zone; see [`ServerBuilder::clock_zone_writable`].
    pub fn clock_zone_writable(mut self, writable: bool) -> Self {
        self.config.clock_zone_writable = writable;
        self
    }
}

/// The zone the clock starts in: `zone`, with each half a peer wrote that
/// the local Device's storage keeps over it while peers may write the zone
/// (#2395). With the opt-in off a half held is left unapplied, in storage,
/// and named in a warning. With it on, a Device without storage gets a
/// warning that a written zone won't outlast the server (#2730).
pub(super) fn start_zone(
    db: &mut ObjectDatabase,
    zone: ClockConfig,
    writable: bool,
) -> ClockConfig {
    let device = db
        .local_device()
        .identifier()
        .and_then(|device| db.get_mut(&device))
        .and_then(|device| device.device_authority_internal())
        .map(|device| (device.written_zone(), device.has_storage()));
    let written = device.map(|(written, _)| written).unwrap_or_default();
    if writable && device.is_some_and(|(_, stored)| !stored) {
        tracing::warn!(
            "Peers may write the Device's zone, but the Device has no storage: a written \
             UTC_Offset or Daylight_Savings_Status lasts until the server stops \
             (device_storage_path keeps it)"
        );
    }
    if written.is_empty() {
        return zone;
    }
    if !writable {
        tracing::warn!(
            utc_offset = ?written.utc_offset,
            daylight_savings_status = ?written.daylight_savings_status,
            "Device storage holds a zone a peer wrote, left unapplied while peers may not write the zone"
        );
        return zone;
    }
    let (offset, summer) =
        written.over((zone.utc_offset_minutes(), zone.daylight_savings_status()));
    // Storage only keeps halves a write takes, so this holds.
    ClockConfig::new(offset, summer).unwrap_or(zone)
}

#[derive(Clone, Copy)]
struct ClockState {
    synchronized_utc: Option<(i128, Instant)>,
    config: ClockConfig,
}

/// Mutable server-owned controller behind the object layer's read-only port.
pub(super) struct ServerClock {
    state: Mutex<ClockState>,
    /// Whether a peer may write the zone (#2395).
    zone_writes: bool,
    /// How often the lock was taken, for tests that pin one hold.
    #[cfg(test)]
    holds: std::sync::atomic::AtomicUsize,
}

impl ServerClock {
    pub(super) fn new(config: ClockConfig) -> Self {
        Self {
            state: Mutex::new(ClockState {
                synchronized_utc: None,
                config,
            }),
            zone_writes: false,
            #[cfg(test)]
            holds: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    /// This clock, taking a peer's write of the zone when `on` (#2395).
    pub(super) fn taking_zone_writes(mut self, on: bool) -> Self {
        self.zone_writes = on;
        self
    }

    /// The state, poisoned or not (#2559): of its two fields, a zone change
    /// assigns `config` and a synchronization `synchronized_utc`, each as
    /// the writer's last step, so a panic leaves no half-written value.
    fn lock(&self) -> MutexGuard<'_, ClockState> {
        #[cfg(test)]
        self.holds
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.state.locked()
    }

    /// The zone local time is kept in.
    pub(super) fn zone(&self) -> ClockConfig {
        self.lock().config
    }

    /// Apply a fully specified local or UTC synchronization value, unless
    /// it would step the clock by more than `max_step`. `step` receives the
    /// step in hundredths, refused or not, when the clock could be read.
    ///
    /// The step is judged, the value converted and the clock set under one
    /// hold of the lock: the zone a local value is judged in is the zone it
    /// is stored under, whatever a zone change does meanwhile. Validation
    /// and conversion finish before the controller is mutated.
    pub(super) fn synchronize_within(
        &self,
        date: Date,
        time: Time,
        is_utc: bool,
        max_step: Option<Duration>,
        step: &mut Option<u128>,
    ) -> Result<(), Error> {
        let supplied_hundredths = date_time_to_hundredths(date, time)?;
        self.lock()
            .synchronize(supplied_hundredths, is_utc, max_step, step)
    }

    /// The UTC instant a synchronization anchored, if one did.
    #[cfg(test)]
    pub(super) fn synchronized_at(&self) -> Option<(i128, Instant)> {
        self.state.locked().synchronized_utc
    }

    /// Poison the state, as a panic under its lock would.
    #[cfg(test)]
    pub(super) fn poison_for_test(&self) {
        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _held = self.state.locked();
            panic!("poison the clock");
        }));
        assert!(unwound.is_err() && self.state.is_poisoned());
    }

    /// [`synchronize_within`](Self::synchronize_within) any step.
    #[cfg(test)]
    pub(super) fn synchronize(&self, date: Date, time: Time, is_utc: bool) -> Result<(), Error> {
        self.synchronize_within(date, time, is_utc, None, &mut None)
    }

    /// Whether the clock takes `config` now: refused, as
    /// [`set_zone`](Self::set_zone) refuses it, when its local date falls
    /// outside the BACnet Date range.
    pub(super) fn check(&self, config: ClockConfig) -> Result<(), Error> {
        self.lock()
            .frame_in(config)
            .map(drop)
            .ok_or(Error::invalid_argument(ZONE_OUT_OF_RANGE))
    }

    /// Replace the zone, keeping the UTC instant and whether a
    /// synchronization set it. The offset and the daylight-saving status
    /// change as one value under the lock every read takes, so no read
    /// pairs one zone's offset with the other's status. A zone whose local
    /// date falls outside the BACnet Date range now is refused, and changes
    /// nothing.
    pub(super) fn set_zone(&self, config: ClockConfig) -> Result<(), Error> {
        let mut state = self.lock();
        state
            .frame_in(config)
            .ok_or(Error::invalid_argument(ZONE_OUT_OF_RANGE))?;
        state.config = config;
        Ok(())
    }
}

impl ClockState {
    /// UTC now: the synchronized instant run on, or the system's.
    fn utc_hundredths(&self) -> Option<i128> {
        match self.synchronized_utc {
            Some((anchor, monotonic_anchor)) => {
                let elapsed = crate::runtime_clock::now().duration_since(monotonic_anchor);
                let elapsed_hundredths = i128::try_from(elapsed.as_millis() / 10).ok()?;
                Some(anchor + elapsed_hundredths)
            }
            None => Some(system_utc_hundredths()),
        }
    }

    /// The frame now, read in `config`.
    fn frame_in(&self, config: ClockConfig) -> Option<ClockFrame> {
        frame_from_utc_hundredths(self.utc_hundredths()?, config)
    }

    /// Judge `supplied`'s step against `max_step` and store it, both in
    /// this state's one zone.
    fn synchronize(
        &mut self,
        supplied: i128,
        is_utc: bool,
        max_step: Option<Duration>,
        step: &mut Option<u128>,
    ) -> Result<(), Error> {
        let config = self.config;
        *step = self
            .frame_in(config)
            .and_then(|frame| time_sync_policy::step_hundredths(supplied, is_utc, frame));
        time_sync_policy::check_step(*step, max_step)?;
        let utc_hundredths = if is_utc {
            supplied
        } else {
            supplied + i128::from(config.utc_offset_minutes) * HUNDREDTHS_PER_MINUTE
                - if config.daylight_savings_status {
                    60 * HUNDREDTHS_PER_MINUTE
                } else {
                    0
                }
        };

        // The peer's value, refused as out of range here (#1833).
        frame_from_utc_hundredths(utc_hundredths, config).ok_or_else(|| {
            Error::from(DecodeError::new(
                DecodingKind::OutOfRange,
                DATE_AT,
                "synchronized time is outside the BACnet Date range",
            ))
        })?;

        self.synchronized_utc = Some((utc_hundredths, crate::runtime_clock::now()));
        Ok(())
    }
}

impl ClockReader for ServerClock {
    /// One copy of the state, taken under the lock, makes the whole frame.
    /// The zone in it is a single `Copy` value, which
    /// [`set_zone`](ServerClock::set_zone) assigns whole under the same
    /// lock, so a frame's UTC_Offset and Daylight_Savings_Status, and the
    /// local time worked out from them, always come from one zone.
    fn read_clock(&self) -> Option<ClockFrame> {
        let state = *self.lock();
        state.frame_in(state.config)
    }

    fn takes_zone_writes(&self) -> bool {
        self.zone_writes
    }

    /// Whether a peer's write leaves a zone the clock takes now: an offset
    /// off the 15-minute grid, or a zone whose local date falls outside the
    /// BACnet Date range, is VALUE_OUT_OF_RANGE.
    fn check_zone(&self, utc_offset: i16, daylight_savings_status: bool) -> Result<(), Error> {
        let range = || {
            Error::protocol(
                bacnet_types::enums::ErrorClass::PROPERTY,
                bacnet_types::enums::ErrorCode::VALUE_OUT_OF_RANGE,
            )
        };
        let zone = ClockConfig::new(utc_offset, daylight_savings_status).map_err(|_| range())?;
        self.lock().frame_in(zone).map(drop).ok_or_else(range)
    }

    /// A peer's write, made as [`set_zone`](ServerClock::set_zone) makes
    /// the host's, once [`check_zone`](ClockReader::check_zone) took it.
    /// The request has held the database for writing since, and every
    /// other writer of the clock (a host's zone change, a
    /// TimeSynchronization) takes the database guard too, so the zone it
    /// checked is still one the clock takes, and a reader holding the
    /// guard sees one pair.
    fn write_zone(&self, utc_offset: i16, daylight_savings_status: bool) -> Result<(), Error> {
        let zone = ClockConfig::new(utc_offset, daylight_savings_status).map_err(|_| {
            Error::protocol(
                bacnet_types::enums::ErrorClass::PROPERTY,
                bacnet_types::enums::ErrorCode::VALUE_OUT_OF_RANGE,
            )
        })?;
        self.lock().config = zone;
        Ok(())
    }
}

#[cfg(test)]
pub(crate) fn clocked_test_database() -> ObjectDatabase {
    let mut db = ObjectDatabase::new();
    db.set_clock_reader(Some(std::sync::Arc::new(ServerClock::new(
        ClockConfig::default(),
    ))));
    db
}

fn system_utc_hundredths() -> i128 {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO);
    i128::from(elapsed.as_secs()) * HUNDREDTHS_PER_SECOND + i128::from(elapsed.subsec_millis() / 10)
}

pub(super) fn date_time_to_hundredths(date: Date, time: Time) -> Result<i128, Error> {
    let year = date
        .actual_year()
        .ok_or_else(|| invalid_datetime(DATE_AT, "date contains an unspecified year"))?;
    if !(1..=12).contains(&date.month) {
        return Err(invalid_datetime(DATE_AT, "date month must be 1..=12"));
    }
    let max_day = days_in_month(year, date.month);
    if date.day == 0 || date.day > max_day {
        return Err(invalid_datetime(
            DATE_AT,
            "date day is invalid for its month",
        ));
    }
    if !(1..=7).contains(&date.day_of_week) {
        return Err(invalid_datetime(DATE_AT, "date day-of-week must be 1..=7"));
    }

    let days = days_from_civil(i64::from(year), i64::from(date.month), i64::from(date.day));
    let expected_day_of_week = (days + 3).rem_euclid(7) as u8 + 1;
    if date.day_of_week != expected_day_of_week {
        return Err(invalid_datetime(
            DATE_AT,
            "date day-of-week is inconsistent",
        ));
    }
    if time.hour > 23 || time.minute > 59 || time.second > 59 || time.hundredths > 99 {
        return Err(invalid_datetime(
            TIME_AT,
            "time must be fully specified and within its field ranges",
        ));
    }

    let day_hundredths = i128::from(time.hour) * 60 * HUNDREDTHS_PER_MINUTE
        + i128::from(time.minute) * HUNDREDTHS_PER_MINUTE
        + i128::from(time.second) * HUNDREDTHS_PER_SECOND
        + i128::from(time.hundredths);
    Ok(i128::from(days) * HUNDREDTHS_PER_DAY + day_hundredths)
}

/// A zone whose local date now falls outside the BACnet Date range: the
/// argument of the `set_clock_zone` call that hands it over (#1833).
const ZONE_OUT_OF_RANGE: &str = "local time in that zone is outside the BACnet Date range";

/// Where a time synchronization request's Date and Time start: each is an
/// application tag of one octet and four contents octets, the Date first
/// (Clauses 16.7 and 16.8).
const DATE_AT: usize = 0;
const TIME_AT: usize = 5;

/// A time synchronization's date or time the service doesn't take: the
/// peer's value at `offset` in its request, refused as out of range
/// (#1833).
fn invalid_datetime(offset: usize, message: &'static str) -> Error {
    DecodeError::new(DecodingKind::OutOfRange, offset, message)
        .within("invalid time synchronization value")
        .into()
}

fn frame_from_utc_hundredths(utc_hundredths: i128, config: ClockConfig) -> Option<ClockFrame> {
    let dst_minutes = if config.daylight_savings_status {
        60
    } else {
        0
    };
    let local_hundredths = utc_hundredths
        - i128::from(config.utc_offset_minutes) * HUNDREDTHS_PER_MINUTE
        + i128::from(dst_minutes) * HUNDREDTHS_PER_MINUTE;
    let days = local_hundredths.div_euclid(HUNDREDTHS_PER_DAY);
    let within_day = local_hundredths.rem_euclid(HUNDREDTHS_PER_DAY);
    let days = i64::try_from(days).ok()?;
    let (year, month, day) = civil_from_days(days);
    let encoded_year = year.checked_sub(1900)?;
    if !(0..=254).contains(&encoded_year) {
        return None;
    }

    let total_seconds = within_day / HUNDREDTHS_PER_SECOND;
    Some(ClockFrame {
        local_date: Date {
            year: encoded_year as u8,
            month: month as u8,
            day: day as u8,
            day_of_week: (days + 3).rem_euclid(7) as u8 + 1,
        },
        local_time: Time {
            hour: (total_seconds / 3600) as u8,
            minute: ((total_seconds % 3600) / 60) as u8,
            second: (total_seconds % 60) as u8,
            hundredths: (within_day % HUNDREDTHS_PER_SECOND) as u8,
        },
        utc_offset: config.utc_offset_minutes,
        daylight_savings_status: config.daylight_savings_status,
    })
}

fn days_in_month(year: u16, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

fn is_leap_year(year: u16) -> bool {
    year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400))
}

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = year - i64::from(month <= 2);
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month_prime = month + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * month_prime + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(year: u16, month: u8, day: u8, day_of_week: u8) -> Date {
        Date {
            year: (year - 1900) as u8,
            month,
            day,
            day_of_week,
        }
    }

    fn time(hour: u8, minute: u8, second: u8, hundredths: u8) -> Time {
        Time {
            hour,
            minute,
            second,
            hundredths,
        }
    }

    #[test]
    fn clock_config_rejects_invalid_offsets() {
        assert!(ClockConfig::new(-1440, false).is_ok());
        assert!(ClockConfig::new(1440, true).is_ok());
        assert!(ClockConfig::new(14, false).is_err());
        assert!(ClockConfig::new(1441, false).is_err());
    }

    #[test]
    fn civil_conversion_covers_offsets_dst_and_rollover() {
        let utc = date_time_to_hundredths(date(2024, 3, 1, 5), time(0, 30, 0, 25)).unwrap();

        let west = frame_from_utc_hundredths(utc, ClockConfig::new(60, false).unwrap()).unwrap();
        assert_eq!(west.local_date, date(2024, 2, 29, 4));
        assert_eq!(west.local_time, time(23, 30, 0, 25));

        let east = frame_from_utc_hundredths(utc, ClockConfig::new(-60, false).unwrap()).unwrap();
        assert_eq!(east.local_date, date(2024, 3, 1, 5));
        assert_eq!(east.local_time, time(1, 30, 0, 25));

        let dst = frame_from_utc_hundredths(utc, ClockConfig::new(60, true).unwrap()).unwrap();
        assert_eq!(dst.local_date, date(2024, 3, 1, 5));
        assert_eq!(dst.local_time, time(0, 30, 0, 25));
    }

    #[test]
    fn synchronization_validation_rejects_wildcards_calendar_errors_and_inconsistent_days() {
        let clock = ServerClock::new(ClockConfig::default());
        let anchor = clock.state.lock().unwrap().synchronized_utc;
        for invalid_date in [
            date(2023, 2, 29, 3),
            date(2024, 2, 29, 5),
            Date {
                year: Date::UNSPECIFIED,
                month: 1,
                day: 1,
                day_of_week: 1,
            },
        ] {
            // The peer's Date, at the start of its request (#1833).
            let refused = clock.synchronize(invalid_date, time(12, 0, 0, 0), true);
            assert!(matches!(refused, Err(Error::Decoding(d)) if d.offset == DATE_AT));
        }
        // Its Time, after the Date's five octets.
        let unspecified = Time {
            hour: Time::UNSPECIFIED,
            minute: 0,
            second: 0,
            hundredths: 0,
        };
        for invalid_time in [unspecified, time(24, 0, 0, 0)] {
            let refused = clock.synchronize(date(2024, 2, 29, 4), invalid_time, true);
            assert!(matches!(refused, Err(Error::Decoding(d)) if d.offset == TIME_AT));
        }
        assert_eq!(clock.state.lock().unwrap().synchronized_utc, anchor);
    }

    #[tokio::test(start_paused = true)]
    async fn local_and_utc_synchronization_share_the_configured_frame() {
        let clock = ServerClock::new(ClockConfig::new(300, true).unwrap());
        clock
            .synchronize(date(2024, 7, 4, 4), time(9, 15, 0, 0), false)
            .unwrap();
        let local = clock.read_clock().unwrap();
        assert_eq!(local.local_date, date(2024, 7, 4, 4));
        assert_eq!((local.local_time.hour, local.local_time.minute), (9, 15));

        clock
            .synchronize(date(2024, 7, 4, 4), time(13, 15, 0, 0), true)
            .unwrap();
        let utc = clock.read_clock().unwrap();
        assert_eq!(utc.local_date, date(2024, 7, 4, 4));
        assert_eq!((utc.local_time.hour, utc.local_time.minute), (9, 15));
        assert_eq!(utc.utc_offset, 300);
        assert!(utc.daylight_savings_status);
    }

    #[tokio::test(start_paused = true)]
    async fn device_schedule_recipient_and_cov_use_the_same_frame() {
        use bacnet_objects::database::ObjectDatabase;
        use bacnet_objects::device::{DeviceConfig, DeviceObject};
        use bacnet_objects::traits::BACnetObject;
        use bacnet_types::enums::PropertyIdentifier;
        use bacnet_types::primitives::PropertyValue;
        use std::sync::Arc;

        let clock = Arc::new(ServerClock::new(ClockConfig::new(300, true).unwrap()));
        clock
            .synchronize(date(2024, 7, 4, 4), time(9, 15, 0, 0), false)
            .unwrap();

        let device = DeviceObject::new(DeviceConfig::default()).unwrap();
        let device_oid = device.object_identifier();
        let mut db = ObjectDatabase::new();
        db.add(Box::new(device)).unwrap();
        db.set_clock_reader(Some(
            Arc::clone(&clock) as Arc<dyn bacnet_objects::clock::ClockReader>
        ));

        let frame = db.clock_frame().unwrap();
        assert_eq!(
            db.get(&device_oid)
                .unwrap()
                .read_property(PropertyIdentifier::LOCAL_TIME, None)
                .unwrap(),
            PropertyValue::Time(frame.local_time)
        );
        let (today, now) = crate::schedule::schedule_instant(frame).unwrap();
        assert_eq!(today.to_date(), frame.local_date);
        assert_eq!(today.weekday(), 4);
        assert_eq!(now, frame.local_time);
        assert_eq!(
            frame.day_of_week(),
            Some(bacnet_types::bitstring::DaysOfWeek::THURSDAY)
        );
        assert_eq!(
            bacnet_device_core::cov::multiple_report::cov_multiple_datetime(frame),
            (frame.local_date, frame.local_time)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn synchronized_clock_crosses_hundredths_at_second_and_minute_edges() {
        for (anchor, after) in [
            (time(0, 0, 0, 99), time(0, 0, 1, 0)),
            (time(0, 0, 59, 99), time(0, 1, 0, 0)),
        ] {
            let clock = ServerClock::new(ClockConfig::default());
            clock
                .synchronize(date(2024, 7, 4, 4), anchor, true)
                .unwrap();
            assert_eq!(clock.read_clock().unwrap().local_time, anchor);
            tokio::time::advance(Duration::from_millis(9)).await;
            assert_eq!(clock.read_clock().unwrap().local_time, anchor);
            tokio::time::advance(Duration::from_millis(1)).await;
            let frame = clock.read_clock().unwrap();
            assert_eq!(frame.local_date, date(2024, 7, 4, 4));
            assert_eq!(frame.local_time, after);
        }
    }
}

#[cfg(test)]
#[path = "clock_vectors.rs"]
mod vectors;

#[cfg(test)]
#[path = "clock_zone_tests.rs"]
mod zone_tests;

#[cfg(test)]
#[path = "clock_zone_write_tests.rs"]
mod zone_write_tests;

#[cfg(test)]
#[path = "clock_zone_kept_tests.rs"]
mod zone_kept_tests;
