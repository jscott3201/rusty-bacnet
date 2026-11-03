# decode_read_property_request / none: self instructions per iteration

Median slope over the ordinals; the second column is B.
Calls in counts calls from other functions (a recursion level calling itself isn't one).

## Functions (callgrind_annotate, hash suffixes stripped)

| A | B | B - A | Calls in (A) | Function |
| --- | --- | --- | --- | --- |
| 122.0 | 48.0 | -74.0 | 2.00 | `bacnet_encoding::tags::decode_tag` |
| 89.0 | 0.0 | -89.0 | 2.00 | `bacnet_encoding::constructed::tagged::ctx_header` |
| 56.0 | 0.0 | -56.0 | 1.00 | `<bacnet_services::read_property::ReadPropertyRequest>::decode` |
| 47.0 | 0.0 | -47.0 | 1.00 | `bacnet_encoding::constructed::tagged::decode_ctx_primitive` |
| 42.0 | 0.0 | -42.0 | 1.00 | `bacnet_encoding::constructed::tagged::decode_ctx_fixed` |
| 31.0 | 0.0 | -31.0 | 1.00 | `bacnet_encoding::constructed::tagged::decode_ctx_unsigned::<u32>` |
| 27.0 | 0.0 | -27.0 | 1.00 | `bacnet_encoding::constructed::tagged::decode_ctx_object_id` |
| 18.0 | 0.0 | -18.0 | 1.00 | `bacnet_encoding::constructed::tagged::expect_end` |
| 14.0 | 0.0 | -14.0 | 1.00 | `bacnet_encoding::constructed::tagged::next_is_context` |
| 10.0 | 10.0 | 0.0 | 1.00 | `<bacnet_types::primitives::ObjectIdentifier>::decode` |
| 9.0 | 10.0 | 1.0 | 1.00 | `bacnet_encoding::primitives::decode_unsigned` |
| 0.0 | 0.0 | 0.0 | 0.00 | `bacnet_counts::parse_n` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<u64 as core::fmt::Display>::fmt` |
| 0.0 | 0.0 | 0.0 | 0.00 | `core::str::converts::from_utf8` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<core::fmt::Formatter>::pad_integral` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_sysdep_parse_arguments` |
| 0.0 | 0.0 | 0.0 | 0.00 | `write` |
| 0.0 | 0.0 | 0.0 | 0.00 | `strdup` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_int_free` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__rustc::__rust_no_alloc_shim_is_unstable_v2` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__pthread_mutex_unlock_usercnt` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_important_hwcaps` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x000000000000cbd0` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<bacnet_counts::AllocSnapshot>::now` |
| 0.0 | 0.0 | 0.0 | 0.00 | `sysmalloc` |
| 0.0 | 0.0 | 0.0 | 0.00 | `alloc_perturb` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x0000000000000590` |
| 0.0 | 0.0 | 0.0 | 0.00 | `getauxval` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::dealloc` |
| 0.0 | 0.0 | 0.0 | 0.00 | `strstr` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_start` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_add_to_slotinfo` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__getauxval2` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_allocate_tls_storage` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_audit_preinit` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__minimal_free` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_map_object_from_fd` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_sysdep_start` |
| 0.0 | 0.0 | 0.0 | 0.00 | `access` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_IO_default_finish` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__GI___close_nocancel` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_audit_objopen` |
| 0.0 | 0.0 | 0.0 | 0.00 | `call_init` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x0000000000003758` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_IO_link_in` |
| 0.0 | 0.0 | 0.0 | 0.00 | `calloc` |
| 0.0 | 0.0 | 0.0 | 0.00 | `expand_dynamic_string_token` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x0000000000003700` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x00000000000005c0` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_init_paths` |
| 0.0 | 0.0 | 0.0 | 0.00 | `rtld_mutex_dummy` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__read_nocancel` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__libc_start_main@@GLIBC_2.34` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__GI_strlen` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__ctype_init` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<std::sync::once::Once>::call_once::<std::rt::cleanup::{closure#0}>::{closure#0}` |
| 0.0 | 0.0 | 0.0 | 0.00 | `lookup_malloc_symbol` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__pthread_attr_extension` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_IO_doallocbuf` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_IO_file_close` |

## Largest function changes, B - A

| A | B | B - A | Function |
| --- | --- | --- | --- |
| 0.0 | 101.0 | 101.0 | `codec_counts::run::{closure#2}` |
| 89.0 | 0.0 | -89.0 | `bacnet_encoding::constructed::tagged::ctx_header` |
| 122.0 | 48.0 | -74.0 | `bacnet_encoding::tags::decode_tag` |
| 56.0 | 0.0 | -56.0 | `<bacnet_services::read_property::ReadPropertyRequest>::decode` |
| 47.0 | 0.0 | -47.0 | `bacnet_encoding::constructed::tagged::decode_ctx_primitive` |
| 42.0 | 0.0 | -42.0 | `bacnet_encoding::constructed::tagged::decode_ctx_fixed` |
| 31.0 | 0.0 | -31.0 | `bacnet_encoding::constructed::tagged::decode_ctx_unsigned::<u32>` |
| 27.0 | 0.0 | -27.0 | `bacnet_encoding::constructed::tagged::decode_ctx_object_id` |
| 18.0 | 0.0 | -18.0 | `bacnet_encoding::constructed::tagged::expect_end` |
| 14.0 | 0.0 | -14.0 | `bacnet_encoding::constructed::tagged::next_is_context` |
| 9.0 | 10.0 | 1.0 | `bacnet_encoding::primitives::decode_unsigned` |
| 0.0 | 0.0 | 0.0 | `<core::fmt::Formatter>::pad_integral` |
| 0.0 | 0.0 | 0.0 | `_dl_sysdep_parse_arguments` |
| 0.0 | 0.0 | 0.0 | `write` |
| 0.0 | 0.0 | 0.0 | `strdup` |
| 0.0 | 0.0 | 0.0 | `_int_free` |
| 0.0 | 0.0 | 0.0 | `__rustc::__rust_no_alloc_shim_is_unstable_v2` |
| 0.0 | 0.0 | 0.0 | `__pthread_mutex_unlock_usercnt` |
| 0.0 | 0.0 | 0.0 | `_dl_important_hwcaps` |
| 0.0 | 0.0 | 0.0 | `0x000000000000cbd0` |
| 0.0 | 0.0 | 0.0 | `<bacnet_counts::AllocSnapshot>::now` |
| 0.0 | 0.0 | 0.0 | `sysmalloc` |
| 0.0 | 0.0 | 0.0 | `alloc_perturb` |
| 0.0 | 0.0 | 0.0 | `0x0000000000000590` |
| 0.0 | 0.0 | 0.0 | `getauxval` |
| 0.0 | 0.0 | 0.0 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::dealloc` |
| 0.0 | 0.0 | 0.0 | `strstr` |
| 0.0 | 0.0 | 0.0 | `_dl_start` |
| 0.0 | 0.0 | 0.0 | `_dl_add_to_slotinfo` |
| 0.0 | 0.0 | 0.0 | `__getauxval2` |

## Source lines (line tables; inlined code counts at its own line)

| A | B | B - A | File:line |
| --- | --- | --- | --- |
| 36.0 | 14.0 | -22.0 | `library/core/src/result.rs:2177` |
| 28.0 | 0.0 | -28.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:361` |
| 20.0 | 0.0 | -20.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:354` |
| 14.0 | 0.0 | -14.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:391` |
| 12.0 | 0.0 | -12.0 | `crates/bacnet-encoding/src/tags.rs:182` |
| 12.0 | 6.0 | -6.0 | `library/core/src/result.rs:0` |
| 12.0 | 0.0 | -12.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:373` |
| 12.0 | 0.0 | -12.0 | `crates/bacnet-encoding/src/tags.rs:80` |
| 11.0 | 0.0 | -11.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:377` |
| 11.0 | 0.0 | -11.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:399` |
| 10.0 | 0.0 | -10.0 | `crates/bacnet-encoding/src/tags.rs:311` |
| 10.0 | 14.0 | 4.0 | `crates/bacnet-encoding/src/tags.rs:0` |
| 10.0 | 0.0 | -10.0 | `crates/bacnet-encoding/src/tags.rs:301` |
| 9.0 | 0.0 | -9.0 | `crates/bacnet-services/src/read_property.rs:51` |
| 9.0 | 0.0 | -9.0 | `crates/bacnet-services/src/read_property.rs:66` |
| 9.0 | 0.0 | -9.0 | `crates/bacnet-services/src/read_property.rs:48` |
| 8.0 | 0.0 | -8.0 | `crates/bacnet-services/src/read_property.rs:73` |
| 8.0 | 0.0 | -8.0 | `crates/bacnet-services/src/read_property.rs:55` |
| 8.0 | 0.0 | -8.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:503` |
| 8.0 | 0.0 | -8.0 | `crates/bacnet-encoding/src/tags.rs:219` |
| 8.0 | 0.0 | -8.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:383` |
| 7.0 | 0.0 | -7.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:444` |
| 7.0 | 4.0 | -3.0 | `library/core/src/convert/num.rs:303` |
| 7.0 | 0.0 | -7.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:314` |
| 7.0 | 0.0 | -7.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:450` |
| 7.0 | 0.0 | -7.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:562` |
| 6.0 | 0.0 | -6.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:571` |
| 6.0 | 0.0 | -6.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:410` |
| 6.0 | 0.0 | -6.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:568` |
| 6.0 | 0.0 | -6.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:348` |
| 6.0 | 0.0 | -6.0 | `crates/bacnet-encoding/src/tags.rs:183` |
| 6.0 | 0.0 | -6.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:372` |
| 6.0 | 2.0 | -4.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:0` |
| 6.0 | 0.0 | -6.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:330` |
| 6.0 | 0.0 | -6.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:451` |
| 5.0 | 0.0 | -5.0 | `crates/bacnet-services/src/read_property.rs:68` |
| 5.0 | 0.0 | -5.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:385` |
| 4.0 | 0.0 | -4.0 | `crates/bacnet-encoding/src/tags.rs:13` |
| 4.0 | 0.0 | -4.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:345` |
| 4.0 | 0.0 | -4.0 | `crates/bacnet-encoding/src/tags.rs:204` |
| 4.0 | 0.0 | -4.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:452` |
| 4.0 | 2.0 | -2.0 | `library/core/src/result.rs:1183` |
| 4.0 | 19.0 | 15.0 | `/wt/benchmarks/counts/src/bin/codec_counts.rs:125` |
| 4.0 | 0.0 | -4.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:113` |
| 4.0 | 0.0 | -4.0 | `crates/bacnet-encoding/src/tags.rs:245` |
| 4.0 | 0.0 | -4.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:115` |
| 4.0 | 0.0 | -4.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:570` |
| 4.0 | 0.0 | -4.0 | `library/core/src/result.rs:2178` |
| 4.0 | 0.0 | -4.0 | `crates/bacnet-encoding/src/tags.rs:79` |
| 3.0 | 0.0 | -3.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:105` |
| 3.0 | 0.0 | -3.0 | `crates/bacnet-types/src/primitives.rs:155` |
| 3.0 | 0.0 | -3.0 | `crates/bacnet-encoding/src/primitives/mod.rs:71` |
| 3.0 | 0.0 | -3.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:104` |
| 3.0 | 0.0 | -3.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:315` |
| 3.0 | 0.0 | -3.0 | `library/core/src/result.rs:1182` |
| 2.0 | 0.0 | -2.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:569` |
| 2.0 | 2.0 | 0.0 | `library/core/src/slice/index.rs:90` |
| 2.0 | 0.0 | -2.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:316` |
| 2.0 | 0.0 | -2.0 | `crates/bacnet-encoding/src/tags.rs:199` |
| 2.0 | 0.0 | -2.0 | `crates/bacnet-encoding/src/tags.rs:190` |
