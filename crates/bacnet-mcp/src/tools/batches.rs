//! A page's objects a batch a call (#2785), for the tools that page a
//! device's objects as the inventory's point records, `bacnet_inventory`
//! and `bacnet_export_223`.
//!
//! A batch is one call of
//! [`Device::point_records`](bacnet_ops::Device::point_records): as many
//! objects as one ReadPropertyMultiple holds
//! ([`point_batch`](bacnet_ops::Device::point_batch); one once the device
//! has refused RPM), no more than the page can still take, and no more
//! than the device's slow limit allows. A page's later batch still being
//! read [`OBJECT_GRACE`] past the call's budget is let go and read again
//! on the next page, so the records before it come back before the call's
//! own timeout drops them; a page's first batch still being read
//! [`FIRST_GRACE`] past it fails the call, naming the batch ([`stuck`]).
//!
//! Either way, for a batch of several objects, the device is slow at that
//! size, so its slow limit ([`Ctx::slow`](crate::context::Ctx)) becomes
//! half the properties of the batch dropped: the next attempt there asks
//! for fewer objects, down to one, which is #2786's per-object rule. No
//! page is stuck for good on a batch too slow to read whole. The limit
//! lives in the server, not the cursor, since a stuck first batch returns
//! no cursor; it lasts [`LIST_TTL`], as a cached Object_List does, so a
//! device slowed by one passing blip is asked in full batches again, and a
//! device still slow pays one dropped batch again.

use std::collections::HashMap;
use std::time::Duration;

use bacnet_ops::inventory::properties;
use bacnet_ops::label::kebab;
use bacnet_ops::{DeviceAddress, InventoryOptions, OpsError, PointRecords};
use bacnet_transport::port::TransportPort;
use bacnet_types::primitives::ObjectIdentifier;
use bacnet_types::sync::MutexExt;
use tokio::time::Instant;

use super::{GRACE, out_of_budget};
use crate::context::{Call, Ctx, Device, LIST_CAP, LIST_TTL, SlowLimit};
use crate::errors::ToolError;
use crate::value::object_text;

/// How long past the call's budget a page's later batch may take: half
/// the grace the call has, so the records read before it come back before
/// the call's own timeout drops them.
pub(super) const OBJECT_GRACE: Duration = Duration::from_secs(GRACE.as_secs() / 2);

/// How long past the budget a page's first batch may take: the call's
/// grace, less a second, so it is this tool that names the batch rather
/// than the call's own timeout saying nothing of it.
pub(super) const FIRST_GRACE: Duration = GRACE.saturating_sub(Duration::from_secs(1));

/// What one batch came to.
pub(super) enum Batch {
    /// Its records, in the order asked, and why the reading stopped
    /// early, if it did.
    Read(PointRecords),
    /// Nothing read, for a reason found before a request went (the route
    /// gone, say) or the client's own failure.
    Failed(OpsError),
    /// Let go [`OBJECT_GRACE`] past the budget: its objects go to the next
    /// page.
    LetGo,
}

/// Read the next batch of `rest`, the objects from where the page goes
/// on, as many as it can still take; `opening` when it is the page's
/// first. Returns the objects asked for, from the first of `rest`, and
/// what came of them.
///
/// # Errors
///
/// A page's first batch still being read [`FIRST_GRACE`] past the budget
/// ([`stuck`]).
pub(super) async fn next_batch<'a, T: TransportPort + 'static>(
    ctx: &Ctx<T>,
    call: &Call,
    device: &Device<T>,
    rest: &'a [ObjectIdentifier],
    options: &InventoryOptions,
    opening: bool,
) -> Result<(&'a [ObjectIdentifier], Batch), ToolError> {
    let count = device.handle.point_batch(rest, options);
    let count = count.min(within(slow_limit(ctx, device), rest, options));
    let batch = &rest[..count];
    let grace = if opening { FIRST_GRACE } else { OBJECT_GRACE };
    let read = device.handle.point_records(batch, options);
    match tokio::time::timeout_at(call.deadline + grace, read).await {
        Ok(Ok(read)) => Ok((batch, Batch::Read(read))),
        Ok(Err(error)) => Ok((batch, Batch::Failed(error))),
        Err(_) => {
            slowed(ctx, device, batch, options);
            if opening {
                let budget = ctx.policy.caps.budget;
                return Err(stuck(call, budget, device.instance, batch));
            }
            call.timed_out();
            Ok((batch, Batch::LetGo))
        }
    }
}

