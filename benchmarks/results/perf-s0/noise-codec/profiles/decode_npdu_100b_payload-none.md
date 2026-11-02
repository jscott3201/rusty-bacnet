# decode_npdu_100b_payload / none: self instructions per iteration

Median slope over the ordinals; B - A in a noise run is A' - A.

## Functions (callgrind_annotate, hash suffixes stripped)

| A | B | B - A | Calls in (A) | Function |
| --- | --- | --- | --- | --- |
| 127.0 | 127.0 | 0.0 | 3.00 | `bacnet_encoding::npdu::decode_npdu` |
| 46.0 | 46.0 | 0.0 | 1.00 | `<bacnet_npdu::view::Npdu>::decode` |
| 21.0 | 21.0 | 0.0 | 1.00 | `bytes::bytes::promotable_even_clone` |
| 15.0 | 15.0 | 0.0 | 1.00 | `bytes::bytes::shared_drop` |
| 5.0 | 5.0 | 0.0 | 1.00 | `__aarch64_ldadd8_relax` |
| 5.0 | 5.0 | 0.0 | 1.00 | `__aarch64_ldadd8_rel` |
| 0.0 | 0.0 | 0.0 | 0.00 | `strcspn` |
| 0.0 | 0.0 | 0.0 | 0.00 | `bacnet_counts::parse_n` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<u64 as core::fmt::Display>::fmt` |
| 0.0 | 0.0 | 0.0 | 0.00 | `core::str::converts::from_utf8` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__call_tls_dtors` |
| 0.0 | 0.0 | 0.0 | 0.00 | `std::env::args` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__rtld_malloc_init_stubs` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_allocate_tls_init` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_count_modids` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__tls_pre_init_tp` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x0000000000003700` |
| 0.0 | 0.0 | 0.0 | 0.00 | `bytes::bytes::promotable_even_drop` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__unregister_atfork` |
| 0.0 | 0.0 | 0.0 | 0.00 | `do_preload` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_fini` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__strtoul_internal` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__aarch64_cas8_acq_rel` |
| 0.0 | 0.0 | 0.0 | 0.00 | `getrlimit` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__GI_memchr` |
| 0.0 | 0.0 | 0.0 | 0.00 | `do_lookup_x` |
| 0.0 | 0.0 | 0.0 | 0.00 | `check_stdfiles_vtables` |
| 0.0 | 0.0 | 0.0 | 0.00 | `do_tunable_update_val` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_new_object` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__GI___close_nocancel` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__rustc::__rust_no_alloc_shim_is_unstable_v2` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x00000000000037ec` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__pthread_tunables_init` |
| 0.0 | 0.0 | 0.0 | 0.00 | `map_doit` |
| 0.0 | 0.0 | 0.0 | 0.00 | `sigaltstack` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_cache_libcmp` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x000000000000cb60` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_determine_tlsoffset` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::dealloc` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x000000000000cbd0` |
| 0.0 | 0.0 | 0.0 | 0.00 | `getauxval` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__cxa_finalize` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_IO_file_read` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_IO_flush_all` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_IO_doallocbuf` |
| 0.0 | 0.0 | 0.0 | 0.00 | `exit` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_sort_maps` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__pthread_attr_extension` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_check_map_versions` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__open_nocancel` |
| 0.0 | 0.0 | 0.0 | 0.00 | `alloc_perturb` |
| 0.0 | 0.0 | 0.0 | 0.00 | `index` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_debug_update` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__GI__IO_un_link.part.0` |
| 0.0 | 0.0 | 0.0 | 0.00 | `ptmalloc_init.part.0` |
| 0.0 | 0.0 | 0.0 | 0.00 | `codec_counts::run::{closure#7}` |
| 0.0 | 0.0 | 0.0 | 0.00 | `bytes::bytes::shallow_clone_vec` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__set_vma_name` |
| 0.0 | 0.0 | 0.0 | 0.00 | `pthread_mutex_unlock@@GLIBC_2.17` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_IO_file_finish@@GLIBC_2.17` |

