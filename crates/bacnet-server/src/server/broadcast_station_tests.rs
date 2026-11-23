//! The remote check every confirmed request the server starts takes (#2732):
//! a station on another network whose MAC there is a broadcast or group form
//! ([`BroadcastStation`]) gets no confirmed request, and the route an
//! unconfirmed one takes is left as it was. A Device binding marked one
//! station (#2738) lifts the forms that are a station on some network.

use super::*;
use crate::server::device_bindings::BindingFreshness;

/// One MAC of each form `RemoteBroadcast` names (bacnet-npdu's own tests
/// cover the forms octet by octet): the MS/TP broadcast, a LonTalk
/// broadcast and group, the VMAC no device has, and the six-octet limited
/// broadcast at the B/IP port and at another.
pub(in crate::server) const FORMS: [&[u8]; 6] = [
    &[0xFF],
    &[0x00, 0x05],
    &[0xFF, 0x07],
    &[0xFF, 0xFF, 0xFF],
    &[0xFF, 0xFF, 0xFF, 0xFF, 0xBA, 0xC0],
    &[0xFF; 6],
];

/// Forms a binding marked one station lifts (#2738): ARCNET's station 255
/// and two-octet stations starting `0x00` or `0xFF`, the ends of each range
/// included.
pub(in crate::server) const LIFTED: [&[u8]; 6] = [
    &[0xFF],
    &[0x00, 0x01],
    &[0x00, 0xFF],
    &[0xFF, 0x00],
    &[0xFF, 0xFE],
    &[0xFF, 0x07],
];

/// Addresses a binding marked one station reaches: each form the mark
/// lifts, and the one octet `0x00`, which the mark never refuses (it only
/// lifts), so an ARCNET network's broadcast there goes uncaught.
pub(in crate::server) const MARKED_STATIONS: [&[u8]; 7] = [
    &[0x00],
    &[0xFF],
    &[0x00, 0x01],
    &[0x00, 0xFF],
    &[0xFF, 0x00],
    &[0xFF, 0xFE],
    &[0xFF, 0x07],
];

/// Forms still refused for a binding marked one station: `0x0000`, `0xFFFF`,
/// and the three- and six-octet forms the mark doesn't touch.
pub(in crate::server) const MARKED_FORMS: [&[u8]; 5] = [
    &[0x00, 0x00],
    &[0xFF, 0xFF],
    &[0xFF, 0xFF, 0xFF],
    &[0xFF, 0xFF, 0xFF, 0xFF, 0xBA, 0xC0],
    &[0xFF; 6],
];

/// Stations of each length: an MS/TP station, a LonTalk subnet and node, a
/// VMAC, a B/IP subnet's directed broadcast (another network's to judge, so
/// taken as one station), and a seven-octet LonTalk Neuron ID.
const STATIONS: [&[u8]; 5] = [
    &[0x14],
    &[0x01, 0x00],
    &[0x7F, 0xFF, 0xFF],
    &[0xC0, 0xA8, 0x01, 0xFF, 0xBA, 0xC0],
    &[0xFF; 7],
];

const NETWORK: u16 = 5;
const ROUTER: [u8; 6] = [10, 0, 0, 9, 0xBA, 0xC0];

fn bound(mac: &[u8], one_station: bool) -> RecipientRoute {
    RecipientRoute::BoundRoutedUnicast {
        network: NETWORK,
        mac: MacAddr::from_slice(mac),
        router: MacAddr::from_slice(&ROUTER),
        freshness: BindingFreshness::Configured,
        one_station,
    }
}

/// An address recipient's route and an unmarked binding's: neither carries
/// the mark.
fn routes(mac: &[u8]) -> [RecipientRoute; 2] {
    [
        RecipientRoute::RemoteUnicast {
            network: NETWORK,
            mac: MacAddr::from_slice(mac),
        },
        bound(mac, false),
    ]
}

/// No link here takes anything for a group, so only the remote check can
/// refuse.
fn no_group(_: &[u8]) -> bool {
    false
}

