//! The remote check every confirmed request the server starts takes (#2732):
//! a station on another network whose MAC there is a broadcast or group form
//! ([`BroadcastStation`]) gets no confirmed request, and the route an
//! unconfirmed one takes is left as it was.

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

fn routes(mac: &[u8]) -> [RecipientRoute; 2] {
    [
        RecipientRoute::RemoteUnicast {
            network: NETWORK,
            mac: MacAddr::from_slice(mac),
        },
        RecipientRoute::BoundRoutedUnicast {
            network: NETWORK,
            mac: MacAddr::from_slice(mac),
            router: MacAddr::from_slice(&ROUTER),
            freshness: BindingFreshness::Configured,
        },
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
        let station = BroadcastStation::of(NETWORK, form).expect("a broadcast form");
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
        assert_eq!(BroadcastStation::of(NETWORK, mac), None, "{mac:02X?}");
        for route in routes(mac) {
            let confirmed = route.into_confirmed(no_group).expect("one station");
            assert_eq!(confirmed.broadcast_station(), None);
        }
    }
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
fn the_log_text_names_the_address_and_what_it_is() {
    let text = |mac: &[u8]| BroadcastStation::of(NETWORK, mac).unwrap().to_string();
    assert_eq!(
        text(&[0xFF]),
        "5:0xFF, station 255, the broadcast of an MS/TP network (ARCNET station 255 can't \
         be named here)"
    );
    assert_eq!(
        text(&[0xFF, 0x07]),
        "5:0xFF07, LonTalk group 7, a multicast"
    );
    let six = text(&[0xFF, 0xFF, 0xFF, 0xFF, 0xBA, 0xC0]);
    assert!(
        six.starts_with(
            "5:0xFFFFFFFFBAC0, a broadcast on every six-octet link: 255.255.255.255:47808"
        ),
        "{six}"
    );
}
