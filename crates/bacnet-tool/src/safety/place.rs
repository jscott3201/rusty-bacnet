//! Where a change goes (#2560): the turn at its address, taken only while
//! the device there is still the one the change was planned and confirmed
//! for, and the address a revert goes to, found again after a restart.
//!
//! The discovery table's row at the address says which device is there.
//! A local address with no row (aged out, or after a restart) is asked:
//! one ReadProperty of the Device object's Object_Identifier by the
//! wildcard instance, once per address in one call of the desk. A routed
//! address with no row has no route; a caller that may look
//! ([`Caller::may_look`]) sends a Who-Is to its network first.
//!
//! [`Caller::may_look`]: super::desk::Caller::may_look

use std::sync::atomic::AtomicBool;
use std::time::Duration;

use bacnet_client::client::{BACnetClient, Broadcast};
use bacnet_client::discovery::DiscoveredDevice;
use bacnet_services::who_is::DeviceInstanceRange;
use bacnet_transport::port::TransportPort;
use bacnet_types::error::Error;

use super::ledger::Entry;
use bacnet_ops::DeviceAddress;
use bacnet_ops::limits::{RequestLimits, Turn};
use bacnet_ops::raw::{self, Halt, Reader};
use bacnet_ops::target::AddressStyle;

/// How long a revert waits for a device it had to look for.
const FIND_FOR: Duration = Duration::from_secs(3);

/// Why a change was not sent to the address it was planned at (#2560).
/// Nothing went to the device for any of them. Addresses print as the
/// client's link writes them (an SC VMAC as hex octets).
#[derive(Debug)]
pub enum Unsent {
    /// The client has no way there: a routed address the discovery table
    /// has no row for (so no router), or a device a Who-Is found no row
    /// for.
    Client(Error),
    /// Another device is at the address now: the discovery table's row
    /// there, or the device's own answer to a read of its Device object's
    /// Object_Identifier, names another instance than the one the change
    /// was confirmed for.
    OtherDevice {
        /// Where the change was to go.
        address: DeviceAddress,
        /// The instance it was planned and confirmed for.
        expected: u32,
        /// The instance at the address now.
        found: u32,
        /// How the link writes addresses.
        style: AddressStyle,
    },
    /// A local address with no row whose device didn't say which it is:
    /// the read of its Object_Identifier got no answer, or no usable one.
    Unidentified {
        /// Where the change was to go.
        address: DeviceAddress,
        /// The instance it was planned and confirmed for.
        expected: u32,
        /// What went wrong with the read.
        why: String,
        /// How the link writes addresses.
        style: AddressStyle,
    },
    /// A revert of an entry kept without an address (written before
    /// #2560), whose instance two or more addresses claim now, so which
    /// device it changed is unknown.
    Contested {
        /// The entry's device instance.
        instance: u32,
        /// Every address claiming it, ordered.
        claims: Vec<DeviceAddress>,
        /// How the link writes addresses.
        style: AddressStyle,
    },
    /// A revert whose caller sends no Who-Is ([`Caller::may_look`]) of a
    /// device the discovery table can't place without one: a routed
    /// address with no row, or an entry without an address whose instance
    /// no row claims.
    ///
    /// [`Caller::may_look`]: super::desk::Caller::may_look
    NotLooking {
        /// The device as the entry names it: its address, or its instance.
        device: String,
    },
}

impl std::fmt::Display for Unsent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Client(error) => write!(f, "{error}"),
            Self::OtherDevice {
                address,
                expected,
                found,
                style,
            } => write!(
                f,
                "{} is device {found} now, not device {expected}: nothing was sent there",
                address.styled(*style)
            ),
            Self::Unidentified {
                address,
                expected,
                why,
                style,
            } => write!(
                f,
                "asked {} which device it is, to make sure it is device {expected}: {why}; \
                 nothing was sent there: discover the device (a Who-Is), then try again",
                address.styled(*style)
            ),
            Self::Contested {
                instance,
                claims,
                style,
            } => {
                let claims: Vec<String> = claims.iter().map(|at| at.styled(*style)).collect();
                write!(
                    f,
                    "device {instance} is claimed by {}, and this entry predates the ledger \
                     keeping addresses, so which one it changed is unknown: revert it again \
                     once one address alone claims {instance}, or write its old value back by \
                     hand at the right one",
                    claims.join(" and ")
                )
            }
            Self::NotLooking { device } => write!(
                f,
                "{device} is not in the discovery table, and this revert sends no Who-Is to \
                 find it: discover the device, then revert again"
            ),
        }
    }
}

impl std::error::Error for Unsent {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Client(error) => Some(error),
            _ => None,
        }
    }
}

impl From<Error> for Unsent {
    fn from(error: Error) -> Self {
        Self::Client(error)
    }
}