/// The most properties a batch asks `device` for, once a batch of several
/// objects outlasted a call's budget there.
fn slow_limit<T: TransportPort + 'static>(ctx: &Ctx<T>, device: &Device<T>) -> Option<usize> {
    let mut slow = ctx.slow.locked();
    limit_of(&mut slow, &device.address, device.instance, Instant::now())
}

/// The limit `slow` holds for the device `instance` at `address` at `now`,
/// forgetting one learned [`LIST_TTL`] or more before, or for another
/// instance: another device may answer there now.
fn limit_of(
    slow: &mut HashMap<DeviceAddress, SlowLimit>,
    address: &DeviceAddress,
    instance: u32,
    now: Instant,
) -> Option<usize> {
    let limit = slow.get(address)?;
    if limit.instance != instance || now.saturating_duration_since(limit.learned) >= LIST_TTL {
        slow.remove(address);
        return None;
    }
    Some(limit.properties)
}

/// How many of `objects`, from the first, ask for no more than `limit`
/// properties together under `options`: at least one, and all of them
/// with no limit.
fn within(limit: Option<usize>, objects: &[ObjectIdentifier], options: &InventoryOptions) -> usize {
    let Some(limit) = limit else {
        return objects.len();
    };
    let mut total = 0;
    let fitting = objects
        .iter()
        .take_while(|object| {
            total += properties(object.object_type(), options).len();
            total <= limit
        })
        .count();
    fitting.max(1).min(objects.len())
}

/// Remember that `batch` outlasted a call's budget at `device`: batches
/// there ask for half its properties for the next [`LIST_TTL`], down to
/// one object. A batch of one teaches nothing, one being the least a batch
/// holds: one slow object, which the agent can step around, leaves the
/// rest of the device's batches as they were.
fn slowed<T: TransportPort + 'static>(
    ctx: &Ctx<T>,
    device: &Device<T>,
    batch: &[ObjectIdentifier],
    options: &InventoryOptions,
) {
    if batch.len() < 2 {
        return;
    }
    let asked: usize = batch
        .iter()
        .map(|object| properties(object.object_type(), options).len())
        .sum();
    let mut slow = ctx.slow.locked();
    let limit = SlowLimit {
        properties: (asked / 2).max(1),
        instance: device.instance,
        learned: Instant::now(),
    };
    remember(&mut slow, &device.address, limit);
}

/// Keep `limit` for `address`, or the smaller one still kept for the same
/// device, learned now; when [`LIST_CAP`] devices are kept, the one
/// learned longest ago goes.
fn remember(
    slow: &mut HashMap<DeviceAddress, SlowLimit>,
    address: &DeviceAddress,
    limit: SlowLimit,
) {
    let kept = limit_of(slow, address, limit.instance, limit.learned);
    let properties = kept.map_or(limit.properties, |kept| kept.min(limit.properties));
    if !slow.contains_key(address) && slow.len() >= LIST_CAP {
        let oldest = slow
            .iter()
            .min_by_key(|(_, kept)| kept.learned)
            .map(|(address, _)| address.clone());
        if let Some(oldest) = oldest {
            slow.remove(&oldest);
        }
    }
    slow.insert(
        address.clone(),
        SlowLimit {
            properties,
            ..limit
        },
    );
}

