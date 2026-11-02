# decode_read_property_request / none: self instructions per iteration

Median slope over the ordinals; the second column is A'.
Calls in counts calls from other functions (a recursion level calling itself isn't one).

## Functions (callgrind_annotate, hash suffixes stripped)

| A | B | B - A | Calls in (A) | Function |
| --- | --- | --- | --- | --- |
| 101.0 | 101.0 | 0.0 | 0.00 | `codec_counts::run::{closure#2}` |
| 48.0 | 48.0 | 0.0 | 2.00 | `bacnet_encoding::tags::decode_tag` |
| 10.0 | 10.0 | 0.0 | 1.00 | `bacnet_encoding::primitives::decode_unsigned` |
| 10.0 | 10.0 | 0.0 | 1.00 | `<bacnet_types::primitives::ObjectIdentifier>::decode` |
| 0.0 | 0.0 | 0.0 | 0.00 | `strcspn` |
| 0.0 | 0.0 | 0.0 | 0.00 | `bacnet_counts::parse_n` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<u64 as core::fmt::Display>::fmt` |
| 0.0 | 0.0 | 0.0 | 0.00 | `core::str::converts::from_utf8` |
| 0.0 | 0.0 | 0.0 | 0.00 | `std::io::stdio::_eprint` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_IO_file_fopen@@GLIBC_2.17` |
| 0.0 | 0.0 | 0.0 | 0.00 | `getpagesize` |
| 0.0 | 0.0 | 0.0 | 0.00 | `OUTLINED_FUNCTION_83` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_catch_error` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__ctype_init` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__call_tls_dtors` |
| 0.0 | 0.0 | 0.0 | 0.00 | `core::fmt::write` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_next_ld_env_entry` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<std::sync::once::Once>::call_once::<std::rt::cleanup::{closure#0}>::{closure#0}` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x0000000000003780` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<core::fmt::Formatter>::write_formatted_parts` |
| 0.0 | 0.0 | 0.0 | 0.00 | `sigaltstack` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__tls_init_tp` |
| 0.0 | 0.0 | 0.0 | 0.00 | `do_tunable_update_val` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_call_fini` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_protect_relro` |
| 0.0 | 0.0 | 0.0 | 0.00 | `sbrk` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x0000000000003758` |
| 0.0 | 0.0 | 0.0 | 0.00 | `check_stdfiles_vtables` |
| 0.0 | 0.0 | 0.0 | 0.00 | `access` |
| 0.0 | 0.0 | 0.0 | 0.00 | `bacnet_encoding::primitives::scalar::write_tagged::<&mut [u8]>` |
| 0.0 | 0.0 | 0.0 | 0.00 | `pthread_mutex_lock@@GLIBC_2.17` |
| 0.0 | 0.0 | 0.0 | 0.00 | `memmove` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<core::fmt::Formatter>::pad` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_sysdep_start_cleanup` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x000000000000cbd0` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__strtoul_internal` |
| 0.0 | 0.0 | 0.0 | 0.00 | `expand_dynamic_string_token` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x0000000004883190` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_IO_old_init` |
| 0.0 | 0.0 | 0.0 | 0.00 | `dl_main` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x00000000000036c0` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x0000000000000650` |
| 0.0 | 0.0 | 0.0 | 0.00 | `stat` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__vfscanf_internal` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_map_object_from_fd` |
| 0.0 | 0.0 | 0.0 | 0.00 | `sigaction` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x00000000000037c0` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__minimal_calloc` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x000000000000b890` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__rustc::__rust_dealloc` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_catch_exception` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_relocate_object` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x0000000000003830` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<str as core::fmt::Display>::fmt` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__GI___tunable_set_val` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x00000000000038c0` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_new_object` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<alloc::raw_vec::RawVecInner<_>>::reserve::do_reserve_and_handle::<alloc::alloc::Global>` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x000000000000cc08` |

## Largest function changes, B - A

| A | B | B - A | Function |
| --- | --- | --- | --- |
| 0.0 | 0.0 | 0.0 | `std::io::stdio::_eprint` |
| 0.0 | 0.0 | 0.0 | `_IO_file_fopen@@GLIBC_2.17` |
| 0.0 | 0.0 | 0.0 | `getpagesize` |
| 0.0 | 0.0 | 0.0 | `OUTLINED_FUNCTION_83` |
| 0.0 | 0.0 | 0.0 | `_dl_catch_error` |
| 0.0 | 0.0 | 0.0 | `__ctype_init` |
| 0.0 | 0.0 | 0.0 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 0.0 | 0.0 | 0.0 | `__call_tls_dtors` |
| 0.0 | 0.0 | 0.0 | `core::fmt::write` |
| 0.0 | 0.0 | 0.0 | `_dl_next_ld_env_entry` |
| 0.0 | 0.0 | 0.0 | `<std::sync::once::Once>::call_once::<std::rt::cleanup::{closure#0}>::{closure#0}` |
| 0.0 | 0.0 | 0.0 | `0x0000000000003780` |
| 0.0 | 0.0 | 0.0 | `<core::fmt::Formatter>::write_formatted_parts` |
| 0.0 | 0.0 | 0.0 | `sigaltstack` |
| 0.0 | 0.0 | 0.0 | `__tls_init_tp` |
| 0.0 | 0.0 | 0.0 | `do_tunable_update_val` |
| 0.0 | 0.0 | 0.0 | `_dl_call_fini` |
| 0.0 | 0.0 | 0.0 | `_dl_protect_relro` |
| 48.0 | 48.0 | 0.0 | `bacnet_encoding::tags::decode_tag` |
| 0.0 | 0.0 | 0.0 | `sbrk` |
| 0.0 | 0.0 | 0.0 | `0x0000000000003758` |
| 0.0 | 0.0 | 0.0 | `check_stdfiles_vtables` |
| 0.0 | 0.0 | 0.0 | `access` |
| 0.0 | 0.0 | 0.0 | `bacnet_encoding::primitives::scalar::write_tagged::<&mut [u8]>` |
| 0.0 | 0.0 | 0.0 | `pthread_mutex_lock@@GLIBC_2.17` |
| 0.0 | 0.0 | 0.0 | `memmove` |
| 0.0 | 0.0 | 0.0 | `<core::fmt::Formatter>::pad` |
| 0.0 | 0.0 | 0.0 | `_dl_sysdep_start_cleanup` |
| 0.0 | 0.0 | 0.0 | `0x000000000000cbd0` |
| 0.0 | 0.0 | 0.0 | `__strtoul_internal` |

## Source lines (line tables; inlined code counts at its own line)

| A | B | B - A | File:line |
| --- | --- | --- | --- |
| 19.0 | 19.0 | 0.0 | `/wt/benchmarks/counts/src/bin/codec_counts.rs:125` |
| 14.0 | 14.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:0` |
| 14.0 | 14.0 | 0.0 | `library/core/src/result.rs:2177` |
| 14.0 | 14.0 | 0.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:424` |
| 8.0 | 8.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:158` |
| 8.0 | 8.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:83` |
| 6.0 | 6.0 | 0.0 | `library/core/src/result.rs:0` |
| 6.0 | 6.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:246` |
| 4.0 | 4.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:142` |
| 4.0 | 4.0 | 0.0 | `library/core/src/convert/num.rs:303` |
| 4.0 | 4.0 | 0.0 | `library/core/src/hint.rs:491` |
| 4.0 | 4.0 | 0.0 | `/wt/benchmarks/counts/src/bin/codec_counts.rs:94` |
| 4.0 | 4.0 | 0.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:407` |
| 4.0 | 4.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:121` |
| 4.0 | 4.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:184` |
| 3.0 | 3.0 | 0.0 | `crates/bacnet-encoding/src/primitives/mod.rs:108` |
| 3.0 | 3.0 | 0.0 | `crates/bacnet-encoding/src/primitives/mod.rs:97` |
| 2.0 | 2.0 | 0.0 | `library/core/src/slice/iter/macros.rs:180` |
| 2.0 | 2.0 | 0.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:527` |
| 2.0 | 2.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:256` |
| 2.0 | 2.0 | 0.0 | `library/core/src/slice/index.rs:90` |
| 2.0 | 2.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:157` |
| 2.0 | 2.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:137` |
| 2.0 | 2.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:128` |
| 2.0 | 2.0 | 0.0 | `crates/bacnet-types/src/primitives.rs:150` |
| 2.0 | 2.0 | 0.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:592` |
| 2.0 | 2.0 | 0.0 | `crates/bacnet-types/src/primitives.rs:141` |
| 2.0 | 2.0 | 0.0 | `crates/bacnet-encoding/src/tags.rs:132` |
| 2.0 | 2.0 | 0.0 | `library/core/src/result.rs:1183` |
| 2.0 | 2.0 | 0.0 | `crates/bacnet-services/src/read_property.rs:0` |
| 2.0 | 2.0 | 0.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:0` |
| 2.0 | 2.0 | 0.0 | `crates/bacnet-services/src/read_property.rs:103` |
| 2.0 | 2.0 | 0.0 | `library/core/src/num/uint_macros.rs:1096` |
| 2.0 | 2.0 | 0.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:780` |
| 1.0 | 1.0 | 0.0 | `crates/bacnet-encoding/src/constructed/tagged.rs:760` |
| 1.0 | 1.0 | 0.0 | `library/core/src/cmp.rs:2144` |
| 1.0 | 1.0 | 0.0 | `crates/bacnet-types/src/primitives.rs:148` |
| 1.0 | 1.0 | 0.0 | `library/core/src/num/uint_macros.rs:743` |
| 1.0 | 1.0 | 0.0 | `crates/bacnet-encoding/src/primitives/mod.rs:105` |
| 1.0 | 1.0 | 0.0 | `crates/bacnet-types/src/primitives.rs:154` |
| 1.0 | 1.0 | 0.0 | `crates/bacnet-types/src/primitives.rs:0` |
| 1.0 | 1.0 | 0.0 | `library/core/src/iter/range.rs:1100` |
| 1.0 | 1.0 | 0.0 | `crates/bacnet-encoding/src/primitives/mod.rs:109` |
| 1.0 | 1.0 | 0.0 | `library/core/src/result.rs:965` |
| 1.0 | 1.0 | 0.0 | `crates/bacnet-types/src/primitives.rs:147` |
| 1.0 | 1.0 | 0.0 | `crates/bacnet-types/src/primitives.rs:149` |
| 0.0 | 0.0 | 0.0 | `./string/./string/strcspn.c:67` |
| 0.0 | 0.0 | 0.0 | `library/core/src/fmt/num.rs:248` |
| 0.0 | 0.0 | 0.0 | `library/core/src/str/validations.rs:241` |
| 0.0 | 0.0 | 0.0 | `./string/./string/strcspn.c:62` |
| 0.0 | 0.0 | 0.0 | `library/core/src/num/mod.rs:1819` |
| 0.0 | 0.0 | 0.0 | `./string/./string/strcspn.c:64` |
| 0.0 | 0.0 | 0.0 | `library/core/src/num/mod.rs:1801` |
| 0.0 | 0.0 | 0.0 | `./string/./string/strcspn.c:65` |
| 0.0 | 0.0 | 0.0 | `library/core/src/num/mod.rs:1820` |
| 0.0 | 0.0 | 0.0 | `library/core/src/num/mod.rs:1822` |
| 0.0 | 0.0 | 0.0 | `library/core/src/str/validations.rs:242` |
| 0.0 | 0.0 | 0.0 | `library/core/src/mem/maybe_uninit.rs:575` |
| 0.0 | 0.0 | 0.0 | `library/core/src/num/mod.rs:1821` |
| 0.0 | 0.0 | 0.0 | `library/core/src/fmt/num.rs:243` |
