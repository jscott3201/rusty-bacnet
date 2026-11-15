//! Resource URIs (design §6). Every name is built from identifiers: a
//! device instance, an object type and instance, a property. A device-list
//! page is named by the row it follows: an instance at an address.

use bacnet_ops::DeviceAddress;
use bacnet_ops::label::kebab;
use bacnet_ops::octets::hex;
use bacnet_ops::parse::{parse_object_specifier, parse_property};
use bacnet_ops::watch::{Point, Watched};
use bacnet_types::MacAddr;
use bacnet_types::constructed::BACnetAddress;
use bacnet_types::primitives::ObjectIdentifier;

use crate::value::object_text;

pub(crate) const DEVICES: &str = "bacnet://devices";
pub(crate) const ALARMS: &str = "bacnet://alarms";
pub(crate) const SESSION: &str = "bacnet://session";
pub(crate) const LEDGER: &str = "bacnet://ledger";

/// Where a device-list page starts after: one row of the discovery table,
/// named by its instance and its address (network, then the station's MAC
/// there). Rows are listed in this order, and no two share it, so a page
/// boundary falls between rows even when several addresses claim one
/// instance.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct DeviceCursor {
    pub(crate) instance: u32,
    pub(crate) network: u16,
    pub(crate) mac: MacAddr,
}

impl DeviceCursor {
    /// `address` is the row's address, as the discovery table keys it.
    pub(crate) fn new(instance: u32, address: BACnetAddress) -> Self {
        Self {
            instance,
            network: address.network_number,
            mac: address.mac_address,
        }
    }

    /// `{instance}/{network}/{mac as lowercase hex}`.
    fn path(&self) -> String {
        format!("{}/{}/{}", self.instance, self.network, hex(&self.mac))
    }

    fn parse(text: &str) -> Result<Self, String> {
        let refusal = || {
            "a device-list cursor is {instance}/{network}/{mac in hex}: an instance 0 to \
             4194303, a network 0 to 65535, a MAC of at most 18 bytes"
                .to_string()
        };
        let [instance_text, network, mac] = text.split('/').collect::<Vec<_>>()[..] else {
            return Err(refusal());
        };
        let instance = instance(instance_text)?;
        let network = (!network.is_empty() && network.bytes().all(|b| b.is_ascii_digit()))
            .then(|| network.parse::<u16>().ok())
            .flatten()
            .ok_or_else(refusal)?;
        Ok(Self {
            instance,
            network,
            mac: unhex(mac).ok_or_else(refusal)?,
        })
    }
}

/// `text` as bytes, from pairs of hex digits (either case).
fn unhex(text: &str) -> Option<MacAddr> {
    if !text.len().is_multiple_of(2)
        || text.len() > 2 * BACnetAddress::MAX_MAC_LEN
        || !text.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(&text[at..at + 2], 16).ok())
        .collect()
}

/// A resource the server offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Uri {
    /// The discovery table's first page.
    Devices,
    /// The discovery table's page of devices past this row.
    DevicesAfter(DeviceCursor),
    /// One device's summary.
    Device(u32),
    /// An object's point facts, or one property.
    Point(PointUri),
    /// Active and unacknowledged alarms of every device in reach.
    Alarms,
    /// One device's active and unacknowledged alarms.
    DeviceAlarms(u32),
    /// The session: level, limits, the watch footprint.
    Session,
    /// This principal's ledger entries in effect.
    Ledger,
}

/// A point as a resource names it: by its device's instance, which the one
/// address claiming it in the discovery table stands for when it is read
/// or watched ([`at`](Self::at)).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct PointUri {
    /// The device instance.
    pub(crate) device: u32,
    /// The object.
    pub(crate) object: ObjectIdentifier,
    /// The object or one property.
    pub(crate) watched: Watched,
}

impl PointUri {
    /// The point of the device at `address`, as the watch hub keys it.
    pub(crate) fn at(self, address: DeviceAddress) -> Point {
        Point {
            device: address,
            object: self.object,
            watched: self.watched,
        }
    }
}

