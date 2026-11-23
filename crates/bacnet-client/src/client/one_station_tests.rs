//! The one-station mark (#2738) on a routed confirmed request: which remote
//! broadcast forms it lifts, in each station form, read from the shared
//! test transport's send log, and how a refusal names it.
use super::*;
use crate::discovery::DiscoveredDevice;
use bacnet_types::enums::{ObjectType, Segmentation};
use bacnet_types::primitives::ObjectIdentifier;

/// A discovery-table row for the station at `mac` on [`NETWORK`] behind
/// `router`, so [`Station::remote`] has a route.
fn routed_row(router: &[u8], mac: &[u8]) -> DiscoveredDevice {
    DiscoveredDevice {
        object_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 77).unwrap(),
        mac_address: MacAddr::from_slice(router),
        max_apdu_length: 1476,
        segmentation_supported: Segmentation::NO_SEGMENTATION,
        max_segments_accepted: None,
        vendor_id: 0,
        last_seen: std::time::Instant::now(),
        last_announced: None,
        source_network: Some(NETWORK),
        source_address: Some(MacAddr::from_slice(mac)),
    }
}

/// Each form by the routed, the table-routed and the local-broadcast form
/// alike: unmarked, the one octet `0xFF` and two octets starting `0x00` or
/// `0xFF` are refused; marked, those go but for `0x0000` and `0xFFFF`.
/// The mark only lifts, so the one octet `0x00` goes either way, and three
/// and six octets of a broadcast form stay refused either way.
#[tokio::test(start_paused = true)]
async fn the_one_station_mark_lifts_exactly_its_forms_in_every_form() {
    let (mut client, sent, _inbound) = client_on(
        DataLink::Mstp,
        &[1],
        no_groups(),
        ClientOptions::default(),
        0,
    )
    .await;
    let router = [9];
    // The MAC, then whether a confirmed request goes unmarked and marked.
    let cases: [(&[u8], bool, bool); 13] = [
        (&[0xFF], false, true),
        (&[0x00], true, true),
        (&[0x05], true, true),
        (&[0x00, 0x01], false, true),
        (&[0x00, 0xFF], false, true),
        (&[0xFF, 0x00], false, true),
        (&[0xFF, 0xFE], false, true),
        (&[0x00, 0x00], false, false),
        (&[0xFF, 0xFF], false, false),
        (&[0x01, 0x00], true, true),
        (&[0xFF; 3], false, false),
        (&[0xFF; 6], false, false),
        (&[0xFF, 0xFF, 0xFF, 0xFF, 0xBA, 0xC0], false, false),
    ];
    for (mac, unmarked, marked) in cases {
        client.device_table.lock().upsert(routed_row(&router, mac));
        for one_station in [false, true] {
            let goes = if one_station { marked } else { unmarked };
            let forms = [
                Station::routed(&router, NETWORK, mac),
                Station::remote(NETWORK, mac),
                Station::routed_via_local_broadcast(NETWORK, mac),
            ]
            .map(|to| if one_station { to.one_station() } else { to });
            for to in &forms {
                assert_eq!(to.is_one_station(), one_station);
                assert_eq!(to.check_confirmed().is_ok(), goes, "{to:?}");
                sent.clear();
                let result = client.confirmed_request(to, READ, &[0x0C]).await;
                if goes {
                    assert!(timed_out(&result), "{to:?}: {result:?}");
                    let [frame] = sent.frames().try_into().unwrap();
                    let dest = frame.decode_npdu().destination.unwrap();
                    assert_eq!((dest.network, dest.mac_address.as_slice()), (NETWORK, mac));
                } else {
                    assert!(
                        refused_with(&result, "only an unconfirmed request"),
                        "{to:?}: {result:?}"
                    );
                    assert!(sent.is_empty(), "{to:?} was sent");
                }
            }
        }
    }
    assert_eq!(client.tsm.lock().pending_count(), 0);
    // The mark is a remote address's: a station on this link stays as it is.
    assert!(!Station::local(&[0xFF]).one_station().is_one_station());
    client.stop().await.unwrap();
}

/// A refusal names the mark as the way through only for an unmarked form
/// the mark lifts, and a marked address still refused says so.
#[tokio::test(start_paused = true)]
async fn a_refusal_names_the_mark_only_where_it_lifts_the_form() {
    let (mut client, _sent, _inbound) = client_on(
        DataLink::Mstp,
        &[1],
        no_groups(),
        ClientOptions::default(),
        0,
    )
    .await;
    let router = [9];
    let station = |mac: &[u8], one_station| {
        let to = Station::routed(&router, NETWORK, mac);
        if one_station { to.one_station() } else { to }
    };
    let way = "mark it one station (one_station)";
    // The station, whether the refusal names the mark, whether it is marked.
    for (to, names_the_mark, marked) in [
        (station(&[0xFF], false), true, false),
        (station(&[0x00, 0x07], false), true, false),
        (station(&[0xFF, 0x07], false), true, false),
        (station(&[0x00, 0x00], false), false, false),
        (station(&[0xFF; 3], false), false, false),
        (station(&[0x00, 0x00], true), false, true),
        (station(&[0xFF, 0xFF], true), false, true),
        (station(&[0xFF; 6], true), false, true),
    ] {
        let result = client.confirmed_request(&to, READ, &[0x0C]).await;
        assert!(
            refused_with(&result, "only an unconfirmed request"),
            "{result:?}"
        );
        assert_eq!(
            refused_with(&result, way),
            names_the_mark,
            "{to:?}: {result:?}"
        );
        assert_eq!(
            refused_with(&result, "even marked one station"),
            marked,
            "{to:?}: {result:?}"
        );
    }
    client.stop().await.unwrap();
}