## Largest function changes, B - A

| A | B | B - A | Function |
| --- | --- | --- | --- |
| 0.0 | 0.0 | 0.0 | `__call_tls_dtors` |
| 0.0 | 0.0 | 0.0 | `std::env::args` |
| 0.0 | 0.0 | 0.0 | `__rtld_malloc_init_stubs` |
| 0.0 | 0.0 | 0.0 | `_dl_allocate_tls_init` |
| 0.0 | 0.0 | 0.0 | `_dl_count_modids` |
| 0.0 | 0.0 | 0.0 | `__tls_pre_init_tp` |
| 0.0 | 0.0 | 0.0 | `0x0000000000003700` |
| 0.0 | 0.0 | 0.0 | `bytes::bytes::promotable_even_drop` |
| 0.0 | 0.0 | 0.0 | `__unregister_atfork` |
| 0.0 | 0.0 | 0.0 | `do_preload` |
| 0.0 | 0.0 | 0.0 | `_dl_fini` |
| 0.0 | 0.0 | 0.0 | `__strtoul_internal` |
| 0.0 | 0.0 | 0.0 | `__aarch64_cas8_acq_rel` |
| 0.0 | 0.0 | 0.0 | `getrlimit` |
| 0.0 | 0.0 | 0.0 | `__GI_memchr` |
| 0.0 | 0.0 | 0.0 | `do_lookup_x` |
| 0.0 | 0.0 | 0.0 | `check_stdfiles_vtables` |
| 0.0 | 0.0 | 0.0 | `do_tunable_update_val` |
| 0.0 | 0.0 | 0.0 | `_dl_new_object` |
| 0.0 | 0.0 | 0.0 | `__GI___close_nocancel` |
| 0.0 | 0.0 | 0.0 | `__rustc::__rust_no_alloc_shim_is_unstable_v2` |
| 0.0 | 0.0 | 0.0 | `0x00000000000037ec` |
| 0.0 | 0.0 | 0.0 | `__pthread_tunables_init` |
| 0.0 | 0.0 | 0.0 | `map_doit` |
| 0.0 | 0.0 | 0.0 | `strcspn` |
| 0.0 | 0.0 | 0.0 | `sigaltstack` |
| 0.0 | 0.0 | 0.0 | `_dl_cache_libcmp` |
| 0.0 | 0.0 | 0.0 | `0x000000000000cb60` |
| 0.0 | 0.0 | 0.0 | `_dl_determine_tlsoffset` |
| 0.0 | 0.0 | 0.0 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::dealloc` |

## Source lines (line tables; inlined code counts at its own line)

| A | B | B - A | File:line |
| --- | --- | --- | --- |
| 25.0 | 25.0 | 0.0 | `crates/bacnet-encoding/src/npdu.rs:107` |
| 15.0 | 15.0 | 0.0 | `crates/bacnet-encoding/src/npdu.rs:230` |
| 14.0 | 14.0 | 0.0 | `crates/bacnet-npdu/src/view.rs:293` |
| 9.0 | 9.0 | 0.0 | `crates/bacnet-encoding/src/npdu.rs:255` |
| 8.0 | 8.0 | 0.0 | `library/core/src/result.rs:1183` |
| 8.0 | 8.0 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:250` |
| 8.0 | 8.0 | 0.0 | `crates/bacnet-encoding/src/npdu.rs:256` |
| 8.0 | 8.0 | 0.0 | `crates/bacnet-encoding/src/npdu.rs:218` |
| 7.0 | 7.0 | 0.0 | `crates/bacnet-npdu/src/view.rs:0` |
| 7.0 | 7.0 | 0.0 | `crates/bacnet-encoding/src/npdu.rs:229` |
| 6.0 | 6.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:677` |
| 6.0 | 6.0 | 0.0 | `crates/bacnet-npdu/src/view.rs:217` |
| 5.0 | 5.0 | 0.0 | `crates/bacnet-npdu/src/view.rs:304` |
| 5.0 | 5.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1448` |
| 4.0 | 4.0 | 0.0 | `library/core/src/sync/atomic.rs:3995` |
| 4.0 | 4.0 | 0.0 | `crates/bacnet-npdu/src/view.rs:218` |
| 4.0 | 4.0 | 0.0 | `library/core/src/ptr/mod.rs:847` |
| 4.0 | 4.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1195` |
| 4.0 | 4.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1459` |
| 4.0 | 4.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1184` |
| 4.0 | 4.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1450` |
| 3.0 | 3.0 | 0.0 | `/wt/benchmarks/counts/src/bin/codec_counts.rs:154` |
| 3.0 | 3.0 | 0.0 | `crates/bacnet-encoding/src/npdu.rs:252` |
| 3.0 | 3.0 | 0.0 | `library/core/src/option.rs:1162` |
| 3.0 | 3.0 | 0.0 | `library/core/src/sync/atomic.rs:4049` |
| 3.0 | 3.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:670` |
| 3.0 | 3.0 | 0.0 | `library/core/src/result.rs:1182` |
| 3.0 | 3.0 | 0.0 | `crates/bacnet-encoding/src/npdu.rs:222` |
| 3.0 | 3.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:645` |
| 2.0 | 2.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:695` |
| 2.0 | 2.0 | 0.0 | `library/core/src/slice/index.rs:544` |
| 2.0 | 2.0 | 0.0 | `library/core/src/cell.rs:2437` |
| 2.0 | 2.0 | 0.0 | `crates/bacnet-encoding/src/npdu.rs:254` |
| 2.0 | 2.0 | 0.0 | `crates/bacnet-encoding/src/npdu.rs:221` |
| 2.0 | 2.0 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:252` |
| 2.0 | 2.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1543` |
| 2.0 | 2.0 | 0.0 | `library/core/src/option.rs:0` |
| 2.0 | 2.0 | 0.0 | `library/core/src/sync/atomic.rs:4029` |
| 2.0 | 2.0 | 0.0 | `library/core/src/slice/index.rs:184` |
| 2.0 | 2.0 | 0.0 | `crates/bacnet-npdu/src/view.rs:221` |
| 1.0 | 1.0 | 0.0 | `crates/bacnet-npdu/src/view.rs:282` |
| 1.0 | 1.0 | 0.0 | `crates/bacnet-npdu/src/priority.rs:51` |
| 1.0 | 1.0 | 0.0 | `crates/bacnet-npdu/src/view.rs:275` |
| 1.0 | 1.0 | 0.0 | `library/core/src/cmp.rs:2144` |
| 1.0 | 1.0 | 0.0 | `library/core/src/slice/index.rs:90` |
| 1.0 | 1.0 | 0.0 | `crates/bacnet-npdu/src/view.rs:228` |
| 1.0 | 1.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1455` |
| 1.0 | 1.0 | 0.0 | `crates/bacnet-npdu/src/view.rs:252` |
| 1.0 | 1.0 | 0.0 | `crates/bacnet-encoding/src/npdu.rs:0` |
| 1.0 | 1.0 | 0.0 | `library/core/src/ptr/const_ptr.rs:871` |
| 1.0 | 1.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/smallvec-1.15.2/src/lib.rs:0` |
| 1.0 | 1.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:658` |
| 1.0 | 1.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1188` |
| 1.0 | 1.0 | 0.0 | `library/core/src/iter/range.rs:1100` |
| 0.0 | 0.0 | 0.0 | `./string/./string/strcspn.c:67` |
| 0.0 | 0.0 | 0.0 | `library/core/src/str/validations.rs:241` |
| 0.0 | 0.0 | 0.0 | `library/core/src/fmt/num.rs:248` |
| 0.0 | 0.0 | 0.0 | `./string/./string/strcspn.c:62` |
| 0.0 | 0.0 | 0.0 | `./string/./string/strcspn.c:64` |
| 0.0 | 0.0 | 0.0 | `library/core/src/num/mod.rs:1819` |
