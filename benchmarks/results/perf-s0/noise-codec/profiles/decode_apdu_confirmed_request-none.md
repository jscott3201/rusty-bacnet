# decode_apdu_confirmed_request / none: self instructions per iteration

Median slope over the ordinals; the second column is A'.
Calls in counts calls from other functions (a recursion level calling itself isn't one).

## Functions (callgrind_annotate, hash suffixes stripped)

| A | B | B - A | Calls in (A) | Function |
| --- | --- | --- | --- | --- |
| 112.0 | 112.0 | 0.0 | 1.00 | `bacnet_encoding::apdu::decode_apdu` |
| 30.0 | 30.0 | 0.0 | 2.00 | `bytes::bytes::shared_drop` |
| 25.0 | 25.0 | 0.0 | 1.00 | `<bytes::bytes::Bytes>::slice::<core::ops::range::RangeFrom<usize>>` |
| 21.0 | 21.0 | 0.0 | 1.00 | `bytes::bytes::promotable_even_clone` |
| 20.0 | 20.0 | 0.0 | 1.00 | `bytes::bytes::shared_clone` |
| 11.0 | 11.0 | 0.0 | 1.00 | `codec_counts::main` |
| 10.0 | 10.0 | 0.0 | 2.00 | `__aarch64_ldadd8_relax` |
| 10.0 | 10.0 | 0.0 | 2.00 | `__aarch64_ldadd8_rel` |
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
| 0.0 | 0.0 | 0.0 | 0.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::realloc` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_protect_relro` |
| 0.0 | 0.0 | 0.0 | 0.00 | `sbrk` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x0000000000003758` |
| 0.0 | 0.0 | 0.0 | 0.00 | `check_stdfiles_vtables` |
| 0.0 | 0.0 | 0.0 | 0.00 | `access` |
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
| 0.0 | 0.0 | 0.0 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::realloc` |
| 0.0 | 0.0 | 0.0 | `_dl_protect_relro` |
| 0.0 | 0.0 | 0.0 | `sbrk` |
| 0.0 | 0.0 | 0.0 | `0x0000000000003758` |
| 0.0 | 0.0 | 0.0 | `check_stdfiles_vtables` |
| 0.0 | 0.0 | 0.0 | `access` |
| 0.0 | 0.0 | 0.0 | `pthread_mutex_lock@@GLIBC_2.17` |
| 0.0 | 0.0 | 0.0 | `memmove` |
| 0.0 | 0.0 | 0.0 | `<core::fmt::Formatter>::pad` |
| 0.0 | 0.0 | 0.0 | `_dl_sysdep_start_cleanup` |
| 0.0 | 0.0 | 0.0 | `0x000000000000cbd0` |
| 0.0 | 0.0 | 0.0 | `__strtoul_internal` |
| 0.0 | 0.0 | 0.0 | `expand_dynamic_string_token` |

## Source lines (line tables; inlined code counts at its own line)

| A | B | B - A | File:line |
| --- | --- | --- | --- |
| 19.0 | 19.0 | 0.0 | `library/core/src/result.rs:837` |
| 16.0 | 16.0 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:250` |
| 11.0 | 11.0 | 0.0 | `library/core/src/ptr/mod.rs:847` |
| 10.0 | 10.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:677` |
| 10.0 | 10.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1448` |
| 9.0 | 9.0 | 0.0 | `crates/bacnet-encoding/src/apdu/mod.rs:0` |
| 8.0 | 8.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1450` |
| 8.0 | 8.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1459` |
| 8.0 | 8.0 | 0.0 | `crates/bacnet-encoding/src/apdu/mod.rs:572` |
| 8.0 | 8.0 | 0.0 | `crates/bacnet-encoding/src/apdu/mod.rs:539` |
| 6.0 | 6.0 | 0.0 | `library/core/src/sync/atomic.rs:4049` |
| 6.0 | 6.0 | 0.0 | `library/core/src/sync/atomic.rs:4029` |
| 5.0 | 5.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:373` |
| 5.0 | 5.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:670` |
| 5.0 | 5.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:386` |
| 5.0 | 5.0 | 0.0 | `crates/bacnet-encoding/src/apdu/mod.rs:547` |
| 4.0 | 4.0 | 0.0 | `library/core/src/cell.rs:2437` |
| 4.0 | 4.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1365` |
| 4.0 | 4.0 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:252` |
| 4.0 | 4.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1543` |
| 4.0 | 4.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1195` |
| 4.0 | 4.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1184` |
| 4.0 | 4.0 | 0.0 | `library/core/src/sync/atomic.rs:3995` |
| 4.0 | 4.0 | 0.0 | `library/core/src/result.rs:1183` |
| 4.0 | 4.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1368` |
| 3.0 | 3.0 | 0.0 | `crates/bacnet-encoding/src/apdu/header.rs:167` |
| 3.0 | 3.0 | 0.0 | `library/core/src/result.rs:1182` |
| 3.0 | 3.0 | 0.0 | `/wt/benchmarks/counts/src/bin/codec_counts.rs:180` |
| 3.0 | 3.0 | 0.0 | `crates/bacnet-encoding/src/apdu/mod.rs:616` |
| 3.0 | 3.0 | 0.0 | `crates/bacnet-encoding/src/apdu/mod.rs:618` |
| 3.0 | 3.0 | 0.0 | `crates/bacnet-encoding/src/apdu/mod.rs:613` |
| 2.0 | 2.0 | 0.0 | `crates/bacnet-encoding/src/apdu/header.rs:92` |
| 2.0 | 2.0 | 0.0 | `library/core/src/sync/atomic.rs:3994` |
| 2.0 | 2.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1455` |
| 2.0 | 2.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/lib.rs:154` |
| 2.0 | 2.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/lib.rs:0` |
| 2.0 | 2.0 | 0.0 | `crates/bacnet-encoding/src/apdu/mod.rs:544` |
| 2.0 | 2.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:304` |
| 2.0 | 2.0 | 0.0 | `crates/bacnet-encoding/src/apdu/header.rs:153` |
| 2.0 | 2.0 | 0.0 | `crates/bacnet-encoding/src/apdu/mod.rs:548` |
| 2.0 | 2.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:383` |
| 2.0 | 2.0 | 0.0 | `crates/bacnet-encoding/src/apdu/mod.rs:607` |
| 2.0 | 2.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:319` |
| 2.0 | 2.0 | 0.0 | `library/core/src/ptr/mod.rs:0` |
| 2.0 | 2.0 | 0.0 | `crates/bacnet-encoding/src/apdu/mod.rs:576` |
| 2.0 | 2.0 | 0.0 | `crates/bacnet-encoding/src/apdu/mod.rs:589` |
| 1.0 | 1.0 | 0.0 | `crates/bacnet-encoding/src/apdu/header.rs:0` |
| 1.0 | 1.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/lib.rs:150` |
| 1.0 | 1.0 | 0.0 | `crates/bacnet-encoding/src/apdu/mod.rs:540` |
| 1.0 | 1.0 | 0.0 | `crates/bacnet-encoding/src/apdu/mod.rs:586` |
| 1.0 | 1.0 | 0.0 | `library/core/src/cmp.rs:2144` |
| 1.0 | 1.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:645` |
| 1.0 | 1.0 | 0.0 | `crates/bacnet-encoding/src/apdu/mod.rs:592` |
| 1.0 | 1.0 | 0.0 | `library/core/src/ptr/const_ptr.rs:871` |
| 1.0 | 1.0 | 0.0 | `crates/bacnet-encoding/src/apdu/mod.rs:614` |
| 1.0 | 1.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:658` |
| 1.0 | 1.0 | 0.0 | `library/core/src/iter/range.rs:1100` |
| 1.0 | 1.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1188` |
| 1.0 | 1.0 | 0.0 | `crates/bacnet-encoding/src/apdu/mod.rs:585` |
| 1.0 | 1.0 | 0.0 | `crates/bacnet-encoding/src/apdu/mod.rs:587` |
