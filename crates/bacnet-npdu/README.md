# bacnet-npdu

The BACnet network-layer vocabulary from Clause 6 of ASHRAE 135, for
[rusty-bacnet](https://github.com/jscott3201/rusty-bacnet): one definition
of each concept, shared by the portable cores (`bacnet-mstp-core`,
`bacnet-bip-core`, `bacnet-router-core`), the hosted transports and network
layer, and the owned NPDU codec of `bacnet-encoding`.

- `#![no_std]`, no allocator, no dependencies.
- `Npdu`: a borrowed view of an NPDU's control information and payload.
  `Npdu::decode` reads one in place and reports an address too long for any
  data link apart from every other fault, with what a router needs to
  address its reject; `Npdu::encoded_len` and `Npdu::encode_into` write one
  into your buffer, refusing anything the decoder refuses before writing an
  octet.
- `NetworkMessageType` and `RejectMessageReason`: open sets of wire codes
  with named constants. Any octet converts, so a `match` on them needs a
  wildcard arm.
- `RejectMessageToNetwork`: a reject's reason and network, read and written
  in place; `RejectLengthError` refuses data that isn't three octets.
- `NetworkPriority`: the closed, two-bit network priority.
- `NpduLimits` and `MAX_NPDU` (1,497 octets): the longest NPDU a link
  carries each way.
- `Mac` and `MAX_MAC_LEN` (18 octets): a link-layer address stored inline.

## Example

```rust
use bacnet_npdu::{Address, NetworkMessageType, Npdu};

// What-Is-Network-Number, for the local network.
let question = Npdu::message(NetworkMessageType::WHAT_IS_NETWORK_NUMBER, &[]);
let mut frame = [0u8; 8];
let len = question.encode_into(&mut frame)?;
assert_eq!(&frame[..len], &[0x01, 0x80, 0x12]);

// A routed APDU, read in place.
let routed = [0x01, 0x20, 0x00, 0x02, 0x01, 0x05, 0xFF, 0x10, 0x08];
let npdu = Npdu::decode(&routed).expect("an NPDU");
assert_eq!(npdu.destination, Some(Address { network: 2, mac: &[5] }));
assert_eq!(npdu.payload, &[0x10, 0x08]);
assert!(!npdu.is_network_message());
# Ok::<(), Box<dyn core::error::Error>>(())
```

## License

MIT
