//! Legacy subscriptions (#2114): `resources/subscribe` and
//! `resources/unsubscribe`, which `rmcp` 3.5.1 still dispatches for
//! sessions that opened with the older `initialize` handshake.
//!
//! A subscribe is checked as a listen stream's URI is (design §6): a point
//! URI needs its device in the discovery table and in reach, and takes a
//! watch on the hub under the same caps, so listen streams, legacy
//! subscriptions and `bacnet_watch` share the 32 points a principal and
//! the 256 a process. Unlike the acknowledgment, which leaves a refused
//! URI out, a refused subscribe is a JSON-RPC error, coded as a refused
//! resource read is. Subscribing twice to one URI is one subscription;
//! unsubscribing drops it and its watch at once.
//!
//! The session's notifier ([`notify`]) follows the subscriptions with the
//! listen streams' [`Follow`] and sends `notifications/resources/updated`
//! to the peer. It holds no lease: the watches live in [`Subs`], which the
//! session's end clears whatever way it ends, so a notifier that stops or
//! panics strands nothing.
//!
//! [`Subs`]: crate::listen::Subs

use std::sync::Arc;

use bacnet_transport::port::TransportPort;
use rmcp::model::ResourceUpdatedNotificationParam;
use rmcp::service::Peer;
use rmcp::{ErrorData, RoleServer};

use crate::context::Ctx;
use crate::errors::ToolError;
use crate::follow::{Follow, Wake};
use crate::listen::SHUTTING_DOWN;
use crate::resources::uri::{self, Uri};

/// The most URIs one session subscribes to: twice the principal's point
/// cap, so a full set of points leaves room for the other resources and a
/// second spelling of a point, and no more. The listen streams' per-device
/// alarm URIs count against it too ([`crate::listen::Subs::uris`]).
pub(crate) fn max_uris<T: TransportPort + 'static>(ctx: &Ctx<T>) -> usize {
    ctx.policy.caps.watches.saturating_mul(2)
}

/// A refusal before anything was sent, as a resource read reports it.
fn refusal(error: &ToolError) -> ErrorData {
    ErrorData::invalid_params(error.summary(), Some(error.json()))
}

/// The refusal for a cap reached.
fn over_cap(reason: &'static str) -> ToolError {
    let message = match reason {
        "process-cap" => "the server watches as many points as it may",
        SHUTTING_DOWN => "the server is shutting down",
        "passive" => "the server is passive: it builds no client and sends nothing",
        _ => "this principal watches as many points as it may; unsubscribe from some first",
    };
    ToolError::refused(reason, message)
}

/// Subscribe the session to `text`.
pub(crate) fn subscribe<T: TransportPort + 'static>(
    ctx: &Ctx<T>,
    text: &str,
) -> Result<(), ErrorData> {
    let parsed = uri::parse(text).map_err(|why| refusal(&ToolError::invalid("uri", why)))?;
    if ctx.subs.legacy_has(text) {
        return Ok(());
    }
    if ctx.hub.is_none() && !matches!(parsed, Uri::Session | Uri::Ledger) {
        return Err(refusal(&over_cap("passive")));
    }
    // Before the hold, which may start a watch: a refusal sends nothing.
    // `legacy_add` checks again under its lock.
    if ctx.subs.uris() >= max_uris(ctx) {
        return Err(refusal(&too_many(ctx)));
    }
    let hold = match parsed {
        Uri::Point(point) => {
            ctx.reach_check(point.device).map_err(|e| refusal(&e))?;
            let hold = ctx
                .subs
                .hold(ctx, point)
                .map_err(|reason| refusal(&over_cap(reason)))?;
            Some(hold)
        }
        Uri::DeviceAlarms(device) => {
            ctx.reach_check(device).map_err(|e| refusal(&e))?;
            None
        }
        Uri::Devices | Uri::DevicesAfter(_) | Uri::Alarms | Uri::Session | Uri::Ledger => None,
        Uri::Device(_) => {
            return Err(refusal(&ToolError::invalid(
                "uri",
                "a device summary has no updates; subscribe to its points or its alarms",
            )));
        }
    };
    ctx.subs
        .legacy_add(text, hold, max_uris(ctx))
        .map_err(|reason| match reason {
            SHUTTING_DOWN => refusal(&over_cap(reason)),
            _ => refusal(&too_many(ctx)),
        })
}

/// The refusal for a session at its URI bound.
fn too_many<T: TransportPort + 'static>(ctx: &Ctx<T>) -> ToolError {
    ToolError::refused(
        "principal-cap",
        format!(
            "this session subscribes to {} resources, its listen streams' device alarms \
             included, as many as it may; unsubscribe from some first",
            max_uris(ctx)
        ),
    )
}

/// Drop the session's subscription to `text`, if it has one.
pub(crate) fn unsubscribe<T: TransportPort + 'static>(ctx: &Ctx<T>, text: &str) {
    ctx.subs.legacy_remove(text);
}

/// Send the session's legacy subscriptions their updates until the server
/// closes or the peer goes. The session owns the task and aborts it when
/// it ends.
pub(crate) async fn notify<T: TransportPort + 'static>(ctx: Arc<Ctx<T>>, peer: Peer<RoleServer>) {
    let mut changed = ctx.subs.legacy_watch();
    let mut follow = Follow::new();
    // Subscriptions made before this task first ran: the receiver counts
    // them as seen already.
    follow.set(
        ctx.engine.as_deref(),
        &ctx.writes.signals,
        ctx.subs.legacy_snapshot(),
    );
    loop {
        let wake = tokio::select! {
            biased;
            () = ctx.closing.cancelled() => return,
            seen = changed.changed() => {
                if seen.is_err() {
                    return;
                }
                follow.set(
                    ctx.engine.as_deref(),
                    &ctx.writes.signals,
                    ctx.subs.legacy_snapshot(),
                );
                continue;
            }
            wake = follow.next() => wake,
        };
        match wake {
            Wake::Updated(name) => {
                if send(&peer, name).await.is_err() {
                    return;
                }
            }
            Wake::Devices => {
                for name in follow.device_uris() {
                    if send(&peer, name.clone()).await.is_err() {
                        return;
                    }
                }
            }
            Wake::Alarms => {
                for name in follow.alarm_uris() {
                    if crate::resources::refresh_alarms(&ctx, name).await
                        && send(&peer, name.clone()).await.is_err()
                    {
                        return;
                    }
                }
            }
        }
    }
}

async fn send(peer: &Peer<RoleServer>, uri: String) -> Result<(), rmcp::service::ServiceError> {
    peer.notify_resource_updated(ResourceUpdatedNotificationParam::new(uri))
        .await
}
