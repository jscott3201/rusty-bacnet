//! Bounds on the request futures' sizes (#2617).
//!
//! Every typed call's future holds `confirmed_request_inner`'s, and the
//! tasks applications run hold those: an ops handle's read, an MCP tool
//! call, a TUI worker's job. A debug build keeps several copies of each
//! future on the stack of whatever polls it, at every layer: in #2617, 320
//! bytes of segmented-request limits held by value put 48 KiB on the TUI's
//! agent tests, which then overflowed the 1 MiB thread stacks CI's stack
//! step gives Windows, and 48 bytes more still cost 16 KiB. Users on small
//! stacks pay the same. The bounds are the sizes
//! at 54fec70ed (debug, 64-bit) plus 32 bytes. State that only an audited
//! or otherwise cold request needs goes behind a box on that path, out of
//! these futures; a change that has to grow them past a bound first shows
//! the stack step still passes on Windows, then moves the bound.

use super::station::ConfirmedTarget;
use super::*;
use bacnet_transport::test_transport::TestTransport;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;

/// `confirmed_request_inner`'s future was 2,640 bytes at 54fec70ed.
const REQUEST_BOUND: usize = 2_640 + 32;

/// `read_property(&Station)`'s future was 2,888 bytes at 54fec70ed.
const READ_PROPERTY_BOUND: usize = 2_888 + 32;

#[tokio::test]
async fn the_request_futures_stay_within_their_bounds() {
    let client = BACnetClient::generic_builder()
        .transport(TestTransport::builder().local_mac(&[1]).build())
        .build()
        .await
        .unwrap();
    let request = client.confirmed_request_inner(
        ConfirmedTarget::Local { mac: &[2] },
        ConfirmedServiceChoice::READ_PROPERTY,
        &[],
    );
    let request_size = std::mem::size_of_val(&request);
    drop(request);
    let station = Station::local(&[2]);
    let read = client.read_property(
        &station,
        ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 1).unwrap(),
        PropertyIdentifier::PRESENT_VALUE,
        None,
    );
    let read_size = std::mem::size_of_val(&read);
    drop(read);
    assert!(
        request_size <= REQUEST_BOUND,
        "confirmed_request_inner's future is {request_size} bytes, over {REQUEST_BOUND}"
    );
    assert!(
        read_size <= READ_PROPERTY_BOUND,
        "read_property's future is {read_size} bytes, over {READ_PROPERTY_BOUND}"
    );
}
