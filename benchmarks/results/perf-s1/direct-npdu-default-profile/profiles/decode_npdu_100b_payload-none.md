# decode_npdu_100b_payload / none: self instructions per iteration

Median slope over the ordinals; the second column is B.
Calls in counts calls from other functions (a recursion level calling itself isn't one).

## Functions (callgrind_annotate, hash suffixes stripped)

| A | B | B - A | Calls in (A) | Function |
| --- | --- | --- | --- | --- |
| 127.0 | 112.0 | -15.0 | 0.00 | `bacnet_encoding::npdu::decode_npdu` |
| 46.0 | 0.0 | -46.0 | 1.00 | `<bacnet_npdu::view::Npdu>::decode` |
| 21.0 | 21.0 | 0.0 | 1.00 | `bytes::bytes::promotable_even_clone` |
| 15.0 | 15.0 | 0.0 | 1.00 | `bytes::bytes::shared_drop` |
| 5.0 | 5.0 | 0.0 | 1.00 | `__aarch64_ldadd8_relax` |
| 5.0 | 5.0 | 0.0 | 1.00 | `__aarch64_ldadd8_rel` |
| 0.0 | 0.0 | 0.0 | 0.00 | `bacnet_counts::parse_n` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<u64 as core::fmt::Display>::fmt` |
| 0.0 | 0.0 | 0.0 | 0.00 | `core::str::converts::from_utf8` |
| 0.0 | 0.0 | 0.0 | 0.00 | `std::rt::lang_start_internal` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<core::io::write::default_write_fmt::Adapter<std::io::stdio::StderrLock> as core::fmt::Write>::write_str` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_debug_initialize` |
| 0.0 | 0.0 | 0.0 | 0.00 | `bytes::bytes::promotable_even_drop` |
| 0.0 | 0.0 | 0.0 | 0.00 | `getrlimit` |
| 0.0 | 0.0 | 0.0 | 0.00 | `pthread_self` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_IO_str_init_static_internal` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_IO_unsave_markers` |
| 0.0 | 0.0 | 0.0 | 0.00 | `getpagesize` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_IO_sputbackc` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x00000000000038a4` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<core::fmt::Formatter>::write_formatted_parts` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<std::env::Args as core::iter::traits::iterator::Iterator>::next` |
| 0.0 | 0.0 | 0.0 | 0.00 | `sigaction` |
| 0.0 | 0.0 | 0.0 | 0.00 | `codec_counts::run::{closure#7}` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__pthread_attr_extension` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__GI___tunables_init` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<bytes::bytes::Bytes as core::convert::From<alloc::vec::Vec<u8>>>::from` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_determine_tlsoffset` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_hwcaps_split` |
| 0.0 | 0.0 | 0.0 | 0.00 | `std::env::args` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_IO_doallocbuf` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__tls_init_tp` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<core::fmt::Formatter>::pad_integral` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_strtoul` |
| 0.0 | 0.0 | 0.0 | 0.00 | `strcmp` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_check_map_versions` |
| 0.0 | 0.0 | 0.0 | 0.00 | `bytes::bytes::shallow_clone_vec` |
| 0.0 | 0.0 | 0.0 | 0.00 | `cache_rpath` |
| 0.0 | 0.0 | 0.0 | 0.00 | `calloc` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_add_to_namespace_list` |
| 0.0 | 0.0 | 0.0 | 0.00 | `tunable_initialize` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<&str as core::fmt::Display>::fmt` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<std::sys::sync::once::futex::Once>::call` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__sigsetjmp` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_init` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<bacnet_counts::AllocSnapshot>::now` |
| 0.0 | 0.0 | 0.0 | 0.00 | `std::io::stdio::print_to_buffer_if_capture_used` |
| 0.0 | 0.0 | 0.0 | 0.00 | `getdelim` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<core::fmt::Formatter>::pad_integral::write_prefix` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__underflow` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x00000000048cca50` |
| 0.0 | 0.0 | 0.0 | 0.00 | `realloc` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_sysdep_start_cleanup` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_sysdep_start` |
| 0.0 | 0.0 | 0.0 | 0.00 | `getauxval` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__call_tls_dtors` |
| 0.0 | 0.0 | 0.0 | 0.00 | `ptmalloc_init.part.0` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x000000000000b840` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__aarch64_cas8_relax` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_tls_static_surplus_init` |

## Largest function changes, B - A

| A | B | B - A | Function |
| --- | --- | --- | --- |
| 46.0 | 0.0 | -46.0 | `<bacnet_npdu::view::Npdu>::decode` |
| 127.0 | 112.0 | -15.0 | `bacnet_encoding::npdu::decode_npdu` |
| 0.0 | 0.0 | 0.0 | `std::rt::lang_start_internal` |
| 0.0 | 0.0 | 0.0 | `<core::io::write::default_write_fmt::Adapter<std::io::stdio::StderrLock> as core::fmt::Write>::write_str` |
| 0.0 | 0.0 | 0.0 | `_dl_debug_initialize` |
| 0.0 | 0.0 | 0.0 | `bytes::bytes::promotable_even_drop` |
| 0.0 | 0.0 | 0.0 | `getrlimit` |
| 0.0 | 0.0 | 0.0 | `pthread_self` |
| 0.0 | 0.0 | 0.0 | `_IO_str_init_static_internal` |
| 0.0 | 0.0 | 0.0 | `core::str::converts::from_utf8` |
| 0.0 | 0.0 | 0.0 | `_IO_unsave_markers` |
| 0.0 | 0.0 | 0.0 | `getpagesize` |
| 0.0 | 0.0 | 0.0 | `_IO_sputbackc` |
| 0.0 | 0.0 | 0.0 | `0x00000000000038a4` |
| 0.0 | 0.0 | 0.0 | `<core::fmt::Formatter>::write_formatted_parts` |
| 0.0 | 0.0 | 0.0 | `<std::env::Args as core::iter::traits::iterator::Iterator>::next` |
| 0.0 | 0.0 | 0.0 | `sigaction` |
| 0.0 | 0.0 | 0.0 | `codec_counts::run::{closure#7}` |
| 0.0 | 0.0 | 0.0 | `__pthread_attr_extension` |
| 0.0 | 0.0 | 0.0 | `__GI___tunables_init` |
| 0.0 | 0.0 | 0.0 | `<bytes::bytes::Bytes as core::convert::From<alloc::vec::Vec<u8>>>::from` |
| 0.0 | 0.0 | 0.0 | `_dl_determine_tlsoffset` |
| 0.0 | 0.0 | 0.0 | `_dl_hwcaps_split` |
| 0.0 | 0.0 | 0.0 | `std::env::args` |
| 0.0 | 0.0 | 0.0 | `_IO_doallocbuf` |
| 0.0 | 0.0 | 0.0 | `__tls_init_tp` |
| 0.0 | 0.0 | 0.0 | `<core::fmt::Formatter>::pad_integral` |
| 0.0 | 0.0 | 0.0 | `_dl_strtoul` |
| 0.0 | 0.0 | 0.0 | `strcmp` |
| 0.0 | 0.0 | 0.0 | `_dl_check_map_versions` |

## Source lines (line tables; inlined code counts at its own line)

| A | B | B - A | File:line |
| --- | --- | --- | --- |
| 25.0 | 0.0 | -25.0 | `crates/bacnet-encoding/src/npdu.rs:107` |
| 15.0 | 0.0 | -15.0 | `crates/bacnet-encoding/src/npdu.rs:230` |
| 14.0 | 0.0 | -14.0 | `crates/bacnet-npdu/src/view.rs:293` |
| 9.0 | 0.0 | -9.0 | `crates/bacnet-encoding/src/npdu.rs:255` |
| 8.0 | 8.0 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:250` |
| 8.0 | 8.0 | 0.0 | `library/core/src/result.rs:1183` |
| 8.0 | 0.0 | -8.0 | `crates/bacnet-encoding/src/npdu.rs:256` |
| 8.0 | 8.0 | 0.0 | `crates/bacnet-encoding/src/npdu.rs:218` |
| 7.0 | 0.0 | -7.0 | `crates/bacnet-npdu/src/view.rs:0` |
| 7.0 | 0.0 | -7.0 | `crates/bacnet-encoding/src/npdu.rs:229` |
| 6.0 | 0.0 | -6.0 | `crates/bacnet-npdu/src/view.rs:217` |
| 6.0 | 6.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:677` |
| 5.0 | 5.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1448` |
| 5.0 | 0.0 | -5.0 | `crates/bacnet-npdu/src/view.rs:304` |
| 4.0 | 4.0 | 0.0 | `library/core/src/sync/atomic.rs:3995` |
| 4.0 | 4.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1195` |
| 4.0 | 4.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1459` |
| 4.0 | 4.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1184` |
| 4.0 | 4.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1450` |
| 4.0 | 4.0 | 0.0 | `library/core/src/ptr/mod.rs:847` |
| 4.0 | 0.0 | -4.0 | `crates/bacnet-npdu/src/view.rs:218` |
| 3.0 | 3.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:670` |
| 3.0 | 0.0 | -3.0 | `crates/bacnet-encoding/src/npdu.rs:252` |
| 3.0 | 3.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:645` |
| 3.0 | 3.0 | 0.0 | `library/core/src/sync/atomic.rs:4049` |
| 3.0 | 3.0 | 0.0 | `/wt/benchmarks/counts/src/bin/codec_counts.rs:154` |
| 3.0 | 3.0 | 0.0 | `library/core/src/result.rs:1182` |
| 3.0 | 0.0 | -3.0 | `library/core/src/option.rs:1162` |
| 3.0 | 3.0 | 0.0 | `crates/bacnet-encoding/src/npdu.rs:222` |
| 2.0 | 2.0 | 0.0 | `library/core/src/slice/index.rs:184` |
| 2.0 | 2.0 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:252` |
| 2.0 | 2.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1543` |
| 2.0 | 0.0 | -2.0 | `crates/bacnet-npdu/src/view.rs:221` |
| 2.0 | 0.0 | -2.0 | `library/core/src/slice/index.rs:544` |
| 2.0 | 0.0 | -2.0 | `crates/bacnet-encoding/src/npdu.rs:254` |
| 2.0 | 2.0 | 0.0 | `crates/bacnet-encoding/src/npdu.rs:221` |
| 2.0 | 2.0 | 0.0 | `library/core/src/sync/atomic.rs:4029` |
| 2.0 | 2.0 | 0.0 | `library/core/src/cell.rs:2437` |
| 2.0 | 2.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:695` |
| 2.0 | 0.0 | -2.0 | `library/core/src/option.rs:0` |
| 1.0 | 0.0 | -1.0 | `crates/bacnet-npdu/src/view.rs:228` |
| 1.0 | 1.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1455` |
| 1.0 | 0.0 | -1.0 | `crates/bacnet-npdu/src/view.rs:252` |
| 1.0 | 1.0 | 0.0 | `library/core/src/ptr/const_ptr.rs:871` |
| 1.0 | 1.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:658` |
| 1.0 | 1.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1188` |
| 1.0 | 1.0 | 0.0 | `crates/bacnet-npdu/src/priority.rs:51` |
| 1.0 | 1.0 | 0.0 | `library/core/src/cmp.rs:2144` |
| 1.0 | 0.0 | -1.0 | `crates/bacnet-npdu/src/view.rs:282` |
| 1.0 | 0.0 | -1.0 | `index.crates.io-1949cf8c6b5b557f/smallvec-1.15.2/src/lib.rs:0` |
| 1.0 | 0.0 | -1.0 | `crates/bacnet-npdu/src/view.rs:275` |
| 1.0 | 19.0 | 18.0 | `crates/bacnet-encoding/src/npdu.rs:0` |
| 1.0 | 0.0 | -1.0 | `library/core/src/slice/index.rs:90` |
| 1.0 | 1.0 | 0.0 | `library/core/src/iter/range.rs:1100` |
| 0.0 | 0.0 | 0.0 | `library/core/src/str/validations.rs:241` |
| 0.0 | 0.0 | 0.0 | `library/core/src/fmt/num.rs:248` |
| 0.0 | 0.0 | 0.0 | `library/core/src/num/mod.rs:1819` |
| 0.0 | 0.0 | 0.0 | `library/core/src/num/mod.rs:1801` |
| 0.0 | 0.0 | 0.0 | `library/core/src/num/uint_macros.rs:2689` |
| 0.0 | 0.0 | 0.0 | `library/core/src/num/mod.rs:1820` |