/// Parse `uri`, accepting `[` `]` and `:` percent-encoded.
pub(crate) fn parse(uri: &str) -> Result<Uri, String> {
    let rest = uri.strip_prefix("bacnet://").ok_or("not a bacnet:// URI")?;
    match rest {
        "devices" => return Ok(Uri::Devices),
        "alarms" => return Ok(Uri::Alarms),
        "session" => return Ok(Uri::Session),
        "ledger" => return Ok(Uri::Ledger),
        _ => {}
    }
    if let Some(after) = rest.strip_prefix("devices/after/") {
        return DeviceCursor::parse(after).map(Uri::DevicesAfter);
    }
    let rest = rest.strip_prefix("device/").ok_or("no such resource")?;
    let parts: Vec<&str> = rest.split('/').collect();
    let device = instance(parts[0])?;
    match parts[1..] {
        [] => Ok(Uri::Device(device)),
        ["alarms"] => Ok(Uri::DeviceAlarms(device)),
        ["object", object] => Ok(Uri::Point(PointUri {
            device,
            object: object_of(object)?,
            watched: Watched::Object,
        })),
        ["object", object, "property", property] => {
            let (property, index) = parse_property(&decode(property)?)?;
            Ok(Uri::Point(PointUri {
                device,
                object: object_of(object)?,
                watched: Watched::Property { property, index },
            }))
        }
        _ => Err("no such resource".into()),
    }
}

/// A device instance: digits only, 0 to 4194303.
fn instance(text: &str) -> Result<u32, String> {
    text.parse::<u32>()
        .ok()
        .filter(|n| {
            text.bytes().all(|b| b.is_ascii_digit()) && *n <= ObjectIdentifier::MAX_INSTANCE
        })
        .ok_or_else(|| "a device instance is 0 to 4194303".into())
}

fn object_of(text: &str) -> Result<ObjectIdentifier, String> {
    let (object_type, instance) = parse_object_specifier(&decode(text)?)?;
    ObjectIdentifier::new(object_type, instance).map_err(|e| e.to_string())
}

/// `text` with `%5B`, `%5D` and `%3A` decoded; any other escape refused.
fn decode(text: &str) -> Result<String, String> {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('%') {
        out.push_str(&rest[..at]);
        let code = rest.get(at + 1..at + 3).ok_or("a bad escape")?;
        out.push(match code.to_ascii_uppercase().as_str() {
            "5B" => '[',
            "5D" => ']',
            "3A" => ':',
            _ => return Err("only [, ] and : may be escaped".into()),
        });
        rest = &rest[at + 3..];
    }
    out.push_str(rest);
    Ok(out)
}

/// The URI of the device-list page past `cursor`'s row.
pub(crate) fn devices_after(cursor: &DeviceCursor) -> String {
    format!("{DEVICES}/after/{}", cursor.path())
}

