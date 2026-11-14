//! Device instance range parsing for Who-Is.

use bacnet_services::who_is::DeviceInstanceRange;
use bacnet_types::error::Error;

/// Parse a discover range string like "1000-2000"; no string asks every
/// device. A Who-Is carries both limits or neither (Clause 16.10, #1483), so
/// a range with one side missing, such as "1000-", is refused by name rather
/// than sent as a request for every device, and so is a low limit above the
/// high one or a limit past the highest instance, 4194303.
pub(crate) fn parse_discover_range(
    range: Option<&str>,
) -> Result<Option<DeviceInstanceRange>, Box<dyn std::error::Error>> {
    let Some(r) = range else {
        return Ok(None);
    };
    let Some((lo, hi)) = r.split_once('-') else {
        return Err(format!("invalid range format: '{r}', expected 'low-high'").into());
    };
    let missing = match (lo.is_empty(), hi.is_empty()) {
        (false, true) => Some("a low limit but no high limit"),
        (true, false) => Some("a high limit but no low limit"),
        _ => None,
    };
    if let Some(missing) = missing {
        return Err(format!(
            "invalid range '{r}': it has {missing}; give both, as 'low-high', or none"
        )
        .into());
    }
    let low = lo
        .parse::<u32>()
        .map_err(|_| format!("invalid range low: '{lo}'"))?;
    let high = hi
        .parse::<u32>()
        .map_err(|_| format!("invalid range high: '{hi}'"))?;
    if low > high {
        return Err(format!("invalid range: low ({low}) > high ({high})").into());
    }
    let range = DeviceInstanceRange::new(low, high).map_err(|error| match error {
        Error::OutOfRange(message) => format!("invalid range '{r}': {message}"),
        other => other.to_string(),
    })?;
    Ok(Some(range))
}

/// `discover --dnet` and `find --dnet`: a remote network from 1 to 65534.
/// Network 0 is this one and 65535 every one, so each is refused with the
/// option that says it (#2560, #2596).
pub(crate) fn parse_dnet(text: &str) -> Result<u16, String> {
    match text.trim().parse::<u16>() {
        Ok(0) => Err("network 0 is this network: leave --dnet out for a local broadcast".into()),
        Ok(u16::MAX) => Err("network 65535 is every network: use --global instead".into()),
        Ok(network) => Ok(network),
        Err(_) => Err(format!("'{text}' is not a network number from 1 to 65534")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `--dnet` takes 1 to 65534, and names the option for 0 and 65535.
    #[test]
    fn a_dnet_is_a_remote_network() {
        assert_eq!(parse_dnet("5"), Ok(5));
        assert_eq!(parse_dnet("65534"), Ok(65534));
        assert!(parse_dnet("0").unwrap_err().contains("leave --dnet out"));
        assert!(parse_dnet("65535").unwrap_err().contains("--global"));
        assert!(parse_dnet("70000").unwrap_err().contains("1 to 65534"));
    }

    #[test]
    fn parses_bounded_and_absent_ranges() {
        assert_eq!(
            parse_discover_range(Some("101-103")).unwrap(),
            Some(DeviceInstanceRange::new(101, 103).unwrap())
        );
        assert_eq!(parse_discover_range(None).unwrap(), None);
    }

    #[test]
    fn rejects_inverted_and_malformed_ranges() {
        let inverted = parse_discover_range(Some("5-1")).unwrap_err().to_string();
        assert!(inverted.contains("low (5) > high (1)"), "{inverted}");
        let malformed = parse_discover_range(Some("12")).unwrap_err().to_string();
        assert!(malformed.contains("expected 'low-high'"), "{malformed}");
    }

    /// The highest instance, 4194303, may be a limit; one past it may not.
    #[test]
    fn limits_stop_at_the_highest_instance() {
        assert_eq!(
            parse_discover_range(Some("0-4194303")).unwrap(),
            Some(DeviceInstanceRange::new(0, 4_194_303).unwrap())
        );
        let past = parse_discover_range(Some("0-4194304"))
            .unwrap_err()
            .to_string();
        assert!(
            past.contains("high limit 4194304 is above the highest instance, 4194303"),
            "{past}"
        );
    }

    /// One limit alone is refused by name, not sent as an unbounded Who-Is
    /// (#1483).
    #[test]
    fn rejects_a_range_with_one_limit() {
        for (text, named) in [
            ("1000-", "a low limit but no high limit"),
            ("-2000", "a high limit but no low limit"),
        ] {
            let error = parse_discover_range(Some(text)).unwrap_err().to_string();
            assert!(
                error.contains(named) && error.contains("give both"),
                "{text}: {error}"
            );
        }
    }
}