/// What the devices at addresses with no row said they are, in one call
/// of the desk: a revert of several entries at one such address asks once.
#[derive(Debug, Default)]
pub(super) struct Asked(Vec<(DeviceAddress, Result<u32, String>)>);

impl Asked {
    /// Make sure the device at `address`, a local address with no row, is
    /// `instance`: asked once per address, then remembered for this call.
    async fn check<T: TransportPort + 'static>(
        &mut self,
        client: &BACnetClient<T>,
        rate: &RequestLimits,
        (address, instance): (&DeviceAddress, u32),
        style: AddressStyle,
    ) -> Result<(), Unsent> {
        let key = address.clone().localized(client.network_number());
        let answer = match self.0.iter().find(|(at, _)| *at == key) {
            Some((_, answer)) => answer.clone(),
            None => {
                let answer = ask(client, rate, address).await;
                self.0.push((key, answer.clone()));
                answer
            }
        };
        match answer {
            Ok(found) if found == instance => Ok(()),
            Ok(found) => Err(Unsent::OtherDevice {
                address: address.clone(),
                expected: instance,
                found,
                style,
            }),
            Err(why) => Err(Unsent::Unidentified {
                address: address.clone(),
                expected: instance,
                why,
                style,
            }),
        }
    }
}

/// Ask the device at `address` which it is, by the wildcard instance (the
/// read the device handles use, #2560): one unicast, no Who-Is.
async fn ask<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    rate: &RequestLimits,
    address: &DeviceAddress,
) -> Result<u32, String> {
    let never = AtomicBool::new(false);
    let mut reader = Reader {
        client,
        device: address.clone(),
        instance: None,
        rate,
        cancel: &never,
        rpm: true,
    };
    reader.instance().await.map_err(|halt| match halt {
        Halt::Device(error) => error.to_string(),
        Halt::Cancelled => "the read was cancelled".into(),
    })
}

/// The turn at the gate of `address`, the one its reads take too (#2560),
/// for the requests of one change to the device `instance` sent there
/// after it: plan, gate and send share the address, so no write is gated
/// at one device and sent to another.
///
/// The discovery table's row at the address is checked before the turn is
/// taken and again once it is held: one claiming another instance (the
/// address re-announced as another device) refuses the change. A local
/// address with no row (aged out, or after a restart) is asked which
/// device it is first, with one ReadProperty; while a row exists, nothing
/// is sent to check. A routed address with no row has no route.
///
/// # Errors
///
/// [`Unsent::OtherDevice`] when the row at the address, or the device's
/// answer, names another instance; [`Unsent::Unidentified`] when a local
/// address with no row doesn't answer that; and [`Unsent::Client`] with
/// [`Error::UnknownDevice`] for a routed address the table has no row for.
/// The caller reports the change as not sent.
pub async fn turn<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    rate: &RequestLimits,
    address: &DeviceAddress,
    instance: u32,
) -> Result<Turn, Unsent> {
    turn_asking(client, rate, address, instance, &mut Asked::default()).await
}

/// [`turn`], asking each address with no row at most once in `asked`.
pub(super) async fn turn_asking<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    rate: &RequestLimits,
    address: &DeviceAddress,
    instance: u32,
    asked: &mut Asked,
) -> Result<Turn, Unsent> {
    let style = AddressStyle::of(client.transport());
    // Before the wait, so a refusal takes no slot. The identity read takes
    // its own turn, so it comes before this one is held.
    let local = address
        .clone()
        .localized(client.network_number())
        .is_local();
    if !claimed(client, address, instance, style).await? && local {
        asked
            .check(client, rate, (address, instance), style)
            .await?;
    }
    let held = raw::turn(client, rate, address).await?;
    // Again once the turn is held: the row may have changed while it waited.
    claimed(client, address, instance, style).await?;
    Ok(held)
}

/// Whether the discovery table has a row at `address` claiming `instance`:
/// false for no row, and a refusal for a row claiming another instance.
async fn claimed<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    address: &DeviceAddress,
    instance: u32,
    style: AddressStyle,
) -> Result<bool, Unsent> {
    match address.row(client).await {
        None => Ok(false),
        Some(row) if row.object_identifier.instance_number() == instance => Ok(true),
        Some(row) => Err(Unsent::OtherDevice {
            address: address.clone(),
            expected: instance,
            found: row.object_identifier.instance_number(),
            style,
        }),
    }
}

