# decode_npdu_100b_payload / none: self instructions per iteration

Median slope over the ordinals; the second column is B.
Calls in counts calls from other functions (a recursion level calling itself isn't one).

## Functions (callgrind_annotate, hash suffixes stripped)

| A | B | B - A | Calls in (A) | Function |
| --- | --- | --- | --- | --- |
| 126.0 | 107.0 | -19.0 | 0.00 | `bacnet_encoding::npdu::decode_npdu` |
| 21.0 | 21.0 | 0.0 | 1.00 | `bytes::bytes::promotable_even_clone` |
| 15.0 | 15.0 | 0.0 | 1.00 | `bytes::bytes::shared_drop` |
| 5.0 | 5.0 | 0.0 | 1.00 | `__aarch64_ldadd8_rel` |
| 5.0 | 5.0 | 0.0 | 1.00 | `__aarch64_ldadd8_relax` |
| 0.0 | 0.0 | 0.0 | 0.00 | `codec_counts::main` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<u64 as core::fmt::Display>::fmt` |
| 0.0 | 0.0 | 0.0 | 0.00 | `core::str::converts::from_utf8` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_IO_file_fopen@@GLIBC_2.17` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__GI_memcpy` |
| 0.0 | 0.0 | 0.0 | 0.00 | `main` |
| 0.0 | 0.0 | 0.0 | 0.00 | `sigaction` |
| 0.0 | 0.0 | 0.0 | 0.00 | `getrlimit` |
| 0.0 | 0.0 | 0.0 | 0.00 | `memset` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__run_exit_handlers` |
| 0.0 | 0.0 | 0.0 | 0.00 | `fillin_rpath.isra.0` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x00000000000038a4` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_relocate_object` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_next_ld_env_entry` |
| 0.0 | 0.0 | 0.0 | 0.00 | `pthread_getattr_np@@GLIBC_2.32` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_debug_initialize` |
| 0.0 | 0.0 | 0.0 | 0.00 | `std::sys::pal::unix::stack_overflow::thread_info::set_current_info` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_audit_activity_map` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_assign_tls_modid` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__minimal_free` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_audit_objopen` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__close_nocancel` |
| 0.0 | 0.0 | 0.0 | 0.00 | `tunable_initialize` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_IO_switch_to_get_mode` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__aarch64_swp4_rel` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_IO_str_init_static_internal` |
| 0.0 | 0.0 | 0.0 | 0.00 | `realloc` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x00000000000090c0` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_setjmp` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_add_to_namespace_list` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x0000000000009110` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__getauxval2` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_call_fini` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_init_first` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<std::sync::once::Once>::call_once_force::<<std::sync::once_lock::OnceLock<std::sync::reentrant_lock::ReentrantLock<core::cell::RefCell<alloc::io::buffered::lin` |
| 0.0 | 0.0 | 0.0 | 0.00 | `munmap` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dlfo_process_initial` |
| 0.0 | 0.0 | 0.0 | 0.00 | `getdelim` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x0000000000003660` |
| 0.0 | 0.0 | 0.0 | 0.00 | `malloc` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_start` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x0000000000003780` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__sigsetjmp` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_map_object` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_dst_count` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_IO_doallocbuf` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_protect_relro` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x00000000048504f8` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_IO_default_finish` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_IO_file_close` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_IO_sputbackc` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__GI___tunables_init` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x0000000000003700` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_sysdep_start` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x0000000004883190` |

## Largest function changes, B - A

| A | B | B - A | Function |
| --- | --- | --- | --- |
| 126.0 | 107.0 | -19.0 | `bacnet_encoding::npdu::decode_npdu` |
| 0.0 | 0.0 | 0.0 | `_IO_file_fopen@@GLIBC_2.17` |
| 0.0 | 0.0 | 0.0 | `__GI_memcpy` |
| 0.0 | 0.0 | 0.0 | `main` |
| 0.0 | 0.0 | 0.0 | `sigaction` |
| 0.0 | 0.0 | 0.0 | `getrlimit` |
| 0.0 | 0.0 | 0.0 | `memset` |
| 0.0 | 0.0 | 0.0 | `__run_exit_handlers` |
| 0.0 | 0.0 | 0.0 | `fillin_rpath.isra.0` |
| 0.0 | 0.0 | 0.0 | `0x00000000000038a4` |
| 0.0 | 0.0 | 0.0 | `_dl_relocate_object` |
| 0.0 | 0.0 | 0.0 | `_dl_next_ld_env_entry` |
| 0.0 | 0.0 | 0.0 | `pthread_getattr_np@@GLIBC_2.32` |
| 0.0 | 0.0 | 0.0 | `_dl_debug_initialize` |
| 0.0 | 0.0 | 0.0 | `std::sys::pal::unix::stack_overflow::thread_info::set_current_info` |
| 0.0 | 0.0 | 0.0 | `_dl_audit_activity_map` |
| 0.0 | 0.0 | 0.0 | `_dl_assign_tls_modid` |
| 0.0 | 0.0 | 0.0 | `__minimal_free` |
| 0.0 | 0.0 | 0.0 | `_dl_audit_objopen` |
| 0.0 | 0.0 | 0.0 | `0x0000000000009600` |
| 0.0 | 0.0 | 0.0 | `__close_nocancel` |
| 0.0 | 0.0 | 0.0 | `tunable_initialize` |
| 0.0 | 0.0 | 0.0 | `_IO_switch_to_get_mode` |
| 0.0 | 0.0 | 0.0 | `__aarch64_swp4_rel` |
| 0.0 | 0.0 | 0.0 | `_IO_str_init_static_internal` |
| 0.0 | 0.0 | 0.0 | `realloc` |
| 0.0 | 0.0 | 0.0 | `0x00000000000090c0` |
| 0.0 | 0.0 | 0.0 | `_setjmp` |
| 0.0 | 0.0 | 0.0 | `_dl_add_to_namespace_list` |
| 0.0 | 0.0 | 0.0 | `0x0000000000009110` |

## Source lines (line tables; inlined code counts at its own line)

| A | B | B - A | File:line |
| --- | --- | --- | --- |
| 28.0 | 0.0 | -28.0 | `crates/bacnet-encoding/src/npdu.rs:109` |
| 12.0 | 0.0 | -12.0 | `crates/bacnet-encoding/src/npdu.rs:257` |
| 8.0 | 8.0 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:250` |
| 8.0 | 0.0 | -8.0 | `crates/bacnet-encoding/src/npdu.rs:258` |
| 8.0 | 8.0 | 0.0 | `library/core/src/result.rs:1183` |
| 8.0 | 0.0 | -8.0 | `crates/bacnet-encoding/src/npdu.rs:220` |
| 8.0 | 0.0 | -8.0 | `crates/bacnet-npdu/src/view.rs:0` |
| 6.0 | 6.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:677` |
| 5.0 | 5.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1448` |
| 4.0 | 4.0 | 0.0 | `library/core/src/ptr/mod.rs:847` |
| 4.0 | 4.0 | 0.0 | `library/core/src/sync/atomic.rs:3995` |
| 4.0 | 4.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1459` |
| 4.0 | 4.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1184` |
| 4.0 | 0.0 | -4.0 | `library/core/src/slice/index.rs:544` |
| 4.0 | 4.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1195` |
| 4.0 | 4.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1450` |
| 4.0 | 0.0 | -4.0 | `library/core/src/option.rs:1162` |
| 3.0 | 3.0 | 0.0 | `library/core/src/result.rs:1182` |
| 3.0 | 3.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:670` |
| 3.0 | 0.0 | -3.0 | `crates/bacnet-encoding/src/npdu.rs:254` |
| 3.0 | 3.0 | 0.0 | `library/core/src/sync/atomic.rs:4049` |
| 3.0 | 4.0 | 1.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:645` |
| 3.0 | 3.0 | 0.0 | `/wt/benchmarks/counts/src/bin/codec_counts.rs:154` |
| 3.0 | 0.0 | -3.0 | `library/core/src/slice/index.rs:0` |
| 2.0 | 2.0 | 0.0 | `library/core/src/cell.rs:2437` |
| 2.0 | 2.0 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:252` |
| 2.0 | 2.0 | 0.0 | `library/core/src/slice/index.rs:184` |
| 2.0 | 0.0 | -2.0 | `crates/bacnet-npdu/src/view.rs:221` |
| 2.0 | 0.0 | -2.0 | `crates/bacnet-encoding/src/npdu.rs:256` |
| 2.0 | 2.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1543` |
| 2.0 | 2.0 | 0.0 | `library/core/src/sync/atomic.rs:4029` |
| 2.0 | 0.0 | -2.0 | `index.crates.io-1949cf8c6b5b557f/smallvec-1.15.2/src/lib.rs:0` |
| 1.0 | 0.0 | -1.0 | `crates/bacnet-npdu/src/view.rs:228` |
| 1.0 | 1.0 | 0.0 | `library/core/src/iter/range.rs:1100` |
| 1.0 | 0.0 | -1.0 | `library/core/src/ptr/mod.rs:0` |
| 1.0 | 0.0 | -1.0 | `crates/bacnet-npdu/src/view.rs:252` |
| 1.0 | 1.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1455` |
| 1.0 | 0.0 | -1.0 | `crates/bacnet-npdu/src/view.rs:282` |
| 1.0 | 0.0 | -1.0 | `crates/bacnet-npdu/src/view.rs:218` |
| 1.0 | 1.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:658` |
| 1.0 | 0.0 | -1.0 | `crates/bacnet-npdu/src/view.rs:275` |
| 1.0 | 1.0 | 0.0 | `library/core/src/ptr/const_ptr.rs:871` |
| 1.0 | 1.0 | 0.0 | `library/core/src/cmp.rs:2144` |
| 1.0 | 1.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1188` |
| 0.0 | 0.0 | 0.0 | `library/core/src/fmt/num.rs:248` |
| 0.0 | 0.0 | 0.0 | `library/core/src/str/validations.rs:241` |
| 0.0 | 0.0 | 0.0 | `library/core/src/num/mod.rs:1819` |
| 0.0 | 0.0 | 0.0 | `library/core/src/num/mod.rs:1801` |
| 0.0 | 0.0 | 0.0 | `library/core/src/num/mod.rs:1821` |
| 0.0 | 0.0 | 0.0 | `library/core/src/mem/maybe_uninit.rs:575` |
| 0.0 | 0.0 | 0.0 | `library/core/src/fmt/num.rs:243` |
| 0.0 | 0.0 | 0.0 | `library/core/src/num/mod.rs:1820` |
| 0.0 | 0.0 | 0.0 | `library/core/src/num/uint_macros.rs:2689` |
| 0.0 | 0.0 | 0.0 | `library/core/src/num/mod.rs:1822` |
| 0.0 | 0.0 | 0.0 | `library/core/src/str/validations.rs:242` |
| 0.0 | 0.0 | 0.0 | `./string/../sysdeps/aarch64/multiarch/../memchr.S:70` |
| 0.0 | 0.0 | 0.0 | `./libio/./libio/genops.c:944` |
| 0.0 | 0.0 | 0.0 | `./string/./string/strstr.c:86` |
| 0.0 | 0.0 | 0.0 | `./elf/./elf/dl-find_object.c:521` |
| 0.0 | 0.0 | 0.0 | `./elf/./elf/dl-lookup.c:816` |
