---
title: "Compose a shared endpoint"
description: "Run requester and bounded responder roles through one B/IP, SC or MS/TP transport owner."
---

[Build and integrate](/rusty-bacnet/development/overview/) / Shared endpoints

Use an endpoint when one local device needs to initiate requests and serve a bounded request set through the same transport. Keep standalone `BACnetClient` for a requester or `BACnetServer` for the full server service set; both remain public APIs.

## One session owns the connection

Rust's `EndpointSession` owns transport startup, ingress and shared outbound coordination. Choose `ClientOnly`, `ServerOnly` or `Both` before startup. The role handles use that owner and cannot independently restart its transport.

This B/IP example binds an ephemeral loopback port, creates both roles, then explicitly stops the owner. It does not discover or write to another device:

```rust
use std::net::Ipv4Addr;
use bacnet_endpoint::{bip::BipEndpointBuilder, DeviceIdentity, SessionRole};

async fn example() -> Result<(), bacnet_types::error::Error> {
    let identity = DeviceIdentity::new(1001, 42)?
        .with_bip_port(1, 0, Ipv4Addr::LOCALHOST, 0)?;
    let db = identity.build_database()?;
    let mut session = BipEndpointBuilder::new(
        Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST,
    )
        .role(SessionRole::Both)
        .database(db)
        .identity(identity)
        .build_session()?;
    session.start().await?;
    let local = session.bip_local_address();
    println!("Bound endpoint: {local:?}");
    session.stop().await?;
    Ok(())
}
```

The declared Network Port above is not automatically registered. Explicit registration is a separate [receiving-port contract](/rusty-bacnet/development/network-number/). A successful start exposes the actual B/IP address, including the ephemeral port; after stop it is unavailable.

For SC, `ScEndpointBuilder` composes a caller-dialed `TlsWebSocket` with `build_hub_session` under the `sc-tls` feature. For MS/TP, `MstpEndpointBuilder` owns one serial link. Python exposes `BipEndpoint`, `ScEndpoint` and `MstpEndpoint`, with `async with endpoint`, `await endpoint.client()` and `await endpoint.server()`. B/IPv6 and Ethernet have no shared endpoint builder; use their standalone owners.

## Check requester and responder scope separately

| Role | Bounded contract |
|---|---|
| Requester reads | ReadProperty, ReadRange and ReadPropertyMultiple; unsegmented requests/responses |
| Requester RPM | 1–64 explicit property occurrences; concrete objects; no ALL, REQUIRED, OPTIONAL or wildcard object selectors |
| Requester writes | Direct IPv4 B/IP WriteProperty only; required commandability declaration; no routed WP or WPM in this profile |
| Default responder | ReadProperty, with Reject/Abort handling; no responder RPM, COV or full-server parity |
| Opt-in responder writes | One built-in local Device's Description and, with a complete source profile, Audit_Notification_Recipient; mandatory mutation authorizer |

The Device's advertised services must match the responder. Server-role declarations include RP and cannot add unsupported services; explicit write opt-in advertises RP+WP only after validation. The endpoint responder does not expose Device COV list properties.

A requester write's commandability is a declaration about the remote property, not permission to write it. Rust requires `Commandability::{Commandable, Noncommandable}`; Python requires `commandability="commandable"` or `"noncommandable"`. For a first integration, exercise reads before enabling mutations. Consult [request admission limits](https://github.com/jscott3201/rusty-bacnet/blob/v0.12.0/docs/request-admission.md) for the shared budgets.

## Add source READ or WRITE Audit reporting deliberately

The optional Rust source Reporter profile belongs to this session and currently covers **direct IPv4 B/IP**. Configure it before startup:

1. Put a concrete built-in Device and the selected Audit Reporter in the database. If it holds several Devices, the lowest instance is the local one ([details](https://github.com/jscott3201/rusty-bacnet/blob/v0.12.0/docs/rust-api.md#databases-with-several-devices)).
2. Provision the Device's typed Audit recipient. A Device recipient needs an immutable `source_audit_device_binding`; a direct Address recipient needs none.
3. Set the Reporter's audit level and READ/WRITE operation policy. `Monitored_Objects` must be absent, including no empty or NULL-only list.
4. Select `with_source_audit_reporter`. A `Both` session also requires an explicit Device write authorizer.

The [canonical source Reporter example](https://github.com/jscott3201/rusty-bacnet/blob/v0.12.0/docs/rust-api.md#bounded-endpoint-source-read-reporting) supplies the exact setup. Python exposes endpoint requests but **no source Reporter configuration**.

READ reporting covers RP, ReadRange and RPM. Records retain attempted identities and outcomes; returned values remain caller-only. An RPM operation can produce a separate record for each eligible occurrence. WRITE reporting records eligible attempts across retries, with a complete Target_Value only when its encoding fits 0–32 bytes. It never invents remote Current_Value, target timestamp or proof that a write executed. Notification delivery failure does not replace the request's result.

This source profile is separate from the full server's [target Reporter ownership](https://github.com/jscott3201/rusty-bacnet/blob/v0.12.0/docs/target-audit-reporters.md), [Device recipient](https://github.com/jscott3201/rusty-bacnet/blob/v0.12.0/docs/device-audit-recipient.md) and Audit Log forwarding contracts.

## Close the owner, including after cancellation

In Rust, await `session.stop()`; in Python, await `close()` or leave the async context. Exported role handles do not keep a closed session usable. Stop owns admitted work and transport cleanup; dropping a caller's request is not a substitute for stopping the endpoint.

Eligible source-reported requests retain terminal observation after caller cancellation. Other caller-owned requests can cancel queued sends, but an already attempted send may have reached its peer. Cancellation never proves that a remote write did not execute.

Python startup and close serialize through the native lifecycle owner. If startup cancellation races publication, explicitly await close. A canceled admitted close waiter leaves cleanup owned; another awaited close joins it. Successful start consumes registrations, so restart does not replay them. See the [full lifecycle contract](https://github.com/jscott3201/rusty-bacnet/blob/v0.12.0/docs/python-api.md#endpoint-lifecycle-and-cancellation) for startup rollback boundaries.

## Next steps

[Register a B/IP receiving port](/rusty-bacnet/development/network-number/) · [Compare transport evidence](/rusty-bacnet/development/transports/) · [Exact Rust endpoint APIs](https://github.com/jscott3201/rusty-bacnet/blob/v0.12.0/docs/rust-api.md#bacnet-endpoint)
