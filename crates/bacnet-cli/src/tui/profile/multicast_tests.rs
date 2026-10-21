//! A B/IP profile's multicast group (#941): validated, saved and handed to
//! the transport.

use super::store::Store;
use super::*;

fn member() -> Profile {
    let mut p = Profile::default();
    p.bip_mut().interface = Ipv4Addr::LOCALHOST;
    p.bip_mut().multicast = Some(Ipv4Addr::new(239, 255, 66, 1));
    p.bip_mut().multicast_ttl = 2;
    p
}

#[test]
fn a_profiles_group_reaches_the_transport_and_survives_a_save() {
    let profile = member();
    let multicast = profile.transport_args().unwrap().multicast.unwrap();
    assert_eq!(
        (multicast.group(), multicast.ttl()),
        (Ipv4Addr::new(239, 255, 66, 1), 2)
    );
    let root = tempfile::tempdir().unwrap();
    let store = Store::new(root.path().join("config"));
    store.save("group", &profile, true).unwrap();
    assert_eq!(store.load("group").unwrap(), profile);
    // Without a group, nothing is joined, and the TTL defaults to 1.
    let plain = Profile::default();
    assert_eq!(plain.bip().multicast_ttl, 1);
    assert!(plain.transport_args().unwrap().multicast.is_none());
}

#[test]
fn a_profiles_group_is_refused_where_it_cant_apply() {
    let refused = |edit: fn(&mut Bip), why: &str| {
        let mut p = member();
        edit(p.bip_mut());
        let error = p.validate().unwrap_err();
        assert!(error.contains(why), "{error}");
    };
    refused(
        |b| b.interface = Ipv4Addr::UNSPECIFIED,
        "explicit interface",
    );
    refused(
        |b| b.multicast = Some(Ipv4Addr::new(10, 0, 0, 1)),
        "class D",
    );
    refused(|b| b.multicast_ttl = 0, "TTL");
    refused(
        |b| b.bbmd = Some("127.0.0.2:47809".parse().unwrap()),
        "BBMD",
    );
    refused(|b| b.port = 0, "multicast_port");
    refused(|b| b.multicast_port = Some(0), "nonzero");
    refused(
        |b| {
            b.multicast = None;
            b.multicast_port = Some(47809);
        },
        "needs a multicast group",
    );
}

#[test]
fn a_profiles_group_port_reaches_the_transport() {
    let mut profile = member();
    profile.bip_mut().port = 0;
    profile.bip_mut().multicast_port = Some(47809);
    let multicast = profile.transport_args().unwrap().multicast.unwrap();
    assert_eq!(multicast.port(), Some(47809));
}