/// A point's canonical URI.
pub(crate) fn of_point(point: &PointUri) -> String {
    let object = object_text(point.object.object_type(), point.object.instance_number());
    let base = format!("bacnet://device/{}/object/{object}", point.device);
    match point.watched {
        Watched::Object => base,
        Watched::Property { property, index } => match index {
            None => format!("{base}/property/{}", kebab(property)),
            Some(index) => format!("{base}/property/{}[{index}]", kebab(property)),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_types::enums::{ObjectType, PropertyIdentifier};

    #[test]
    fn uris_parse_to_identifiers_and_back() {
        assert_eq!(parse("bacnet://devices"), Ok(Uri::Devices));
        assert_eq!(parse("bacnet://device/1201"), Ok(Uri::Device(1201)));
        assert_eq!(
            parse("bacnet://device/1201/alarms"),
            Ok(Uri::DeviceAlarms(1201))
        );
        let ai = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
        let Ok(Uri::Point(point)) = parse("bacnet://device/1201/object/ai:1") else {
            panic!("a point");
        };
        assert_eq!(point.object, ai);
        assert_eq!(
            of_point(&point),
            "bacnet://device/1201/object/analog-input:1"
        );
        // The coming BACnet RDF encoding's `type,instance` names the same
        // point; the canonical URI keeps the colon.
        assert_eq!(
            parse("bacnet://device/1201/object/analog-input,1"),
            Ok(Uri::Point(point))
        );
        let Ok(Uri::Point(element)) =
            parse("bacnet://device/7/object/analog-output:2/property/priority-array%5B8%5D")
        else {
            panic!("a property");
        };
        assert_eq!(
            element.watched,
            Watched::Property {
                property: PropertyIdentifier::PRIORITY_ARRAY,
                index: Some(8)
            }
        );
        assert_eq!(
            of_point(&element),
            "bacnet://device/7/object/analog-output:2/property/priority-array[8]"
        );
        for bad in [
            "http://devices",
            "bacnet://device/4194304",
            "bacnet://device/+1",
            "bacnet://device/1/object",
            "bacnet://device/1/object/ai:1/property",
            "bacnet://device/1/object/ai:1%2F",
            "bacnet://device/1/thing",
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }

    /// A device-list cursor is an instance, a network and a MAC in hex: a
    /// row, so the claimants of one instance are told apart (#2589).
    #[test]
    fn device_list_cursors_parse_as_rows() {
        let cursor = |instance, network, mac: &[u8]| {
            Ok(Uri::DevicesAfter(DeviceCursor {
                instance,
                network,
                mac: MacAddr::from_slice(mac),
            }))
        };
        assert_eq!(
            parse("bacnet://devices/after/1201/0/0a000002bac0"),
            cursor(1201, 0, &[10, 0, 0, 2, 0xBA, 0xC0])
        );
        assert_eq!(
            parse("bacnet://devices/after/4194303/65535/0A"),
            cursor(4_194_303, 65535, &[10])
        );
        assert_eq!(parse("bacnet://devices/after/0/0/"), cursor(0, 0, &[]));
        let row = DeviceCursor::new(
            7,
            BACnetAddress {
                network_number: 2001,
                mac_address: MacAddr::from_slice(&[0x0c, 0xff]),
            },
        );
        assert_eq!(
            parse(&devices_after(&row)),
            Ok(Uri::DevicesAfter(row.clone())),
            "the URI a page names parses back"
        );
        assert_eq!(devices_after(&row), "bacnet://devices/after/7/2001/0cff");
        assert!(
            DeviceCursor::new(
                1000,
                BACnetAddress {
                    network_number: 0,
                    mac_address: MacAddr::from_slice(&[10, 0, 0, 100])
                }
            ) < DeviceCursor::new(
                1000,
                BACnetAddress {
                    network_number: 0,
                    mac_address: MacAddr::from_slice(&[10, 0, 0, 101])
                }
            ),
            "claimants of one instance are ordered by address"
        );
        for bad in [
            "bacnet://devices/after/4194304/0/00",
            "bacnet://devices/after/+1/0/00",
            "bacnet://devices/after/-1/0/00",
            "bacnet://devices/after/1a/0/00",
            "bacnet://devices/after//0/00",
            "bacnet://devices/after/1201",
            "bacnet://devices/after/1201/0",
            "bacnet://devices/after/1/0/00/",
            "bacnet://devices/after/1/65536/00",
            "bacnet://devices/after/1/+0/00",
            "bacnet://devices/after/1/0/0",
            "bacnet://devices/after/1/0/0g",
            "bacnet://devices/after/1/0/ 0",
            "bacnet://devices/after/1/0/0a0b0c0d0e0f0a0b0c0d0e0f0a0b0c0d0e0f00",
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
        for bad in [
            "bacnet://devices/",
            "bacnet://devices/after",
            "bacnet://devices/1",
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }
}
