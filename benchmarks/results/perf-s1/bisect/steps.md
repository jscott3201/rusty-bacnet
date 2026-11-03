# Per-function changes at the steps that moved (self instructions and calls per iteration, counting allocator included)

Default release profile:

```
## encode_read_property_request: 1da307a74 -> fca238953: total 501.0 -> 768.0 (+267.0)
calls changed: <&mut [u8] as bacnet_encoding::sink::EncodeSink>::write 0->2, write 0->1, __GI_memcpy 5->7, bacnet_encoding::primitives::scalar::encode_ctx_object_id 1->0, bacnet_encoding::primitives::scalar::encode_ctx_unsigned 1->0, <&mut [u8]> 0->2, bacnet_services::read_property::encode_reference 0->3, {closure#0}> 0->1
      0.0   160.0  +160.0  <&mut [u8] as bacnet_encoding::sink::EncodeSink>::write
      0.0   102.0  +102.0  bacnet_encoding::primitives::scalar::write_tagged::<&mut [u8]>
     83.0     0.0   -83.0  bacnet_encoding::primitives::scalar::encode_ctx_unsigned
     66.0     0.0   -66.0  bacnet_encoding::primitives::scalar::encode_ctx_object_id
      0.0    53.0   +53.0  bacnet_services::read_property::encode_reference
      0.0    42.0   +42.0  <bytes::bytes_mut::BytesMut as bacnet_encoding::sink::EncodeSink>::write
     96.0   128.0   +32.0  __GI_memcpy
      0.0    24.0   +24.0  bacnet_services::who_is::staged::<bytes::bytes_mut::BytesMut, <bacnet_services::read_property::ReadPropertyReq
## decode_npdu_100b_payload: fca238953 -> 62a5baf1d: total 225.0 -> 276.0 (+51.0)
calls changed: <bacnet_npdu::view::Npdu>::decode 0->1
      0.0    46.0   +46.0  <bacnet_npdu::view::Npdu>::decode
    134.0   139.0    +5.0  bacnet_encoding::npdu::decode_npdu
## encode_npdu_100b_payload: fca238953 -> 62a5baf1d: total 449.0 -> 488.0 (+39.0)
calls changed: <bacnet_npdu::view::Npdu>::encode_into 0->1, <bacnet_npdu::view::Npdu>::encoded_len 0->1, put_slice 1->0, <bytes::bytes_mut::BytesMut>::resize 0->1, __GI_memcpy 3->1, __GI_memset 0->1
    121.0     0.0  -121.0  <bytes::bytes_mut::BytesMut as bytes::buf::buf_mut::BufMut>::put_slice
      0.0    94.0   +94.0  <bacnet_npdu::view::Npdu>::encode_into
     60.0    20.0   -40.0  __GI_memcpy
      0.0    37.0   +37.0  <bytes::bytes_mut::BytesMut>::resize
     22.0    48.0   +26.0  bacnet_encoding::npdu::encode_npdu
      0.0    22.0   +22.0  <bacnet_npdu::view::Npdu>::encoded_len
      0.0    21.0   +21.0  __GI_memset
## decode_npdu_100b_payload: f729a457d -> 4a098fccb: total 276.0 -> 221.0 (-55.0)
calls changed: __aarch64_ldadd8_rel 2->1, __aarch64_ldadd8_relax 2->1, bytes::bytes::shared_clone 1->0, bytes::bytes::shared_drop 2->1
     20.0     0.0   -20.0  bytes::bytes::shared_clone
     30.0    15.0   -15.0  bytes::bytes::shared_drop
    139.0   129.0   -10.0  bacnet_encoding::npdu::decode_npdu
     10.0     5.0    -5.0  __aarch64_ldadd8_rel
     10.0     5.0    -5.0  __aarch64_ldadd8_relax
## encode_read_property_request: 4a098fccb -> 28262cb77: total 747.0 -> 674.0 (-73.0)
calls changed: __GI_memcpy 7->6, <&mut [u8]> 2->1
     98.0    49.0   -49.0  bacnet_encoding::primitives::scalar::write_tagged::<&mut [u8]>
    128.0   112.0   -16.0  __GI_memcpy
    156.0   147.0    -9.0  <&mut [u8] as bacnet_encoding::sink::EncodeSink>::write
     54.0    55.0    +1.0  bacnet_services::read_property::encode_reference
## encode_read_property_request: 2788802c3 -> 06722c58e: total 674.0 -> 747.0 (+73.0)
calls changed: __GI_memcpy 6->7, <&mut [u8]> 1->2
     49.0    98.0   +49.0  bacnet_encoding::primitives::scalar::write_tagged::<&mut [u8]>
    112.0   128.0   +16.0  __GI_memcpy
    147.0   156.0    +9.0  <&mut [u8] as bacnet_encoding::sink::EncodeSink>::write
     55.0    54.0    -1.0  bacnet_services::read_property::encode_reference
```

Release-like (fat LTO, one codegen unit):

```
## encode_read_property_request: 0853ea403 -> d5a08d625: total 493.0 -> 288.0 (-205.0)
calls changed: encode 2->1, put_slice 3->0, __GI_memcpy 4->0
    166.0     0.0  -166.0  <bytes::bytes_mut::BytesMut as bytes::buf::buf_mut::BufMut>::put_slice
     76.0     0.0   -76.0  __GI_memcpy
     29.0    63.0   +34.0  <bacnet_services::read_property::ReadPropertyRequest>::encode
     17.0    20.0    +3.0  bacnet_encoding::primitives::encode_unsigned
## encode_npdu_100b_payload: 0853ea403 -> d5a08d625: total 455.0 -> 334.0 (-121.0)
calls changed: put_slice 1->0, __GI_memcpy 3->1
    142.0     0.0  -142.0  <bytes::bytes_mut::BytesMut as bytes::buf::buf_mut::BufMut>::put_slice
     24.0    85.0   +61.0  bacnet_encoding::npdu::encode_npdu
     60.0    20.0   -40.0  __GI_memcpy
## encode_npdu_100b_payload: fca238953 -> 62a5baf1d: total 333.0 -> 420.0 (+87.0)
calls changed: __GI_memset 0->1
     84.0   151.0   +67.0  bacnet_encoding::npdu::encode_npdu
      0.0    21.0   +21.0  __GI_memset
     32.0    31.0    -1.0  codec_counts::run::{closure#4}
```
