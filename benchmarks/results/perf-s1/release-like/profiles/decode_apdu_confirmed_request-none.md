# decode_apdu_confirmed_request / none: self instructions per iteration

Median slope over the ordinals; the second column is B.
Calls in counts calls from other functions (a recursion level calling itself isn't one).

## Functions (callgrind_annotate, hash suffixes stripped)

| A | B | B - A | Calls in (A) | Function |
| --- | --- | --- | --- | --- |
| 139.0 | 120.0 | -19.0 | 1.00 | `bacnet_encoding::apdu::decode_apdu` |
| 30.0 | 30.0 | 0.0 | 2.00 | `bytes::bytes::shared_drop` |
| 21.0 | 21.0 | 0.0 | 1.00 | `bytes::bytes::promotable_even_clone` |
| 20.0 | 20.0 | 0.0 | 1.00 | `bytes::bytes::shared_clone` |
| 11.0 | 0.0 | -11.0 | 1.00 | `codec_counts::run` |
| 10.0 | 10.0 | 0.0 | 2.00 | `__aarch64_ldadd8_relax` |
| 10.0 | 10.0 | 0.0 | 2.00 | `__aarch64_ldadd8_rel` |
| -0.0 | -0.0 | 0.0 | 0.00 | `rindex` |
| -0.0 | 0.0 | 0.0 | 0.00 | `____strtoul_l_internal` |
| 0.0 | 11.0 | 11.0 | 0.00 | `codec_counts::main` |
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

## Largest function changes, B - A

| A | B | B - A | Function |
| --- | --- | --- | --- |
| 139.0 | 120.0 | -19.0 | `bacnet_encoding::apdu::decode_apdu` |
| 0.0 | 11.0 | 11.0 | `codec_counts::main` |
| 11.0 | 0.0 | -11.0 | `codec_counts::run` |
| -0.0 | 0.0 | 0.0 | `____strtoul_l_internal` |
| 0.0 | 0.0 | 0.0 | `do_tunable_update_val` |
| 0.0 | 0.0 | 0.0 | `__getauxval2` |
| 0.0 | 0.0 | 0.0 | `bcmp` |
| 0.0 | 0.0 | 0.0 | `tcache_init.part.0` |
| 0.0 | 0.0 | 0.0 | `0x0000000000000590` |
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
| 0.0 | 0.0 | 0.0 | `_IO_file_close_it@@GLIBC_2.17` |
| 0.0 | 0.0 | 0.0 | `0x0000000000003758` |

## Source lines (line tables; inlined code counts at its own line)

| A | B | B - A | File:line |
| --- | --- | --- | --- |
| 21.0 | 19.0 | -2.0 | `library/core/src/result.rs:837` |
| 16.0 | 16.0 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:250` |
| 15.0 | 12.0 | -3.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:677` |
| 11.0 | 11.0 | 0.0 | `library/core/src/ptr/mod.rs:847` |
| 10.0 | 10.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1448` |
| 9.0 | 0.0 | -9.0 | `crates/bacnet-encoding/src/apdu/mod.rs:538` |
| 8.0 | 5.0 | -3.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:670` |
| 8.0 | 8.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1459` |
| 8.0 | 0.0 | -8.0 | `crates/bacnet-encoding/src/apdu/mod.rs:508` |
| 8.0 | 8.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1450` |
| 7.0 | 9.0 | 2.0 | `crates/bacnet-encoding/src/apdu/mod.rs:0` |
| 6.0 | 6.0 | 0.0 | `library/core/src/sync/atomic.rs:4029` |
| 6.0 | 0.0 | -6.0 | `crates/bacnet-encoding/src/apdu/mod.rs:583` |
| 6.0 | 6.0 | 0.0 | `library/core/src/sync/atomic.rs:4049` |
| 5.0 | 0.0 | -5.0 | `crates/bacnet-encoding/src/apdu/mod.rs:516` |
| 5.0 | 0.0 | -5.0 | `crates/bacnet-encoding/src/apdu/mod.rs:77` |
| 4.0 | 4.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1368` |
| 4.0 | 4.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:376` |
| 4.0 | 4.0 | 0.0 | `library/core/src/cell.rs:2437` |
| 4.0 | 4.0 | 0.0 | `library/core/src/sync/atomic.rs:3995` |
| 4.0 | 4.0 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:252` |
| 4.0 | 4.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1365` |
| 4.0 | 4.0 | 0.0 | `library/core/src/result.rs:1183` |
| 4.0 | 4.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1543` |
| 4.0 | 4.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1184` |
| 4.0 | 4.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1195` |
| 3.0 | 3.0 | 0.0 | `library/core/src/result.rs:1182` |
| 3.0 | 3.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:319` |
| 3.0 | 3.0 | 0.0 | `/wt/benchmarks/counts/src/bin/codec_counts.rs:180` |
| 3.0 | 0.0 | -3.0 | `crates/bacnet-encoding/src/apdu/mod.rs:513` |
| 3.0 | 0.0 | -3.0 | `library/core/src/result.rs:2177` |
| 2.0 | 1.0 | -1.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:645` |
| 2.0 | 0.0 | -2.0 | `crates/bacnet-encoding/src/apdu/mod.rs:578` |
| 2.0 | 2.0 | 0.0 | `library/core/src/ptr/mod.rs:0` |
| 2.0 | 0.0 | -2.0 | `crates/bacnet-encoding/src/apdu/mod.rs:166` |
| 2.0 | 2.0 | 0.0 | `library/core/src/ptr/const_ptr.rs:871` |
| 2.0 | 2.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1455` |
| 2.0 | 0.0 | -2.0 | `crates/bacnet-encoding/src/apdu/mod.rs:517` |
| 2.0 | 8.0 | 6.0 | `crates/bacnet-encoding/src/apdu/mod.rs:572` |
| 2.0 | 0.0 | -2.0 | `crates/bacnet-encoding/src/apdu/mod.rs:541` |
| 2.0 | 0.0 | -2.0 | `crates/bacnet-encoding/src/apdu/mod.rs:167` |
| 2.0 | 2.0 | 0.0 | `library/core/src/sync/atomic.rs:3994` |
| 1.0 | 0.0 | -1.0 | `crates/bacnet-encoding/src/apdu/mod.rs:545` |
| 1.0 | 1.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:304` |
| 1.0 | 0.0 | -1.0 | `crates/bacnet-encoding/src/apdu/mod.rs:554` |
| 1.0 | 1.0 | 0.0 | `library/core/src/iter/range.rs:1100` |
| 1.0 | 1.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:658` |
| 1.0 | 1.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1188` |
| 1.0 | 1.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:383` |
| 1.0 | 0.0 | -1.0 | `crates/bacnet-encoding/src/apdu/mod.rs:551` |
| 1.0 | 0.0 | -1.0 | `crates/bacnet-encoding/src/apdu/mod.rs:509` |
| 1.0 | 0.0 | -1.0 | `crates/bacnet-encoding/src/apdu/mod.rs:557` |
| 1.0 | 0.0 | -1.0 | `crates/bacnet-encoding/src/apdu/mod.rs:579` |
| 1.0 | 0.0 | -1.0 | `crates/bacnet-encoding/src/apdu/mod.rs:550` |
| 1.0 | 1.0 | 0.0 | `library/core/src/cmp.rs:2144` |
| 1.0 | 0.0 | -1.0 | `crates/bacnet-encoding/src/apdu/mod.rs:552` |
| 1.0 | 0.0 | -1.0 | `crates/bacnet-encoding/src/apdu/mod.rs:165` |
| -0.0 | 0.0 | 0.0 | `./stdlib/../stdlib/strtol_l.c:478` |
| 0.0 | 0.0 | 0.0 | `library/core/src/str/validations.rs:241` |
| 0.0 | 0.0 | 0.0 | `library/core/src/fmt/num.rs:248` |