#[test]
fn a_confirmed_route_to_a_remote_broadcast_form_is_refused_and_a_unicast_one_kept() {
    for form in FORMS {
        let station = BroadcastStation::of(NETWORK, form, false).expect("a broadcast form");
        for route in routes(form) {
            assert_eq!(
                route.clone().into_confirmed(no_group).err(),
                Some(ConfirmedRouteRefusal::RemoteBroadcast(station.clone())),
                "{route:?}"
            );
            let unicast = route.clone().into_unicast(no_group).expect("one next hop");
            assert_eq!(unicast.broadcast_station(), Some(station.clone()));
        }
    }
    for mac in STATIONS {
        assert_eq!(
            BroadcastStation::of(NETWORK, mac, false),
            None,
            "{mac:02X?}"
        );
        for route in routes(mac).into_iter().chain([bound(mac, true)]) {
            let confirmed = route.into_confirmed(no_group).expect("one station");
            assert_eq!(confirmed.broadcast_station(), None);
        }
    }
}

/// A binding marked one station takes a confirmed request at each form the
/// mark lifts, which an address recipient and an unmarked binding at the
/// same MAC can't, and stays refused at the forms it doesn't (#2738).
#[test]
fn a_binding_marked_one_station_lifts_exactly_its_forms() {
    for mac in LIFTED {
        assert_eq!(BroadcastStation::of(NETWORK, mac, true), None, "{mac:02X?}");
        let confirmed = bound(mac, true)
            .into_confirmed(no_group)
            .expect("a station");
        assert!(confirmed.one_station);
        assert_eq!(confirmed.broadcast_station(), None, "{mac:02X?}");
        for route in routes(mac) {
            assert!(
                matches!(
                    route.clone().into_confirmed(no_group),
                    Err(ConfirmedRouteRefusal::RemoteBroadcast(_))
                ),
                "{route:?}"
            );
        }
    }
    for mac in MARKED_FORMS {
        let station = BroadcastStation::of(NETWORK, mac, true).expect("refused when marked");
        assert_eq!(
            bound(mac, true).into_confirmed(no_group).err(),
            Some(ConfirmedRouteRefusal::RemoteBroadcast(station.clone())),
            "{mac:02X?}"
        );
        let unicast = bound(mac, true).into_unicast(no_group).expect("one hop");
        assert_eq!(unicast.broadcast_station(), Some(station));
    }
    // The mark only lifts: the one octet 0x00 is a station marked or not.
    for one_station in [false, true] {
        assert_eq!(BroadcastStation::of(NETWORK, &[0x00], one_station), None);
        let confirmed = bound(&[0x00], one_station).into_confirmed(no_group);
        assert_eq!(confirmed.expect("a station").broadcast_station(), None);
    }
    // The mark is dropped where the binding is localized: the link judges.
    let local = bound(&[0xFF], true).localize(Some(NETWORK), no_group, no_group);
    let confirmed = local
        .into_confirmed(no_group)
        .expect("the link says station");
    assert!(!confirmed.one_station);
}

/// A route on this network has no remote station to judge: its MAC is the
/// link's to call a group, which `into_unicast` asks.
#[test]
fn a_local_route_takes_only_the_link_group_check() {
    for mac in FORMS {
        let route = RecipientRoute::LocalUnicast(MacAddr::from_slice(mac));
        let confirmed = route
            .into_confirmed(no_group)
            .expect("the link says station");
        assert_eq!(confirmed.broadcast_station(), None);
    }
}

#[test]
fn the_log_text_names_the_address_what_it_is_and_the_mark() {
    let text = |mac: &[u8], one_station| {
        BroadcastStation::of(NETWORK, mac, one_station)
            .unwrap()
            .to_string()
    };
    assert_eq!(
        text(&[0xFF], false),
        "5:0xFF, station 255, the broadcast of an MS/TP network (a station on ARCNET); \
         only a routed Device binding marked one station names a station at such an \
         address: a recipient's or a subscriber's address can't be marked"
    );
    assert!(text(&[0xFF, 0x07], false).starts_with("5:0xFF07, LonTalk group 7, a multicast; only"));
    assert_eq!(
        text(&[0x00, 0x00], true),
        "5:0x0000, a LonTalk broadcast to the whole domain, even marked one station"
    );
    let six = text(&[0xFF, 0xFF, 0xFF, 0xFF, 0xBA, 0xC0], false);
    assert!(
        six.starts_with(
            "5:0xFFFFFFFFBAC0, a broadcast on every six-octet link: 255.255.255.255:47808"
        ),
        "{six}"
    );
    assert!(!six.contains("marked"), "{six}");
}
