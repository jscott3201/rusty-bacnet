## Calls in per request, encode_npdu_100b_payload/none: B - A (total over the 4 functions that moved by 0.5 or more: -1.0; the largest 4 listed)

| A | B | Change | Function |
| --- | --- | --- | --- |
| 1.00 | 3.00 | +2.00 | `__GI_memcpy` |
| 1.00 | 0.00 | -1.00 | `<bacnet_npdu::view::Npdu>::encoded_len` |
| 1.00 | 0.00 | -1.00 | `<bytes::bytes_mut::BytesMut>::resize` |
| 1.00 | 0.00 | -1.00 | `__GI_memset` |

## Self instructions per request, encode_npdu_100b_payload/none: B - A (total over the 6 functions that moved by 0.5 or more: +1.0; the largest 6 listed)

| A | B | Change | Function |
| --- | --- | --- | --- |
| 94.00 | 141.00 | +47.00 | `<bacnet_npdu::view::Npdu>::encode_into` |
| 37.00 | 0.00 | -37.00 | `<bytes::bytes_mut::BytesMut>::resize` |
| 20.00 | 53.00 | +33.00 | `__GI_memcpy` |
| 22.00 | 0.00 | -22.00 | `<bacnet_npdu::view::Npdu>::encoded_len` |
| 21.00 | 0.00 | -21.00 | `__GI_memset` |
| 48.00 | 49.00 | +1.00 | `bacnet_encoding::npdu::encode_npdu` |

