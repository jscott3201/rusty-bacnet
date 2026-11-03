# decode_read_property_request / none: self instructions per iteration

Median slope over the ordinals; the second column is B.
Calls in counts calls from other functions (a recursion level calling itself isn't one).

## Functions (callgrind_annotate, hash suffixes stripped)

| A | B | B - A | Calls in (A) | Function |
| --- | --- | --- | --- | --- |
| 122.0 | 48.0 | -74.0 | 2.00 | `bacnet_encoding::tags::decode_tag` |
| 95.0 | 0.0 | -95.0 | 2.00 | `bacnet_encoding::constructed::tagged::ctx_header` |
| 47.0 | 61.0 | 14.0 | 2.00 | `bacnet_encoding::constructed::tagged::decode_ctx_unsigned::<u32>` |
| 45.0 | 0.0 | -45.0 | 2.00 | `<bacnet_services::read_property::ReadPropertyRequest>::decode` |
| 35.0 | 0.0 | -35.0 | 1.00 | `bacnet_encoding::constructed::tagged::decode_ctx_fixed` |
| 0.0 | 0.0 | 0.0 | 0.00 | `codec_counts::main` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<u64 as core::fmt::Display>::fmt` |
| 0.0 | 0.0 | 0.0 | 0.00 | `core::str::converts::from_utf8` |
| 0.0 | 0.0 | 0.0 | 0.00 | `do_tunable_update_val` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__getauxval2` |
| 0.0 | 0.0 | 0.0 | 0.00 | `bcmp` |
| 0.0 | 0.0 | 0.0 | 0.00 | `tcache_init.part.0` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x0000000000000590` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_check_all_versions` |
| 0.0 | 0.0 | 0.0 | 0.00 | `poll` |
| 0.0 | 0.0 | 0.0 | 0.00 | `check_match` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_debug_initialize` |
| 0.0 | 0.0 | 0.0 | 0.00 | `bacnet_encoding::primitives::encode_unsigned` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__GI_strlen` |
| 0.0 | 0.0 | 0.0 | 0.00 | `exit` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_IO_file_fopen@@GLIBC_2.17` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_IO_sputbackc` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_audit_activity_map` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__GI_memchr` |
| 0.0 | 0.0 | 0.0 | 0.00 | `fclose@@GLIBC_2.17` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x0000000000000600` |
| 0.0 | 0.0 | 0.0 | 0.00 | `std::io::stdio::_eprint` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_sysdep_start_cleanup` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<std::sync::once::Once>::call_once::<std::rt::cleanup::{closure#0}>::{closure#0}` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__GI_memset` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__init_cpu_features` |
| 0.0 | 0.0 | 0.0 | 0.00 | `memset` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_IO_file_close_it@@GLIBC_2.17` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x0000000000003758` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__GI_memcpy` |
| 0.0 | 0.0 | 0.0 | 0.00 | `pthread_getattr_np@@GLIBC_2.32` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__minimal_malloc` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__new_exitfn` |
| 0.0 | 0.0 | 0.0 | 0.00 | `malloc` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_IO_file_doallocate` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_audit_preinit` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_debug_state` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__minimal_calloc` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__read_nocancel` |
| 0.0 | 0.0 | 0.0 | 0.00 | `strcmp` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_allocate_tls_init` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__vfscanf_internal` |
| 0.0 | 0.0 | 0.0 | 0.00 | `open_verify.constprop.0` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_fixup` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x0000000000000634` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_int_free` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x000000000001b580` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__call_tls_dtors` |
| 0.0 | 0.0 | 0.0 | 0.00 | `getauxval` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_receive_error` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x0000000000008ed0` |
| 0.0 | 0.0 | 0.0 | 0.00 | `tunable_initialize` |
| 0.0 | 0.0 | 0.0 | 0.00 | `do_preload` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_init_first` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<core::fmt::Formatter>::pad_integral` |

## Largest function changes, B - A

| A | B | B - A | Function |
| --- | --- | --- | --- |
| 95.0 | 0.0 | -95.0 | `bacnet_encoding::constructed::tagged::ctx_header` |
| 122.0 | 48.0 | -74.0 | `bacnet_encoding::tags::decode_tag` |
| 0.0 | 71.0 | 71.0 | `codec_counts::run::{closure#2}` |
| 45.0 | 0.0 | -45.0 | `<bacnet_services::read_property::ReadPropertyRequest>::decode` |
| 35.0 | 0.0 | -35.0 | `bacnet_encoding::constructed::tagged::decode_ctx_fixed` |
| 47.0 | 61.0 | 14.0 | `bacnet_encoding::constructed::tagged::decode_ctx_unsigned::<u32>` |
| 0.0 | 0.0 | 0.0 | `do_tunable_update_val` |
| 0.0 | 0.0 | 0.0 | `__getauxval2` |
| 0.0 | 0.0 | 0.0 | `bcmp` |
| 0.0 | 0.0 | 0.0 | `tcache_init.part.0` |
| 0.0 | 0.0 | 0.0 | `0x0000000000000590` |
| 0.0 | 0.0 | 0.0 | `_dl_check_all_versions` |
| 0.0 | 0.0 | 0.0 | `poll` |
| 0.0 | 0.0 | 0.0 | `check_match` |
| 0.0 | 0.0 | 0.0 | `_dl_debug_initialize` |
| 0.0 | 0.0 | 0.0 | `bacnet_encoding::primitives::encode_unsigned` |
| 0.0 | 0.0 | 0.0 | `__GI_strlen` |
| 0.0 | 0.0 | 0.0 | `exit` |
| 0.0 | 0.0 | 0.0 | `_IO_file_fopen@@GLIBC_2.17` |
| 0.0 | 0.0 | 0.0 | `_IO_sputbackc` |
| 0.0 | 0.0 | 0.0 | `_dl_audit_activity_map` |
| 0.0 | 0.0 | 0.0 | `__GI_memchr` |
| 0.0 | 0.0 | 0.0 | `core::str::converts::from_utf8` |
| 0.0 | 0.0 | 0.0 | `fclose@@GLIBC_2.17` |
| 0.0 | 0.0 | 0.0 | `0x0000000000000600` |
| 0.0 | 0.0 | 0.0 | `std::io::stdio::_eprint` |
| 0.0 | 0.0 | 0.0 | `_dl_sysdep_start_cleanup` |
| 0.0 | 0.0 | 0.0 | `<std::sync::once::Once>::call_once::<std::rt::cleanup::{closure#0}>::{closure#0}` |
| 0.0 | 0.0 | 0.0 | `__GI_memset` |
| 0.0 | 0.0 | 0.0 | `__init_cpu_features` |

## Source lines (line tables; inlined code counts at its own line)

| A | B | B - A | File:line |
| --- | --- | --- | --- |
| 26.0 | 0.0 | -26.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:361` |
| 18.0 | 11.0 | -7.0 | `library/core/src/result.rs:2177` |
| 16.0 | 0.0 | -16.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:354` |
| 14.0 | 0.0 | -14.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:391` |
| 12.0 | 0.0 | -12.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:373` |
| 12.0 | 0.0 | -12.0 | `crates/bacnet-encoding/src/tags.rs:80` |
| 12.0 | 0.0 | -12.0 | `crates/bacnet-encoding/src/tags.rs:182` |
| 11.0 | 0.0 | -11.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:399` |
| 11.0 | 0.0 | -11.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:383` |
| 10.0 | 0.0 | -10.0 | `crates/bacnet-encoding/src/tags.rs:311` |
| 10.0 | 16.0 | 6.0 | `crates/bacnet-encoding/src/tags.rs:0` |
| 10.0 | 0.0 | -10.0 | `crates/bacnet-encoding/src/tags.rs:301` |
| 9.0 | 0.0 | -9.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:562` |
| 9.0 | 0.0 | -9.0 | `crates/bacnet-services/src/read_property.rs:48` |
| 8.0 | 0.0 | -8.0 | `crates/bacnet-services/src/read_property.rs:73` |
| 8.0 | 0.0 | -8.0 | `crates/bacnet-encoding/src/tags.rs:219` |
| 8.0 | 0.0 | -8.0 | `crates/bacnet-services/src/read_property.rs:55` |
| 8.0 | 4.0 | -4.0 | `library/core/src/convert/num.rs:303` |
| 7.0 | 0.0 | -7.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:450` |
| 7.0 | 0.0 | -7.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:571` |
| 7.0 | 0.0 | -7.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:345` |
| 6.0 | 0.0 | -6.0 | `crates/bacnet-services/src/read_property.rs:68` |
| 6.0 | 0.0 | -6.0 | `crates/bacnet-encoding/src/tags.rs:183` |
| 6.0 | 0.0 | -6.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:372` |
| 6.0 | 4.0 | -2.0 | `library/core/src/result.rs:0` |
| 5.0 | 0.0 | -5.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:410` |
| 5.0 | 0.0 | -5.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:570` |
| 5.0 | 2.0 | -3.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:0` |
| 4.0 | 0.0 | -4.0 | `crates/bacnet-services/src/lib.rs:0` |
| 4.0 | 0.0 | -4.0 | `crates/bacnet-encoding/src/tags.rs:245` |
| 4.0 | 1.0 | -3.0 | `library/core/src/result.rs:2178` |
| 4.0 | 13.0 | 9.0 | `/wt/benchmarks/counts/src/bin/codec_counts.rs:125` |
| 4.0 | 2.0 | -2.0 | `library/core/src/result.rs:1183` |
| 4.0 | 0.0 | -4.0 | `crates/bacnet-encoding/src/tags.rs:79` |
| 4.0 | 0.0 | -4.0 | `crates/bacnet-encoding/src/tags.rs:13` |
| 4.0 | 0.0 | -4.0 | `crates/bacnet-encoding/src/tags.rs:204` |
| 3.0 | 0.0 | -3.0 | `library/core/src/result.rs:1182` |
| 3.0 | 0.0 | -3.0 | `crates/bacnet-encoding/src/primitives/mod.rs:71` |
| 3.0 | 0.0 | -3.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:348` |
| 3.0 | 0.0 | -3.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:315` |
| 2.0 | 0.0 | -2.0 | `crates/bacnet-encoding/src/tags.rs:199` |
| 2.0 | 0.0 | -2.0 | `crates/bacnet-encoding/src/tags.rs:190` |
| 2.0 | 0.0 | -2.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:104` |
| 2.0 | 0.0 | -2.0 | `crates/bacnet-encoding/src/tags.rs:194` |
| 2.0 | 0.0 | -2.0 | `crates/bacnet-encoding/src/tags.rs:218` |
| 2.0 | 0.0 | -2.0 | `crates/bacnet-types/src/primitives.rs:154` |
| 2.0 | 0.0 | -2.0 | `crates/bacnet-types/src/primitives.rs:143` |
| 2.0 | 0.0 | -2.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:400` |
| 2.0 | 1.0 | -1.0 | `library/core/src/slice/iter/macros.rs:180` |
| 1.0 | 0.0 | -1.0 | `crates/bacnet-types/src/primitives.rs:153` |
| 1.0 | 1.0 | 0.0 | `library/core/src/iter/range.rs:1100` |
| 1.0 | 0.0 | -1.0 | `crates/bacnet-encoding/src/primitives/mod.rs:78` |
| 1.0 | 2.0 | 1.0 | `library/core/src/num/uint_macros.rs:1096` |
| 1.0 | 1.0 | 0.0 | `library/core/src/slice/index.rs:90` |
| 1.0 | 0.0 | -1.0 | `crates/bacnet-types/src/primitives.rs:152` |
| 1.0 | 0.0 | -1.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:550` |
| 1.0 | 1.0 | 0.0 | `library/core/src/cmp.rs:2144` |
| 1.0 | 1.0 | 0.0 | `library/core/src/result.rs:965` |
| 1.0 | 1.0 | 0.0 | `library/core/src/num/uint_macros.rs:743` |
| 0.0 | 0.0 | 0.0 | `library/core/src/str/validations.rs:241` |
