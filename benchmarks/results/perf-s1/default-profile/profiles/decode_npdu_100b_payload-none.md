# decode_npdu_100b_payload / none: self instructions per iteration

Median slope over the ordinals; the second column is B.
Calls in counts calls from other functions (a recursion level calling itself isn't one).

## Functions (callgrind_annotate, hash suffixes stripped)

| A | B | B - A | Calls in (A) | Function |
| --- | --- | --- | --- | --- |
| 131.0 | 127.0 | -4.0 | 0.00 | `bacnet_encoding::npdu::decode_npdu` |
| 30.0 | 15.0 | -15.0 | 2.00 | `bytes::bytes::shared_drop` |
| 21.0 | 21.0 | 0.0 | 1.00 | `bytes::bytes::promotable_even_clone` |
| 20.0 | 0.0 | -20.0 | 1.00 | `bytes::bytes::shared_clone` |
| 10.0 | 5.0 | -5.0 | 2.00 | `__aarch64_ldadd8_rel` |
| 10.0 | 5.0 | -5.0 | 2.00 | `__aarch64_ldadd8_relax` |
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
| 0.0 | 0.0 | 0.0 | 0.00 | `<bytes::bytes::Bytes as core::convert::From<alloc::vec::Vec<u8>>>::from` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x0000000000003758` |
| 0.0 | 0.0 | 0.0 | 0.00 | `calloc` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_IO_link_in` |
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
| 0.0 | 0.0 | 0.0 | 0.00 | `memmove` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x0000000000009a80` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_name_match_p` |
| 0.0 | 0.0 | 0.0 | 0.00 | `std::sys::args::unix::imp::ARGV_INIT_ARRAY::init_wrapper` |

## Largest function changes, B - A

| A | B | B - A | Function |
| --- | --- | --- | --- |
| 0.0 | 46.0 | 46.0 | `<bacnet_npdu::view::Npdu>::decode` |
| 20.0 | 0.0 | -20.0 | `bytes::bytes::shared_clone` |
| 30.0 | 15.0 | -15.0 | `bytes::bytes::shared_drop` |
| 10.0 | 5.0 | -5.0 | `__aarch64_ldadd8_rel` |
| 10.0 | 5.0 | -5.0 | `__aarch64_ldadd8_relax` |
| 131.0 | 127.0 | -4.0 | `bacnet_encoding::npdu::decode_npdu` |
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
| 0.0 | 0.0 | 0.0 | `_dl_allocate_tls_storage` |
| 0.0 | 0.0 | 0.0 | `_dl_audit_preinit` |
| 0.0 | 0.0 | 0.0 | `__minimal_free` |
| 0.0 | 0.0 | 0.0 | `_dl_map_object_from_fd` |
| 0.0 | 0.0 | 0.0 | `_dl_sysdep_start` |

## Source lines (line tables; inlined code counts at its own line)

| A | B | B - A | File:line |
| --- | --- | --- | --- |
| 23.0 | 0.0 | -23.0 | `crates/bacnet-encoding/src/npdu.rs:323` |
| 17.0 | 6.0 | -11.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:677` |
| 16.0 | 8.0 | -8.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:250` |
| 12.0 | 3.0 | -9.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:670` |
| 10.0 | 5.0 | -5.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1448` |
| 8.0 | 8.0 | 0.0 | `library/core/src/result.rs:1183` |
| 8.0 | 4.0 | -4.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1459` |
| 8.0 | 4.0 | -4.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1450` |
| 8.0 | 1.0 | -7.0 | `crates/bacnet-encoding/src/npdu.rs:0` |
| 8.0 | 0.0 | -8.0 | `crates/bacnet-encoding/src/npdu.rs:200` |
| 8.0 | 0.0 | -8.0 | `crates/bacnet-encoding/src/npdu.rs:334` |
| 6.0 | 3.0 | -3.0 | `library/core/src/sync/atomic.rs:4049` |
| 6.0 | 2.0 | -4.0 | `library/core/src/sync/atomic.rs:4029` |
| 4.0 | 2.0 | -2.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1543` |
| 4.0 | 4.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1195` |
| 4.0 | 2.0 | -2.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:252` |
| 4.0 | 4.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1184` |
| 4.0 | 2.0 | -2.0 | `crates/bacnet-encoding/src/npdu.rs:221` |
| 4.0 | 1.0 | -3.0 | `library/core/src/ptr/const_ptr.rs:871` |
| 4.0 | 0.0 | -4.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1368` |
| 4.0 | 2.0 | -2.0 | `library/core/src/cell.rs:2437` |
| 4.0 | 4.0 | 0.0 | `library/core/src/ptr/mod.rs:847` |
| 4.0 | 4.0 | 0.0 | `library/core/src/sync/atomic.rs:3995` |
| 4.0 | 0.0 | -4.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1365` |
| 3.0 | 3.0 | 0.0 | `/wt/benchmarks/counts/src/bin/codec_counts.rs:154` |
| 3.0 | 0.0 | -3.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:304` |
| 3.0 | 3.0 | 0.0 | `library/core/src/result.rs:1182` |
| 2.0 | 1.0 | -1.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1455` |
| 2.0 | 0.0 | -2.0 | `crates/bacnet-encoding/src/npdu.rs:206` |
| 2.0 | 0.0 | -2.0 | `library/core/src/sync/atomic.rs:3994` |
| 2.0 | 0.0 | -2.0 | `crates/bacnet-encoding/src/npdu.rs:201` |
| 2.0 | 1.0 | -1.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:658` |
| 2.0 | 0.0 | -2.0 | `crates/bacnet-encoding/src/npdu.rs:212` |
| 2.0 | 0.0 | -2.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:383` |
| 2.0 | 0.0 | -2.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/lib.rs:154` |
| 2.0 | 0.0 | -2.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:0` |
| 2.0 | 0.0 | -2.0 | `crates/bacnet-encoding/src/npdu.rs:205` |
| 1.0 | 1.0 | 0.0 | `library/core/src/iter/range.rs:1100` |
| 1.0 | 1.0 | 0.0 | `library/core/src/cmp.rs:2144` |
| 1.0 | 0.0 | -1.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/lib.rs:142` |
| 1.0 | 0.0 | -1.0 | `crates/bacnet-encoding/src/npdu.rs:263` |
| 1.0 | 3.0 | 2.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:645` |
| 1.0 | 0.0 | -1.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/lib.rs:150` |
| 1.0 | 0.0 | -1.0 | `crates/bacnet-encoding/src/npdu.rs:291` |
| 1.0 | 1.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1188` |
| 1.0 | 0.0 | -1.0 | `crates/bacnet-encoding/src/npdu.rs:302` |
| 1.0 | 0.0 | -1.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:376` |
| 1.0 | 0.0 | -1.0 | `crates/bacnet-encoding/src/npdu.rs:233` |
| 0.0 | 0.0 | 0.0 | `library/core/src/str/validations.rs:241` |
| 0.0 | 0.0 | 0.0 | `library/core/src/fmt/num.rs:248` |
| 0.0 | 0.0 | 0.0 | `library/core/src/num/mod.rs:1819` |
| 0.0 | 0.0 | 0.0 | `library/core/src/num/mod.rs:1801` |
| 0.0 | 0.0 | 0.0 | `library/core/src/fmt/num.rs:243` |
| 0.0 | 0.0 | 0.0 | `library/core/src/mem/maybe_uninit.rs:575` |
| 0.0 | 0.0 | 0.0 | `library/core/src/num/mod.rs:1820` |
| 0.0 | 0.0 | 0.0 | `library/core/src/num/mod.rs:1822` |
| 0.0 | 0.0 | 0.0 | `library/core/src/str/validations.rs:242` |
| 0.0 | 0.0 | 0.0 | `library/core/src/num/uint_macros.rs:2689` |
| 0.0 | 0.0 | 0.0 | `library/core/src/num/mod.rs:1821` |
| 0.0 | 0.0 | 0.0 | `./elf/./elf/dl-version.c:337` |
