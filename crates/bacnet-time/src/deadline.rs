//! [`earliest`]: merging optional deadlines.

/// The earliest of `deadlines`, where `None` is no deadline: `None` only
/// when none of them is set.
///
/// `Option::min` and `Ord::min` on `Option<T>` order `None` first, so they
/// make "no deadline" win, and a timer merged that way never fires. This one
/// skips it. It takes any ordered time: this crate's [`Instant`], or a
/// host's own, such as `std::time::Instant`.
///
/// ```
/// use bacnet_time::{earliest, Instant};
///
/// let at = Some(Instant::from_millis(5));
/// assert_eq!(earliest([None, at]), at);
/// assert_eq!(Option::min(None, at), None);
/// assert_eq!(earliest([at, None, Some(Instant::from_millis(3))]), Some(Instant::from_millis(3)));
/// assert_eq!(earliest::<Instant>([None, None]), None);
/// ```
///
/// [`Instant`]: crate::Instant
#[must_use]
pub fn earliest<T: Ord>(deadlines: impl IntoIterator<Item = Option<T>>) -> Option<T> {
    deadlines.into_iter().flatten().min()
}
