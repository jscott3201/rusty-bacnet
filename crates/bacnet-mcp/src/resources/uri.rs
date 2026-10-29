//! Resource URIs (design §6). Every name is built from identifiers: a
//! device instance, an object type and instance, a property.

use bacnet_ops::label::kebab;
use bacnet_ops::parse::{parse_object_specifier, parse_property};
use bacnet_ops::watch::{Point, Watched};
use bacnet_types::primitives::ObjectIdentifier;

use crate::value::object_text;

pub(crate) const DEVICES: &str = "bacnet://devices";
pub(crate) const ALARMS: &str = "bacnet://alarms";
pub(crate) const SESSION: &str = "bacnet://session";
pub(crate) const LEDGER: &str = "bacnet://ledger";

/// A resource the server offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Uri {
    /// The discovery table's first page.
    Devices,
    /// The discovery table's page of devices above this instance.
    DevicesAfter(u32),
    /// One device's summary.
    Device(u32),
    /// An object's point facts, or one property.
    Point(Point),
    /// Active and unacknowledged alarms of every device in reach.
    Alarms,
    /// One device's active and unacknowledged alarms.
    DeviceAlarms(u32),
    /// The session: level, limits, the watch footprint.
    Session,
    /// This principal's ledger entries in effect.
    Ledger,
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
        return instance(after).map(Uri::DevicesAfter);
    }
    let rest = rest.strip_prefix("device/").ok_or("no such resource")?;
    let parts: Vec<&str> = rest.split('/').collect();
    let device = instance(parts[0])?;
    match parts[1..] {
        [] => Ok(Uri::Device(device)),
        ["alarms"] => Ok(Uri::DeviceAlarms(device)),
        ["object", object] => Ok(Uri::Point(Point {
            device,
            object: object_of(object)?,
            watched: Watched::Object,
        })),
        ["object", object, "property", property] => {
            let (property, index) = parse_property(&decode(property)?)?;
            Ok(Uri::Point(Point {
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

/// The URI of the device-list page past `instance`.
pub(crate) fn devices_after(instance: u32) -> String {
    format!("{DEVICES}/after/{instance}")
}

/// A point's canonical URI.
pub(crate) fn of_point(point: &Point) -> String {
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

    /// A device-list cursor is an instance, validated as a device's is.
    #[test]
    fn device_list_cursors_parse_as_instances() {
        assert_eq!(
            parse("bacnet://devices/after/1201"),
            Ok(Uri::DevicesAfter(1201))
        );
        assert_eq!(
            parse("bacnet://devices/after/4194303"),
            Ok(Uri::DevicesAfter(4_194_303))
        );
        assert_eq!(
            parse(&devices_after(0)),
            Ok(Uri::DevicesAfter(0)),
            "the URI a page names parses back"
        );
        let refusal = Err("a device instance is 0 to 4194303".to_string());
        for bad in [
            "bacnet://devices/after/4194304",
            "bacnet://devices/after/+1",
            "bacnet://devices/after/-1",
            "bacnet://devices/after/1a",
            "bacnet://devices/after/",
            "bacnet://devices/after/1/",
            "bacnet://devices/after/ 1",
        ] {
            assert_eq!(parse(bad), refusal, "{bad}");
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
