## Calls in per request, full_stack_encode_rp/none: B - A (total over the 4 functions that moved by 0.5 or more: -1.0; the largest 4 listed)

| A | B | Change | Function |
| --- | --- | --- | --- |
| 14.00 | 16.00 | +2.00 | `__GI_memcpy` |
| 1.00 | 0.00 | -1.00 | `<bacnet_npdu::view::Npdu>::encoded_len` |
| 1.00 | 0.00 | -1.00 | `<bytes::bytes_mut::BytesMut>::resize` |
| 1.00 | 0.00 | -1.00 | `__GI_memset` |

## Self instructions per request, full_stack_encode_rp/none: B - A (total over the 6 functions that moved by 0.5 or more: +10.0; the largest 6 listed)

| A | B | Change | Function |
| --- | --- | --- | --- |
| 94.00 | 141.00 | +47.00 | `<bacnet_npdu::view::Npdu>::encode_into` |
| 37.00 | 0.00 | -37.00 | `<bytes::bytes_mut::BytesMut>::resize` |
| 254.00 | 287.00 | +33.00 | `__GI_memcpy` |
| 22.00 | 0.00 | -22.00 | `<bacnet_npdu::view::Npdu>::encoded_len` |
| 12.00 | 0.00 | -12.00 | `__GI_memset` |
| 52.00 | 53.00 | +1.00 | `bacnet_encoding::npdu::encode_npdu` |