/// Where the revert of `entry` goes, settled before anything is sent there
/// (the identity read included), in its local form once the client knows
/// its own network's number. Sends nothing unless `look`.
///
/// An entry with an address goes there. A routed one with no row has no
/// route: with `look` (the caller may look, [`Caller::may_look`]) its
/// network is asked with a Who-Is for the instance first; without, it is
/// reported. An entry written before #2560 kept no address: its device is
/// the one address claiming its instance, after a Who-Is for it when none
/// does and `look`.
///
/// # Errors
///
/// [`Unsent::NotLooking`] for a device that can't be placed without a
/// Who-Is when `look` is false, [`Unsent::Contested`] for an entry without
/// an address whose instance two or more addresses claim, and
/// [`Unsent::Client`] when a Who-Is found nothing.
///
/// [`Caller::may_look`]: super::desk::Caller::may_look
pub async fn revert_address<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    entry: &Entry,
    look: bool,
) -> Result<DeviceAddress, Unsent> {
    let own = client.network_number();
    let style = AddressStyle::of(client.transport());
    let instance = entry.device;
    let Some(address) = &entry.address else {
        if look {
            look_for(client, instance).await;
        }
        return match DeviceAddress::claiming(client, instance).await {
            Ok(at) => Ok(at.localized(own)),
            Err(Error::AmbiguousDevice { candidates, .. }) => {
                // A routed row on this network not yet folded into the
                // local one (#2572) is no second claim.
                let mut claims: Vec<DeviceAddress> = candidates
                    .iter()
                    .filter_map(|c| DeviceAddress::new(c.network_number, &c.mac_address).ok())
                    .map(|at| at.localized(own))
                    .collect();
                claims.sort();
                claims.dedup();
                match <[DeviceAddress; 1]>::try_from(claims) {
                    Ok([at]) => Ok(at),
                    Err(claims) => Err(Unsent::Contested {
                        instance,
                        claims,
                        style,
                    }),
                }
            }
            Err(Error::UnknownDevice { .. }) if !look => Err(Unsent::NotLooking {
                device: format!("device {instance}"),
            }),
            Err(error) => Err(Unsent::Client(error)),
        };
    };
    let at = address.clone().localized(own);
    if !at.is_local() && at.row(client).await.is_none() {
        if !look {
            return Err(Unsent::NotLooking {
                device: at.styled(style),
            });
        }
        reach(client, &at, instance).await;
    }
    Ok(at)
}

/// Make sure the client has a route to `address` before a write to the
/// device `instance` there: a revert after a restart may come before any
/// Who-Is. A local address needs none, nor does one the discovery table
/// has a row for, so nothing is sent for them. A routed address without one
/// asks its network with a Who-Is for that one instance, and waits a
/// little for the device at that address to answer; another address
/// answering is never taken for it.
pub async fn reach<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    address: &DeviceAddress,
    instance: u32,
) {
    let local = address
        .clone()
        .localized(client.network_number())
        .is_local();
    let heard = Heard::At(address);
    if local || heard.holds(client).await {
        return;
    }
    let Ok(range) = DeviceInstanceRange::new(instance, instance) else {
        return;
    };
    if let Err(error) = client
        .who_is(Broadcast::Network(address.network()), Some(range))
        .await
    {
        tracing::warn!("looking for device {instance} at {address}: {error}");
        return;
    }
    if !heard.wait(client).await {
        tracing::warn!("device {instance} at {address} did not answer a Who-Is");
    }
}

/// For a ledger entry written before #2560, which kept no address: when no
/// row claims the device `instance`, a Who-Is for that instance, and a
/// little wait for an answer.
async fn look_for<T: TransportPort + 'static>(client: &BACnetClient<T>, instance: u32) {
    let heard = Heard::Claims(instance);
    if heard.holds(client).await {
        return;
    }
    let Ok(range) = DeviceInstanceRange::new(instance, instance) else {
        return;
    };
    match client.who_is(Broadcast::Global, Some(range)).await {
        Ok(()) => {
            if !heard.wait(client).await {
                tracing::warn!("device {instance} did not answer a Who-Is");
            }
        }
        Err(error) => tracing::warn!("looking for device {instance}: {error}"),
    }
}

/// What a Who-Is to find a device waits to hear of.
enum Heard<'a> {
    /// A row at this address.
    At(&'a DeviceAddress),
    /// A row claiming this instance.
    Claims(u32),
}

impl Heard<'_> {
    /// True when the discovery table holds it now.
    async fn holds<T: TransportPort + 'static>(&self, client: &BACnetClient<T>) -> bool {
        match self {
            Self::At(address) => address.row(client).await.is_some(),
            Self::Claims(instance) => client
                .discovered_devices()
                .await
                .iter()
                .any(|d: &DiscoveredDevice| d.object_identifier.instance_number() == *instance),
        }
    }

    /// Check every 100 ms for up to [`FIND_FOR`]; true once it holds.
    async fn wait<T: TransportPort + 'static>(&self, client: &BACnetClient<T>) -> bool {
        let step = Duration::from_millis(100);
        let mut waited = Duration::ZERO;
        while waited < FIND_FOR {
            tokio::time::sleep(step).await;
            waited += step;
            if self.holds(client).await {
                return true;
            }
        }
        false
    }
}
