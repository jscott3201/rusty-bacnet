//! A device named by its instance, placed by a front end that holds a
//! low-level client and no [`Client`](crate::Client) (#2824): the CLI's
//! one-shot commands, the TUI's command bar and the write desk's revert of
//! an entry kept by instance. The rule is a handle's first placement's,
//! with nothing remembered but rows registered by hand: unless such a row
//! claims the instance, one Who-Is for it goes out first (when the caller
//! may send one) and its answers are given the settle time, then every
//! claim is asked which device answers there, all at once, and the answers
//! decide ([`placed`]).

use std::time::Duration;

use bacnet_client::client::{BACnetClient, DeviceEventKind};
use bacnet_client::discovery::DiscoveredDevice;
use bacnet_services::who_is::DeviceInstanceRange;
use bacnet_transport::port::TransportPort;
use tokio::sync::broadcast::error::RecvError;

use super::DeviceAddress;
use super::confirm::{Answer, every, identify, over_cap, placed};
use super::resolve::SETTLE;
use crate::client::Scope;
use crate::error::{OpsError, said};
use crate::limits::RequestLimits;

/// Where [`place_instance`] sends its one Who-Is, and how long it waits for
/// a first answer when no address claims the instance.
#[derive(Debug, Clone)]
pub struct Look {
    /// Where the Who-Is goes.
    pub scope: Scope,
    /// The wait for a first answer when no address claims the instance.
    pub wait: Duration,
}

/// What [`place_instance`] found.
#[derive(Debug)]
pub struct Placement {
    /// Each claim, with what its device answered, or was known to say (a
    /// row registered by hand); empty when none was asked.
    pub asked: Vec<(DeviceAddress, Answer)>,
    /// Where the device is, or why it isn't placed.
    pub placed: Result<DeviceAddress, OpsError>,
}

impl Placement {
    /// Why device `name` is reached nowhere, each address as `show` writes
    /// it: each address asked and what answered there, or the error's own
    /// text. `None` once placed.
    pub fn refusal(&self, name: &str, show: &dyn Fn(&DeviceAddress) -> String) -> Option<String> {
        let error = self.placed.as_ref().err()?;
        Some(match (self.asked.as_slice(), error) {
            ([(at, answer)], OpsError::NotConfirmed { .. }) => format!(
                "device {name} is claimed at {}, but asked which device answers there, {}: \
                 nothing was sent to it",
                show(at),
                said(Some(*answer))
            ),
            ([_, _, ..], _) => {
                let each: Vec<String> = self
                    .asked
                    .iter()
                    .map(|(at, answer)| format!("{} ({})", show(at), said(Some(*answer))))
                    .collect();
                format!(
                    "device {name} is claimed at {}, and not one of them alone answered as it; \
                     name one by its address",
                    each.join(" and ")
                )
            }
            (_, OpsError::Ambiguous { candidates, .. }) => {
                let each: Vec<String> = candidates.iter().map(show).collect();
                format!(
                    "device {name} is claimed at {} addresses ({}), too many to ask; name one \
                     by its address",
                    candidates.len(),
                    each.join(", ")
                )
            }
            (_, error) => error.to_string(),
        })
    }
}

/// Place device `instance` (on `network` when the target names one) from
/// `client`'s discovery table (#2824): unless a row registered by hand
/// claims it, one Who-Is for it to `look`'s scope first (none without
/// `look`), its answers given half a second, or, with no claim at all, `look`'s
/// wait for a first answer; then each claim asked which device answers
/// there ([`identify`], all at once, each at its gate under `limits`), and
/// [`placed`] decides. More claims needing a question than
/// [`MAX_ASKED`](super::MAX_ASKED) place only a single one registered by
/// hand, else are ambiguous, asked nothing.
pub async fn place_instance<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    limits: &RequestLimits,
    instance: u32,
    network: Option<u16>,
    look: Option<Look>,
) -> Placement {
    let failed = |error: OpsError| Placement {
        asked: Vec::new(),
        placed: Err(error),
    };
    let own = client.network_number();
    let scoped = network.map(|n| if Some(n) == own { 0 } else { n });
    let mut claims = claims(client, instance, scoped).await;
    // Where a Who-Is went, when one did.
    let mut asked = None;
    if !claims.iter().any(|(_, registered)| *registered)
        && let Some(look) = &look
    {
        let range = match DeviceInstanceRange::single(instance) {
            Ok(range) => range,
            Err(error) => return failed(error.into()),
        };
        let mut events = client.device_events();
        // A Who-Is the link can't send is one asked all the same: the
        // claims the table holds are asked.
        let sent = crate::raw::who_is(client, &look.scope, Some(range)).await;
        match &sent {
            Ok(()) => asked = Some(look.scope.clone()),
            Err(error) => tracing::warn!("looking for device {instance}: {error}"),
        }
        if sent.is_ok() && claims.is_empty() {
            let wanted = |row: &DiscoveredDevice| {
                row.object_identifier.instance_number() == instance
                    && DeviceAddress::of(row)
                        .is_some_and(|at| scoped.is_none_or(|n| at.localized(own).network() == n))
            };
            let mut deadline = tokio::time::Instant::now() + look.wait;
            let mut heard = false;
            loop {
                match tokio::time::timeout_at(deadline, events.recv()).await {
                    Err(_) | Ok(Err(RecvError::Closed)) => break,
                    Ok(Ok(event))
                        if event.kind != DeviceEventKind::Lost && wanted(&event.device) =>
                    {
                        if !heard {
                            heard = true;
                            deadline = deadline.min(tokio::time::Instant::now() + SETTLE);
                        }
                    }
                    Ok(Ok(_) | Err(RecvError::Lagged(_))) => {}
                }
            }
        } else if sent.is_ok() {
            tokio::time::sleep(SETTLE).await;
        }
        claims = self::claims(client, instance, scoped).await;
    }
    if claims.is_empty() {
        return failed(OpsError::UnknownDevice {
            target: match network {
                Some(network) => format!("{network}:{instance}"),
                None => instance.to_string(),
            },
            asked,
        });
    }
    let before: Vec<(DeviceAddress, Option<Answer>)> = claims
        .iter()
        .map(|(at, registered)| (at.clone(), registered.then_some(Answer::Is(instance))))
        .collect();
    if let Some(decided) = over_cap(instance, &before) {
        return Placement {
            asked: Vec::new(),
            placed: decided,
        };
    }
    let asks = before.iter().map(|(at, known)| async move {
        match known {
            Some(answer) => *answer,
            None => identify(client, limits, at, instance).await,
        }
    });
    let answers = every(asks).await;
    let asked: Vec<(DeviceAddress, Answer)> =
        before.into_iter().map(|(at, _)| at).zip(answers).collect();
    Placement {
        placed: placed(instance, &asked),
        asked,
    }
}

/// The addresses whose rows claim `instance`, on `network` (as the table
/// numbers it) when one is named, each with whether its row was
/// registered by hand, never announced; ordered, one per address.
async fn claims<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    instance: u32,
    network: Option<u16>,
) -> Vec<(DeviceAddress, bool)> {
    let own = client.network_number();
    let mut claims: Vec<(DeviceAddress, bool)> = client
        .discovered_devices()
        .await
        .iter()
        .filter(|row| row.object_identifier.instance_number() == instance)
        .filter_map(|row| {
            let at = DeviceAddress::of(row)?.localized(own);
            Some((at, row.last_announced.is_none()))
        })
        .filter(|(at, _)| network.is_none_or(|n| at.network() == n))
        .collect();
    claims.sort();
    claims.dedup_by(|a, b| a.0 == b.0);
    claims
}
