---
title: "Network Port and Number controls"
description: "Separate a declared object, a registered B/IP receiving port, and passive local Number learning."
---

[Build and integrate](/rusty-bacnet/development/overview/) / Network Port and Number controls

A Network Port object, a transport's bound socket and a learned network number are related but distinct. Choose the contract that fits your task.

## Three different actions

| Action | What it establishes |
|---|---|
| Declare `NetworkPortObject::new_bip` or a DeviceIdentity port | A configured object snapshot; no socket bind, NIC discovery or automatic authority |
| Register one B/IP receiving port | An explicit association between one selected built-in object and this owner's actual bound address and B/IP mode |
| Receive local Number controls | Passive nonrouter learning and replies on an opted-in transport; no new registration, router or configured-number API |

## Register a B/IP receiving port

Use registration when a full server or shared B/IP endpoint should expose its actual receiving port through the selected Network Port object.

1. Create a built-in IPV4 Network Port with a concrete unicast interface, configured UDP port and local Port ID 1–255. UDP port zero is allowed before bind; it is not the Port ID.
2. Add it to the database and explicitly select its object identifier with `ServerConfig.registered_network_port`, the server builder's `.registered_network_port(oid)`, or `BipEndpointBuilder::registered_network_port(oid)`.
3. Start the owner. It validates the bound B/IP capability and reconciles the selected IP, actual UDP port, derived MAC and B/IP mode: NORMAL, FOREIGN or BBMD.
4. Read the selected object or, on the documented RP/RPM paths, the Network Port wildcard instance 4194303. Successful responses name the concrete selected object.

Mask, gateway and DNS stay explicit configuration. Port `APDU_Length` (399) describes local capacity independently of Device `Max_APDU_Length_Accepted` (62), remote requester limits and routed path limits. Do not copy one limit into all four roles.

The port follows the transport's mode (#939). A foreign device adds FD_BBMD_Address and FD_Subscription_Lifetime; a BBMD adds its BDT, FDT and BBMD_Accept_FD_Registrations, read live from the transport's tables. These rows are read-only for now. Registration rejects a transport that is both a BBMD and a foreign device, a wildcard interface and non-B/IP links. It protects the selected object against replacement/removal and activation-dependent changes while admitted work and the socket remain owned. A declaration in a shared database cannot give an unregistered responder another owner's association. The [canonical registration contract](https://github.com/jscott3201/rusty-bacnet/blob/v0.12.0/docs/rust-api.md#registered-bip-network-port) covers alias resolution and cleanup details.

## Let passive Number learning do its work

Supported owners consume What-Is-Network-Number and Network-Number-Is automatically. There is no proactive startup announcement. An unregistered runtime starts UNKNOWN even if its database contains an unrelated configured object. A registered nonzero configured number starts CONFIGURED and retains that authority.

| Current state and input | Result |
|---|---|
| UNKNOWN + valid local query | No reply because the owner knows no number |
| UNKNOWN or LEARNED + valid broadcast NNI, flag 0 | Learn the announced number |
| Any nonconfigured learned state + valid broadcast NNI, flag 1 | Learn with configured precedence; later flag-0 announcements cannot replace it |
| Locally CONFIGURED + conflicting NNI | Keep configuration; emit a debug diagnostic |
| Known state + valid local unicast or broadcast query | Send a local-broadcast NNI; learned states transmit flag 0 |

Routed controls, malformed payloads and unicast NNI are ignored. Rejection of zero, reserved 65535 and flags beyond 0/1 is the implementation's validation policy. A Forwarded-NPDU can be a logical broadcast even when its UDP hop is unicast; it still has to pass that mode's transport admission rules. Learning a number does not authenticate its origin.

Standalone clients use the same learning rules without a Device object, registration or configured-number setter. Reconstructing an unregistered runtime resets it to UNKNOWN; learned state is not persistent configuration.

## Know which owner and link were exercised

Full servers and shared endpoints cover NORMAL B/IP, SC and MS/TP; B/IP BBMD/foreign Number controls also have bounded shared-endpoint wire evidence. Standalone Rust clients have independent Number wire evidence for NORMAL/BBMD/foreign B/IP, SC, Linux Ethernet and normal/configured-foreign B/IPv6. Full B/IPv6 servers cover both modes. The standalone B/IP client cases independently capture BBMD Original-Broadcast and configured-foreign DBTN through admission/refusal, alternate forwarding and registration NAK/retry, with requester progress and stopped-socket release. Independent standalone-client MS/TP frame evidence remains under #879. These are Number-specific claims; broader BBMD/foreign endpoint administration remains experimental.

Each owner has a bounded serial Number worker separate from APDU dispatch. Holding that producer does not stop independent incoming request/ACK processing, but a shared physical writer still serializes outgoing bytes. Stop cancels and joins owned control work. Already transmitted bytes cannot be retracted.

## Next steps

Use the [transport and evidence matrix](/rusty-bacnet/development/transports/) to distinguish actual wire, simulated and platform coverage. For complete state, capacity and per-media rules, read [Local Network Number controls](https://github.com/jscott3201/rusty-bacnet/blob/v0.12.0/docs/rust-api.md#local-network-number-controls).
