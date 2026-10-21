//! The clock the hosted MS/TP and B/IP nodes run their cores on.

use bacnet_time::HostClock;

/// Tokio's time on the cores' timeline, from now: its zero is this call.
///
/// The clock reads `tokio::time::Instant`, as a std instant, so paused-time
/// tests drive the cores' deadlines with `tokio::time::advance`, and a timer
/// set at the host instant of a deadline fires on the same clock.
///
/// bacnet-network's router has the same constructor (`router/engine.rs`).
pub(crate) fn tokio_clock() -> HostClock {
    let now = || tokio::time::Instant::now().into_std();
    HostClock::new(now()).reading(now)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_time::Instant;

    /// Paused time moves the clock, and a core deadline maps to a Tokio
    /// instant and back exactly, so a timer set there never wakes early.
    #[tokio::test(start_paused = true)]
    async fn the_clock_follows_paused_time_and_round_trips() {
        let clock = tokio_clock();
        assert_eq!(clock.now(), Instant::ZERO);
        tokio::time::advance(std::time::Duration::from_nanos(2_500)).await;
        let now = clock.now();
        assert_eq!(now, Instant::from_nanos(2_500));
        assert_eq!(
            clock.to_std(now).map(tokio::time::Instant::from_std),
            Some(tokio::time::Instant::now())
        );
        let deadline = Instant::from_nanos(10_001);
        let wake = clock.to_std(deadline).unwrap();
        assert_eq!(clock.instant(wake), deadline);
    }
}
