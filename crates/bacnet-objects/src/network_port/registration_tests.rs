//! A port refuses an owner's reservation by why it can't take one (#1833),
//! with the same text for each.

use super::*;
use alloc::sync::Arc;

const UNAVAILABLE: &str = "Network Port is unavailable for B/IP registration";

#[test]
fn a_reservation_is_refused_by_why_the_port_cannot_take_one() {
    // No IP configuration: no B/IP port.
    let mut virtual_port = NetworkPortObject::new_non_bip(
        1,
        "Port",
        NetworkType::VIRTUAL,
        0,
        MacAddr::from_slice(&[1]),
        1476,
    )
    .unwrap();
    let refused = virtual_port.reserve_internal(&Arc::new(()));
    assert!(
        matches!(&refused, Err(Error::Unsupported(why)) if why.to_string() == UNAVAILABLE),
        "{refused:?}"
    );

    // Held by another owner: a one-holder resource taken.
    let mut port = NetworkPortObject::new_bip(1, "Port", BipPortConfig::default()).unwrap();
    let owner = Arc::new(());
    port.reserve_internal(&owner).unwrap();
    let refused = port.reserve_internal(&Arc::new(()));
    assert!(
        matches!(&refused, Err(Error::LimitReached(why)) if why.to_string() == UNAVAILABLE),
        "{refused:?}"
    );
    drop(owner);

    // Left Out_Of_Service by the application: its configuration refused.
    port.out_of_service = true;
    let refused = port.reserve_internal(&Arc::new(()));
    assert!(
        matches!(&refused, Err(Error::InvalidConfig(why)) if why.to_string() == UNAVAILABLE),
        "{refused:?}"
    );

    // Back in service and unheld, it takes the reservation.
    port.out_of_service = false;
    port.reserve_internal(&Arc::new(())).unwrap();
}
