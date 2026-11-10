## Calls in per request, encode_read_property_request/none: B - A (total over the 3 functions that moved by 0.5 or more: -3.0; the largest 3 listed)

| A | B | Change | Function |
| --- | --- | --- | --- |
| 1.00 | 0.00 | -1.00 | `<bytes::bytes_mut::BytesMut as bacnet_encoding::sink::EncodeSink>::write` |
| 7.00 | 6.00 | -1.00 | `__GI_memcpy` |
| 2.00 | 1.00 | -1.00 | `bacnet_encoding::primitives::scalar::write_tagged::<&mut [u8]>` |

## Self instructions per request, encode_read_property_request/none: B - A (total over the 5 functions that moved by 0.5 or more: -94.0; the largest 5 listed)

| A | B | Change | Function |
| --- | --- | --- | --- |
| 98.00 | 49.00 | -49.00 | `bacnet_encoding::primitives::scalar::write_tagged::<&mut [u8]>` |
| 42.00 | 0.00 | -42.00 | `<bytes::bytes_mut::BytesMut as bacnet_encoding::sink::EncodeSink>::write` |
| 54.00 | 76.00 | +22.00 | `bacnet_services::read_property::encode_reference` |
| 128.00 | 112.00 | -16.00 | `__GI_memcpy` |
| 156.00 | 147.00 | -9.00 | `<&mut [u8] as bacnet_encoding::sink::EncodeSink>::write` |

