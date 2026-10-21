# bacnet-endpoint-core

Shared outbound transaction coordination and single-owner APDU ingress for the
[Rusty BACnet](https://github.com/jscott3201/rusty-bacnet) workspace.
[`coordinator::OutboundTransactionCoordinator`] leases device-wide invoke IDs
and matches decoded responses to their owning operation. [`endpoint_ingress`]
classifies incoming APDUs into bounded role queues above one network layer.
Applications that need a complete client/server lifecycle can use
`bacnet-endpoint`; this crate provides the underlying coordination components.

This hosted crate requires `std` and allocating state. It depends on Tokio,
`bacnet-network` and `bacnet-transport`, including when only the synchronous
coordinator is used. It has no public Cargo features, alloc-only profile or
no-heap profile.

## Quickstart from development source

This dependency block assumes an application directory beside a checkout named
`rusty-bacnet`. These paths refer to development source, not a published crate
version; adjust them for another layout. Every crate used by the program is a
direct dependency.

```toml
[dependencies]
bacnet-endpoint-core = { path = "../rusty-bacnet/crates/bacnet-endpoint-core" }
bacnet-encoding = { path = "../rusty-bacnet/crates/bacnet-encoding" }
bacnet-types = { path = "../rusty-bacnet/crates/bacnet-types" }
```

Reserve one requester lease and locally construct its WriteProperty SimpleACK.
Admitting the terminal response claims its delivery slot; completion releases
the exact lease. This program performs local correlation bookkeeping using a
synthetic APDU. It sends no request and opens no transport or network socket.

```rust
use bacnet_encoding::apdu::{Apdu, SimpleAck};
use bacnet_endpoint_core::coordinator::{
    AdmissionOutcome, CanonicalPeer, LeaseMetadata, OutboundTransactionCoordinator,
    ReleaseOutcome, TerminalPolicy,
};
use bacnet_types::enums::ConfirmedServiceChoice;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let coordinator = OutboundTransactionCoordinator::new();
    let peer = CanonicalPeer::direct(&[2]);
    let token = coordinator.reserve(LeaseMetadata::requester(
        peer.clone(),
        ConfirmedServiceChoice::WRITE_PROPERTY,
        TerminalPolicy::SimpleAck,
    ))?;
    let response = Apdu::SimpleAck(SimpleAck {
        invoke_id: token.invoke_id(),
        service_choice: ConfirmedServiceChoice::WRITE_PROPERTY,
    });

    let admission = match coordinator.admit(&peer, &response)? {
        AdmissionOutcome::Admitted(admission) => admission,
        other => panic!("expected the reserved response, got {other:?}"),
    };
    assert_eq!(admission.token(), token);
    assert_eq!(
        coordinator.admit(&peer, &response)?,
        AdmissionOutcome::DuplicateTerminal,
    );
    assert_eq!(coordinator.complete(token)?, ReleaseOutcome::Released);
    assert_eq!(coordinator.active_count()?, 0);
    Ok(())
}
```

## Ownership and limits

The coordinator has one device-wide pool of 256 invoke IDs. Local generation
tokens distinguish successive leases for cleanup, but do not distinguish stale
on-wire responses after invoke-ID reuse: the generation is not carried in the
response. Admission does not release a lease; its owner must complete, cancel
or release the exact token. Peer and service matching are correlation checks,
not authentication. Routed source addresses remain claims, not credentials.

Each coordinator method holds a synchronous mutex only while inspecting or
updating its in-memory state. The example needs no Tokio runtime, but the
crate's ingress and asynchronous notification facilities still depend on Tokio.
[`endpoint_ingress::EndpointIngress`] owns network ingress and bounded role
classification; it does not provide the endpoint's application lifecycle or
full standalone-server service dispatch.

The example establishes local lease and duplicate-terminal behavior. It does
not exercise network delivery, transport security, firmware, or a physical
device, and does not establish BACnet conformance.

## Features and documentation

There are no public Cargo features. The docs.rs profile uses
`--no-default-features` on `x86_64-unknown-linux-gnu`; this does not remove the
crate's hosted `std`, Tokio or allocation requirements. This README is also the
crate landing page, so its Rust program is checked as a doctest.
