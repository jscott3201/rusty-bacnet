# decode_apdu_confirmed_request / none: self instructions per iteration

Median slope over the ordinals; B - A in a noise run is A' - A.

## Functions (callgrind_annotate, hash suffixes stripped)

| A | B | B - A | Calls in (A) | Function |
| --- | --- | --- | --- | --- |
| 112.0 | 112.0 | 0.0 | 4.00 | `bacnet_encoding::apdu::decode_apdu` |
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
| 0.0 | 0.0 | 0.0 | 0.00 | `strcspn` |
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
| 0.0 | 0.0 | 0.0 | 0.00 | `bytes::bytes::shallow_clone_vec` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__set_vma_name` |
| 0.0 | 0.0 | 0.0 | 0.00 | `pthread_mutex_unlock@@GLIBC_2.17` |

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
| 19.0 | 19.0 | 0.0 | `library/core/src/result.rs:837` |
| 16.0 | 16.0 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:250` |
| 11.0 | 11.0 | 0.0 | `library/core/src/ptr/mod.rs:847` |
| 10.0 | 10.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:677` |
| 10.0 | 10.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1448` |
| 9.0 | 9.0 | 0.0 | `crates/bacnet-encoding/src/apdu/mod.rs:0` |
| 8.0 | 8.0 | 0.0 | `crates/bacnet-encoding/src/apdu/mod.rs:572` |
| 8.0 | 8.0 | 0.0 | `crates/bacnet-encoding/src/apdu/mod.rs:539` |
| 8.0 | 8.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1459` |
| 8.0 | 8.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1450` |
| 6.0 | 6.0 | 0.0 | `library/core/src/sync/atomic.rs:4049` |
| 6.0 | 6.0 | 0.0 | `library/core/src/sync/atomic.rs:4029` |
| 5.0 | 5.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:373` |
| 5.0 | 5.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:670` |
| 5.0 | 5.0 | 0.0 | `crates/bacnet-encoding/src/apdu/mod.rs:547` |
| 5.0 | 5.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:386` |
| 4.0 | 4.0 | 0.0 | `library/core/src/sync/atomic.rs:3995` |
| 4.0 | 4.0 | 0.0 | `library/core/src/result.rs:1183` |
| 4.0 | 4.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1368` |
| 4.0 | 4.0 | 0.0 | `library/core/src/cell.rs:2437` |
| 4.0 | 4.0 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:252` |
| 4.0 | 4.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1543` |
| 4.0 | 4.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1365` |
| 4.0 | 4.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1195` |
| 4.0 | 4.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1184` |
| 3.0 | 3.0 | 0.0 | `/wt/benchmarks/counts/src/bin/codec_counts.rs:180` |
| 3.0 | 3.0 | 0.0 | `crates/bacnet-encoding/src/apdu/mod.rs:616` |
| 3.0 | 3.0 | 0.0 | `crates/bacnet-encoding/src/apdu/mod.rs:618` |
| 3.0 | 3.0 | 0.0 | `crates/bacnet-encoding/src/apdu/mod.rs:613` |
| 3.0 | 3.0 | 0.0 | `library/core/src/result.rs:1182` |
| 3.0 | 3.0 | 0.0 | `crates/bacnet-encoding/src/apdu/header.rs:167` |
| 2.0 | 2.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:304` |
| 2.0 | 2.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:319` |
| 2.0 | 2.0 | 0.0 | `crates/bacnet-encoding/src/apdu/mod.rs:548` |
| 2.0 | 2.0 | 0.0 | `crates/bacnet-encoding/src/apdu/header.rs:153` |
| 2.0 | 2.0 | 0.0 | `crates/bacnet-encoding/src/apdu/mod.rs:607` |
| 2.0 | 2.0 | 0.0 | `library/core/src/ptr/mod.rs:0` |
| 2.0 | 2.0 | 0.0 | `library/core/src/sync/atomic.rs:3994` |
| 2.0 | 2.0 | 0.0 | `crates/bacnet-encoding/src/apdu/mod.rs:576` |
| 2.0 | 2.0 | 0.0 | `crates/bacnet-encoding/src/apdu/mod.rs:589` |
| 2.0 | 2.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/lib.rs:154` |
| 2.0 | 2.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/lib.rs:0` |
| 2.0 | 2.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1455` |
| 2.0 | 2.0 | 0.0 | `crates/bacnet-encoding/src/apdu/header.rs:92` |
| 2.0 | 2.0 | 0.0 | `crates/bacnet-encoding/src/apdu/mod.rs:544` |
| 2.0 | 2.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:383` |
| 1.0 | 1.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:376` |
| 1.0 | 1.0 | 0.0 | `crates/bacnet-encoding/src/apdu/mod.rs:592` |
| 1.0 | 1.0 | 0.0 | `crates/bacnet-encoding/src/apdu/mod.rs:614` |
| 1.0 | 1.0 | 0.0 | `library/core/src/cmp.rs:2144` |
| 1.0 | 1.0 | 0.0 | `crates/bacnet-encoding/src/apdu/mod.rs:585` |
| 1.0 | 1.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/lib.rs:150` |
| 1.0 | 1.0 | 0.0 | `crates/bacnet-encoding/src/apdu/mod.rs:587` |
| 1.0 | 1.0 | 0.0 | `library/core/src/ptr/const_ptr.rs:871` |
| 1.0 | 1.0 | 0.0 | `crates/bacnet-encoding/src/apdu/header.rs:0` |
| 1.0 | 1.0 | 0.0 | `crates/bacnet-encoding/src/apdu/mod.rs:586` |
| 1.0 | 1.0 | 0.0 | `crates/bacnet-encoding/src/apdu/mod.rs:540` |
| 1.0 | 1.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/lib.rs:142` |
| 1.0 | 1.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:645` |
| 1.0 | 1.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:658` |
