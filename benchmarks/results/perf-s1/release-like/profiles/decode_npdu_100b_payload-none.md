# decode_npdu_100b_payload / none: self instructions per iteration

Median slope over the ordinals; the second column is B.
Calls in counts calls from other functions (a recursion level calling itself isn't one).

## Functions (callgrind_annotate, hash suffixes stripped)

| A | B | B - A | Calls in (A) | Function |
| --- | --- | --- | --- | --- |
| 128.0 | 126.0 | -2.0 | 0.00 | `bacnet_encoding::npdu::decode_npdu` |
| 30.0 | 15.0 | -15.0 | 2.00 | `bytes::bytes::shared_drop` |
| 21.0 | 21.0 | 0.0 | 1.00 | `bytes::bytes::promotable_even_clone` |
| 20.0 | 0.0 | -20.0 | 1.00 | `bytes::bytes::shared_clone` |
| 10.0 | 5.0 | -5.0 | 2.00 | `__aarch64_ldadd8_relax` |
| 10.0 | 5.0 | -5.0 | 2.00 | `__aarch64_ldadd8_rel` |
| 0.0 | 0.0 | 0.0 | 0.00 | `codec_counts::main` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<u64 as core::fmt::Display>::fmt` |
| 0.0 | 0.0 | 0.0 | 0.00 | `core::str::converts::from_utf8` |
| 0.0 | 0.0 | 0.0 | 0.00 | `do_tunable_update_val` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__getauxval2` |
| 0.0 | 0.0 | 0.0 | 0.00 | `bcmp` |
| 0.0 | 0.0 | 0.0 | 0.00 | `tcache_init.part.0` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x0000000000000590` |
| 0.0 | 0.0 | 0.0 | 0.00 | `codec_counts::run::{closure#7}` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_check_all_versions` |
| 0.0 | 0.0 | 0.0 | 0.00 | `poll` |
| 0.0 | 0.0 | 0.0 | 0.00 | `check_match` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_debug_initialize` |
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
| 0.0 | 0.0 | 0.0 | 0.00 | `bytes::bytes::shallow_clone_vec` |
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

## Largest function changes, B - A

| A | B | B - A | Function |
| --- | --- | --- | --- |
| 20.0 | 0.0 | -20.0 | `bytes::bytes::shared_clone` |
| 30.0 | 15.0 | -15.0 | `bytes::bytes::shared_drop` |
| 10.0 | 5.0 | -5.0 | `__aarch64_ldadd8_relax` |
| 10.0 | 5.0 | -5.0 | `__aarch64_ldadd8_rel` |
| 128.0 | 126.0 | -2.0 | `bacnet_encoding::npdu::decode_npdu` |
| 0.0 | 0.0 | 0.0 | `do_tunable_update_val` |
| 0.0 | 0.0 | 0.0 | `__getauxval2` |
| 0.0 | 0.0 | 0.0 | `bcmp` |
| 0.0 | 0.0 | 0.0 | `tcache_init.part.0` |
| 0.0 | 0.0 | 0.0 | `0x0000000000000590` |
| 0.0 | 0.0 | 0.0 | `codec_counts::run::{closure#7}` |
| 0.0 | 0.0 | 0.0 | `_dl_check_all_versions` |
| 0.0 | 0.0 | 0.0 | `poll` |
| 0.0 | 0.0 | 0.0 | `check_match` |
| 0.0 | 0.0 | 0.0 | `_dl_debug_initialize` |
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
| 0.0 | 0.0 | 0.0 | `memset` |

## Source lines (line tables; inlined code counts at its own line)

| A | B | B - A | File:line |
| --- | --- | --- | --- |
| 25.0 | 0.0 | -25.0 | `crates/bacnet-encoding/src/npdu.rs:323` |
| 17.0 | 6.0 | -11.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:677` |
| 16.0 | 8.0 | -8.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:250` |
| 11.0 | 3.0 | -8.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:670` |
| 10.0 | 5.0 | -5.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1448` |
| 8.0 | 0.0 | -8.0 | `crates/bacnet-encoding/src/npdu.rs:200` |
| 8.0 | 0.0 | -8.0 | `crates/bacnet-encoding/src/npdu.rs:334` |
| 8.0 | 8.0 | 0.0 | `library/core/src/result.rs:1183` |
| 8.0 | 4.0 | -4.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1459` |
| 8.0 | 4.0 | -4.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1450` |
| 7.0 | 0.0 | -7.0 | `crates/bacnet-encoding/src/npdu.rs:0` |
| 6.0 | 2.0 | -4.0 | `library/core/src/sync/atomic.rs:4029` |
| 6.0 | 3.0 | -3.0 | `library/core/src/sync/atomic.rs:4049` |
| 4.0 | 4.0 | 0.0 | `library/core/src/ptr/mod.rs:847` |
| 4.0 | 0.0 | -4.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1368` |
| 4.0 | 2.0 | -2.0 | `library/core/src/cell.rs:2437` |
| 4.0 | 4.0 | 0.0 | `library/core/src/sync/atomic.rs:3995` |
| 4.0 | 2.0 | -2.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:252` |
| 4.0 | 0.0 | -4.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1365` |
| 4.0 | 2.0 | -2.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1543` |
| 4.0 | 4.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1184` |
| 4.0 | 4.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1195` |
| 3.0 | 0.0 | -3.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:304` |
| 3.0 | 0.0 | -3.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:383` |
| 3.0 | 0.0 | -3.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:0` |
| 3.0 | 3.0 | 0.0 | `library/core/src/result.rs:1182` |
| 3.0 | 3.0 | 0.0 | `/wt/benchmarks/counts/src/bin/codec_counts.rs:154` |
| 3.0 | 1.0 | -2.0 | `library/core/src/ptr/const_ptr.rs:871` |
| 2.0 | 0.0 | -2.0 | `crates/bacnet-encoding/src/npdu.rs:212` |
| 2.0 | 1.0 | -1.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:658` |
| 2.0 | 0.0 | -2.0 | `crates/bacnet-encoding/src/npdu.rs:205` |
| 2.0 | 0.0 | -2.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/lib.rs:154` |
| 2.0 | 0.0 | -2.0 | `crates/bacnet-encoding/src/npdu.rs:206` |
| 2.0 | 1.0 | -1.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1455` |
| 2.0 | 0.0 | -2.0 | `crates/bacnet-encoding/src/npdu.rs:201` |
| 2.0 | 0.0 | -2.0 | `library/core/src/sync/atomic.rs:3994` |
| 1.0 | 3.0 | 2.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:645` |
| 1.0 | 0.0 | -1.0 | `crates/bacnet-encoding/src/npdu.rs:291` |
| 1.0 | 1.0 | 0.0 | `library/core/src/iter/range.rs:1100` |
| 1.0 | 0.0 | -1.0 | `crates/bacnet-encoding/src/npdu.rs:302` |
| 1.0 | 1.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1188` |
| 1.0 | 0.0 | -1.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/lib.rs:150` |
| 1.0 | 0.0 | -1.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:376` |
| 1.0 | 0.0 | -1.0 | `crates/bacnet-encoding/src/npdu.rs:233` |
| 1.0 | 0.0 | -1.0 | `crates/bacnet-encoding/src/npdu.rs:263` |
| 1.0 | 1.0 | 0.0 | `library/core/src/cmp.rs:2144` |
| 1.0 | 0.0 | -1.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/lib.rs:142` |
| 0.0 | 0.0 | 0.0 | `library/core/src/str/validations.rs:241` |
| 0.0 | 0.0 | 0.0 | `library/core/src/fmt/num.rs:248` |
| 0.0 | 0.0 | 0.0 | `library/core/src/num/mod.rs:1801` |
| 0.0 | 0.0 | 0.0 | `library/core/src/num/mod.rs:1819` |
| 0.0 | 0.0 | 0.0 | `library/core/src/num/uint_macros.rs:2689` |
| 0.0 | 0.0 | 0.0 | `library/core/src/str/validations.rs:242` |
| 0.0 | 0.0 | 0.0 | `library/core/src/mem/maybe_uninit.rs:575` |
| 0.0 | 0.0 | 0.0 | `library/core/src/num/mod.rs:1820` |
| 0.0 | 0.0 | 0.0 | `library/core/src/num/mod.rs:1822` |
| 0.0 | 0.0 | 0.0 | `library/core/src/fmt/num.rs:243` |
| 0.0 | 0.0 | 0.0 | `library/core/src/num/mod.rs:1821` |
| 0.0 | 0.0 | 0.0 | `./nptl/./nptl/libc-cleanup.c:29` |
| 0.0 | 0.0 | 0.0 | `./string/../sysdeps/aarch64/strchr.S:109` |