/// The error for a page's first batch, `batch`, still being read
/// [`FIRST_GRACE`] past the budget: the call's own (a timeout once it has
/// sent requests), naming the batch's first object, where a cursor would
/// go on, and the ways past it. A batch of one is #2786's: read it alone
/// or leave its type out. A larger one names its range too, and the next
/// call asks for fewer objects there ([`slowed`]).
pub(super) fn stuck(
    call: &Call,
    budget: Duration,
    device: u32,
    batch: &[ObjectIdentifier],
) -> ToolError {
    let object = batch[0];
    let error = match out_of_budget(call, budget) {
        ToolError::Timeout { busy, .. } => {
            call.timed_out();
            let after = call.started.elapsed().as_millis();
            ToolError::Timeout {
                device: Some(device),
                after_ms: u64::try_from(after).unwrap_or(u64::MAX),
                busy,
            }
        }
        refused => refused,
    };
    let kind = kebab(object.object_type());
    let leave_out = format!(
        "leave its type, {kind}, out: give types the device's other types \
         (bacnet_describe_device lists them) and start again without a cursor"
    );
    let hint = match batch {
        [_] => format!(
            "it alone outlasted the call's budget. Read it with bacnet_read, a property at a \
             time; or {leave_out}"
        ),
        [.., last] => format!(
            "the {} objects from it to {}, read together, outlasted the call's budget. Call \
             again the same way: the next call reads fewer at a time here, down to one. Or {}",
            batch.len(),
            object_text(last.object_type(), last.instance_number()),
            leave_out
        ),
        [] => unreachable!("a batch holds an object"),
    };
    ToolError::At {
        error: Box::new(error),
        object: object_text(object.object_type(), object.instance_number()),
        hint,
    }
}

#[cfg(test)]
mod tests {
    use bacnet_types::enums::ObjectType;

    use super::*;

    fn at(n: u8) -> DeviceAddress {
        DeviceAddress::local(&[10, 0, 0, n, 0xBA, 0xC0]).unwrap()
    }

    fn limit(properties: usize, instance: u32, learned: Instant) -> SlowLimit {
        SlowLimit {
            properties,
            instance,
            learned,
        }
    }

    /// A limit below one object's properties still reads that one object.
    #[test]
    fn a_limit_below_one_object_reads_one() {
        let options = InventoryOptions::new().with_values(true);
        let inputs: Vec<ObjectIdentifier> = (1..=4)
            .map(|n| ObjectIdentifier::new(ObjectType::ANALOG_INPUT, n).unwrap())
            .collect();
        // An analog input with values asks for seven.
        assert_eq!(within(Some(1), &inputs, &options), 1);
        assert_eq!(within(Some(6), &inputs, &options), 1);
        assert_eq!(within(Some(14), &inputs, &options), 2);
        assert_eq!(within(None, &inputs, &options), 4);
    }

    /// A limit lasts five minutes, the Object_List cache's time, and is
    /// forgotten when the address has another instance; the same device
    /// keeps the smaller of two.
    #[tokio::test(start_paused = true)]
    async fn a_limit_expires_and_belongs_to_its_device() {
        let mut slow = HashMap::new();
        remember(&mut slow, &at(2), limit(16, 7, Instant::now()));
        remember(&mut slow, &at(2), limit(20, 7, Instant::now()));
        assert_eq!(limit_of(&mut slow, &at(2), 7, Instant::now()), Some(16));
        tokio::time::advance(LIST_TTL - Duration::from_secs(1)).await;
        assert_eq!(limit_of(&mut slow, &at(2), 7, Instant::now()), Some(16));
        tokio::time::advance(Duration::from_secs(1)).await;
        let expired = limit_of(&mut slow, &at(2), 7, Instant::now());
        assert_eq!(expired, None, "expired");
        assert!(slow.is_empty());
        remember(&mut slow, &at(3), limit(8, 7, Instant::now()));
        let other = limit_of(&mut slow, &at(3), 8, Instant::now());
        assert_eq!(other, None, "another device at the address");
        assert!(slow.is_empty());
    }

    /// At the cap the limit learned longest ago goes, whatever the map's
    /// order.
    #[tokio::test(start_paused = true)]
    async fn the_oldest_limit_goes_at_the_cap() {
        let mut slow = HashMap::new();
        let cap = u8::try_from(LIST_CAP).unwrap();
        // Learned from the last address down, so the oldest is the last.
        for n in (1..=cap).rev() {
            remember(&mut slow, &at(n), limit(4, u32::from(n), Instant::now()));
            tokio::time::advance(Duration::from_millis(10)).await;
        }
        assert_eq!(slow.len(), LIST_CAP);
        remember(&mut slow, &at(200), limit(4, 200, Instant::now()));
        assert_eq!(slow.len(), LIST_CAP);
        assert!(!slow.contains_key(&at(cap)), "the oldest went");
        assert!(slow.contains_key(&at(1)) && slow.contains_key(&at(200)));
    }
}
