//! Bounds on the request futures' sizes (#2617).
//!
//! Every typed call's future holds `confirmed_request_inner`'s, which holds
//! the segmented request's, and the tasks applications run hold those: an
//! ops handle's read, an MCP tool call, a TUI worker's job. A debug build
//! keeps several copies of each future on the stack of whatever polls it,
//! at every layer: in #2617, 320 bytes of segmented-request limits held by
//! value put 48 KiB on the TUI's agent tests, which then overflowed the
//! 1 MiB thread stacks CI's stack step gives Windows, and 48 bytes more
//! still cost 16 KiB. Users on small stacks pay the same.
//!
//! The bounds are the sizes after #2558's splits (debug, 64-bit, Rust
//! 1.99.0, measured on macOS aarch64 only) plus 32 bytes. State that only
//! an audited or otherwise cold request needs
//! goes behind a box on that path, out of these futures; a change that has
//! to grow them past a bound first shows the stack step still passes on
//! Windows, then moves the bound. A toolchain bump can move these layouts
//! too, either way: one that trips a bound means measuring the sizes on dev
//! again with the new compiler and setting the bounds from those, not a
//! regression to chase here.
//!
//! Nothing the dispatch and segmented-receive futures hold is laid out by
//! platform: no field under a `target_os`, `unix` or `windows` cfg, and the
//! one feature-gated field, a direct BACnet/SC peer's certificate, sits
//! behind an `Arc`. So they take the same 32-byte margin. The request
//! futures' bounds were measured the same way; CI checks every bound on
//! Linux and Windows as well.
//!
//! The wait for the answer, `wait_for_confirmed_response`, has no bound of
//! its own: its future went from 1,024 to 1,120 bytes in #2558, the timer
//! expiries' captures, and neither future that holds it grew. In
//! `confirmed_request_inner` it sits beside the larger segmented request;
//! in the segmented request, inside `send_segments`, beside the larger wait
//! for a window's acknowledgement.

use super::segmentation_context::SegmentedRequestLimits;
use super::station::ConfirmedTarget;
use super::*;
use bacnet_transport::test_transport::TestTransport;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;

/// `confirmed_request_inner`'s future was 2,640 bytes before #2558 split
/// the request side, and 2,576 after.
const REQUEST_BOUND: usize = 2_576 + 32;

/// `read_property(&Station)`'s future was 2,904 bytes before that split,
/// and 2,840 after.
const READ_PROPERTY_BOUND: usize = 2_840 + 32;

/// `segmented_confirmed_request`'s future was 1,976 bytes before that
/// split, and 1,920 after.
const SEGMENTED_BOUND: usize = 1_920 + 32;

#[tokio::test]
async fn the_request_futures_stay_within_their_bounds() {
    let client = BACnetClient::generic_builder()
        .transport(TestTransport::builder().local_mac(&[1]).build())
        .build()
        .await
        .unwrap();
    let target = ConfirmedTarget::Local { mac: &[2] };
    let request =
        client.confirmed_request_inner(target, ConfirmedServiceChoice::READ_PROPERTY, &[], None);
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
    let mut lease = None;
    let mut limits = SegmentedRequestLimits {
        named: &target,
        remote_max_apdu: 480,
        remote_max_segments: None,
        routed_forwarded_npci_len: None,
        routed_path_lease: &mut lease,
        sized_by: None,
        attributes: None,
    };
    let segmented = client.segmented_confirmed_request(
        target,
        ConfirmedServiceChoice::WRITE_PROPERTY,
        &[],
        &mut limits,
    );
    let segmented_size = std::mem::size_of_val(&segmented);
    drop(segmented);
    assert!(
        request_size <= REQUEST_BOUND,
        "confirmed_request_inner's future is {request_size} bytes, over {REQUEST_BOUND}"
    );
    assert!(
        read_size <= READ_PROPERTY_BOUND,
        "read_property's future is {read_size} bytes, over {READ_PROPERTY_BOUND}"
    );
    assert!(
        segmented_size <= SEGMENTED_BOUND,
        "segmented_confirmed_request's future is {segmented_size} bytes, over {SEGMENTED_BOUND}"
    );
}

/// `dispatch_apdu`'s future, which the client's dispatch task holds while
/// it handles each frame, was 2,784 bytes before #2558 moved most of its
/// arms into functions of their own, and 2,152 after (debug, 64-bit, Rust
/// 1.99.0). A local saved across two awaits sits in the future's prefix, so
/// the arms' locals used to add up rather than overlap.
const DISPATCH_BOUND: usize = 2_152 + 32;

/// `handle_segmented_complex_ack`'s future, which the dispatch future holds
/// while a segment is received, was 1,512 bytes before #2558 split it, and
/// 1,464 after.
const SEGMENTED_RECEIVE_BOUND: usize = 1_464 + 32;

type SegState = &'static mut HashMap<SegKey, SegmentedReceiveState>;

/// The size of `dispatch_apdu`'s future, without making one.
fn dispatch_size<F>(
    _make: impl FnOnce(
        &'static super::dispatch_context::DispatchContext<'static, TestTransport>,
        SegState,
        super::dispatch_context::InboundApdu<'static>,
        Apdu,
        ResponseLimits,
    ) -> F,
) -> usize {
    std::mem::size_of::<F>()
}

/// The size of `handle_segmented_complex_ack`'s future, without making one.
fn receive_size<F>(
    _make: impl FnOnce(
        &'static Arc<SyncMutex<Tsm>>,
        &'static Arc<NetworkLayer<TestTransport>>,
        SegState,
        super::segmentation_context::InboundSegmentSource<'static>,
        bacnet_encoding::apdu::ComplexAck,
        ResponseLimits,
    ) -> F,
) -> usize {
    std::mem::size_of::<F>()
}

#[test]
fn the_dispatch_futures_stay_within_their_bounds() {
    let dispatch = dispatch_size(BACnetClient::<TestTransport>::dispatch_apdu);
    let receive = receive_size(BACnetClient::<TestTransport>::handle_segmented_complex_ack);
    assert!(
        dispatch <= DISPATCH_BOUND,
        "dispatch_apdu's future is {dispatch} bytes, over {DISPATCH_BOUND}"
    );
    assert!(
        receive <= SEGMENTED_RECEIVE_BOUND,
        "handle_segmented_complex_ack's future is {receive} bytes, over {SEGMENTED_RECEIVE_BOUND}"
    );
}
