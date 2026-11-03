# decode_npdu_100b_payload / none: self instructions per iteration

Median slope over the ordinals; the second column is B.
Calls in counts calls from other functions (a recursion level calling itself isn't one).

## Functions (callgrind_annotate, hash suffixes stripped)

| A | B | B - A | Calls in (A) | Function |
| --- | --- | --- | --- | --- |
| 126.0 | 108.0 | -18.0 | 0.00 | `bacnet_encoding::npdu::decode_npdu` |
| 21.0 | 21.0 | 0.0 | 1.00 | `bytes::bytes::promotable_even_clone` |
| 15.0 | 15.0 | 0.0 | 1.00 | `bytes::bytes::shared_drop` |
| 5.0 | 5.0 | 0.0 | 1.00 | `__aarch64_ldadd8_relax` |
| 5.0 | 5.0 | 0.0 | 1.00 | `__aarch64_ldadd8_rel` |
| 0.0 | 0.0 | 0.0 | 0.00 | `codec_counts::main` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<u64 as core::fmt::Display>::fmt` |
| 0.0 | 0.0 | 0.0 | 0.00 | `core::str::converts::from_utf8` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__internal_atexit` |
| 0.0 | 0.0 | 0.0 | 0.00 | `std::sys::backtrace::__rust_begin_short_backtrace::<fn(), ()>` |
| 0.0 | 0.0 | 0.0 | 0.00 | `pthread_attr_setaffinity_np@@GLIBC_2.32` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_map_object_from_fd` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<core::fmt::Formatter>::write_formatted_parts` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x0000000000003660` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_sort_maps` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x0000000004883190` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__aarch64_cas4_acq` |
| 0.0 | 0.0 | 0.0 | 0.00 | `malloc` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x0000000000003780` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<core::fmt::Formatter>::pad` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__GI___close_nocancel` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__fopen_internal` |
| 0.0 | 0.0 | 0.0 | 0.00 | `calloc` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_tunable_set_top_pad` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<std::sys::stdio::unix::Stderr as core::io::write::Write>::write_all` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_add_to_namespace_list` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<core::fmt::Formatter>::pad_formatted_parts` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 0.0 | 0.0 | 0.0 | 0.00 | `dfs_traversal.part.0` |
| 0.0 | 0.0 | 0.0 | 0.00 | `std::io::stdio::_eprint` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x0000000000009050` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<std::sync::once_lock::OnceLock<std::sync::reentrant_lock::ReentrantLock<core::cell::RefCell<alloc::io::buffered::linewriter::LineWriter<std::io::stdio::StdoutR` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x00000000000038c0` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__init_cpu_features_constructor` |
| 0.0 | 0.0 | 0.0 | 0.00 | `handle_preload_list` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_check_all_versions` |
| 0.0 | 0.0 | 0.0 | 0.00 | `write` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__GI_memset` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x00000000000090c0` |
| 0.0 | 0.0 | 0.0 | 0.00 | `check_stdfiles_vtables` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x00000000000038a4` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_find_object_init` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_IO_file_open` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__strtoul_internal` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__read_nocancel` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x00000000048504f8` |
| 0.0 | 0.0 | 0.0 | 0.00 | `std::sys::pal::unix::stack_overflow::imp::make_handler` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<&str as core::fmt::Display>::fmt` |
| 0.0 | 0.0 | 0.0 | 0.00 | `expand_dynamic_string_token` |
| 0.0 | 0.0 | 0.0 | 0.00 | `tcache_init.part.0` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__rtld_malloc_init_stubs` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x0000000000003700` |
| 0.0 | 0.0 | 0.0 | 0.00 | `strdup` |
| 0.0 | 0.0 | 0.0 | 0.00 | `sigaltstack` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<std::sync::once::Once>::call_once::<std::rt::cleanup::{closure#0}>::{closure#0}` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x0000000000000634` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_bti_check` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__open_nocancel` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__glibc_morecore` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<core::fmt::Formatter>::pad_integral` |

## Largest function changes, B - A

| A | B | B - A | Function |
| --- | --- | --- | --- |
| 126.0 | 108.0 | -18.0 | `bacnet_encoding::npdu::decode_npdu` |
| 0.0 | 0.0 | 0.0 | `__internal_atexit` |
| 0.0 | 0.0 | 0.0 | `std::sys::backtrace::__rust_begin_short_backtrace::<fn(), ()>` |
| 0.0 | 0.0 | 0.0 | `pthread_attr_setaffinity_np@@GLIBC_2.32` |
| 0.0 | 0.0 | 0.0 | `_dl_map_object_from_fd` |
| 0.0 | 0.0 | 0.0 | `<core::fmt::Formatter>::write_formatted_parts` |
| 0.0 | 0.0 | 0.0 | `0x0000000000003660` |
| 0.0 | 0.0 | 0.0 | `_dl_sort_maps` |
| 0.0 | 0.0 | 0.0 | `0x0000000004883190` |
| 0.0 | 0.0 | 0.0 | `__aarch64_cas4_acq` |
| 0.0 | 0.0 | 0.0 | `malloc` |
| 0.0 | 0.0 | 0.0 | `0x0000000000003780` |
| 0.0 | 0.0 | 0.0 | `<core::fmt::Formatter>::pad` |
| 0.0 | 0.0 | 0.0 | `__GI___close_nocancel` |
| 0.0 | 0.0 | 0.0 | `__fopen_internal` |
| 0.0 | 0.0 | 0.0 | `calloc` |
| 0.0 | 0.0 | 0.0 | `_dl_tunable_set_top_pad` |
| 0.0 | 0.0 | 0.0 | `<std::sys::stdio::unix::Stderr as core::io::write::Write>::write_all` |
| 0.0 | 0.0 | 0.0 | `_dl_add_to_namespace_list` |
| 0.0 | 0.0 | 0.0 | `<core::fmt::Formatter>::pad_formatted_parts` |
| 0.0 | 0.0 | 0.0 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 0.0 | 0.0 | 0.0 | `dfs_traversal.part.0` |
| 0.0 | 0.0 | 0.0 | `std::io::stdio::_eprint` |
| 0.0 | 0.0 | 0.0 | `0x0000000000009050` |
| 0.0 | 0.0 | 0.0 | `<std::sync::once_lock::OnceLock<std::sync::reentrant_lock::ReentrantLock<core::cell::RefCell<alloc::io::buffered::linewriter::LineWriter<std::io::stdio::StdoutR` |
| 0.0 | 0.0 | 0.0 | `0x00000000000038c0` |
| 0.0 | 0.0 | 0.0 | `__init_cpu_features_constructor` |
| 0.0 | 0.0 | 0.0 | `handle_preload_list` |
| 0.0 | 0.0 | 0.0 | `_dl_check_all_versions` |
| 0.0 | 0.0 | 0.0 | `write` |

## Source lines (line tables; inlined code counts at its own line)

| A | B | B - A | File:line |
| --- | --- | --- | --- |
| 28.0 | 0.0 | -28.0 | `crates/bacnet-encoding/src/npdu.rs:107` |
| 12.0 | 0.0 | -12.0 | `crates/bacnet-encoding/src/npdu.rs:255` |
| 8.0 | 0.0 | -8.0 | `crates/bacnet-npdu/src/view.rs:0` |
| 8.0 | 8.0 | 0.0 | `library/core/src/result.rs:1183` |
| 8.0 | 0.0 | -8.0 | `crates/bacnet-encoding/src/npdu.rs:256` |
| 8.0 | 8.0 | 0.0 | `crates/bacnet-encoding/src/npdu.rs:218` |
| 8.0 | 8.0 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:250` |
| 6.0 | 6.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:677` |
| 5.0 | 5.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1448` |
| 4.0 | 0.0 | -4.0 | `library/core/src/option.rs:1162` |
| 4.0 | 4.0 | 0.0 | `library/core/src/sync/atomic.rs:3995` |
| 4.0 | 4.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1459` |
| 4.0 | 0.0 | -4.0 | `library/core/src/slice/index.rs:544` |
| 4.0 | 4.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1195` |
| 4.0 | 4.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1450` |
| 4.0 | 4.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1184` |
| 4.0 | 4.0 | 0.0 | `library/core/src/ptr/mod.rs:847` |
| 3.0 | 0.0 | -3.0 | `library/core/src/slice/index.rs:0` |
| 3.0 | 3.0 | 0.0 | `library/core/src/sync/atomic.rs:4049` |
| 3.0 | 3.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:670` |
| 3.0 | 3.0 | 0.0 | `/wt/benchmarks/counts/src/bin/codec_counts.rs:154` |
| 3.0 | 3.0 | 0.0 | `library/core/src/result.rs:1182` |
| 3.0 | 3.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:645` |
| 3.0 | 0.0 | -3.0 | `crates/bacnet-encoding/src/npdu.rs:252` |
| 2.0 | 0.0 | -2.0 | `crates/bacnet-encoding/src/npdu.rs:254` |
| 2.0 | 2.0 | 0.0 | `library/core/src/sync/atomic.rs:4029` |
| 2.0 | 0.0 | -2.0 | `index.crates.io-1949cf8c6b5b557f/smallvec-1.15.2/src/lib.rs:0` |
| 2.0 | 2.0 | 0.0 | `library/core/src/slice/index.rs:184` |
| 2.0 | 2.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1543` |
| 2.0 | 0.0 | -2.0 | `crates/bacnet-npdu/src/view.rs:221` |
| 2.0 | 2.0 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:252` |
| 2.0 | 2.0 | 0.0 | `library/core/src/cell.rs:2437` |
| 1.0 | 0.0 | -1.0 | `crates/bacnet-npdu/src/view.rs:218` |
| 1.0 | 0.0 | -1.0 | `crates/bacnet-npdu/src/view.rs:228` |
| 1.0 | 1.0 | 0.0 | `library/core/src/ptr/const_ptr.rs:871` |
| 1.0 | 1.0 | 0.0 | `library/core/src/iter/range.rs:1100` |
| 1.0 | 0.0 | -1.0 | `crates/bacnet-npdu/src/view.rs:252` |
| 1.0 | 1.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1455` |
| 1.0 | 1.0 | 0.0 | `library/core/src/cmp.rs:2144` |
| 1.0 | 0.0 | -1.0 | `library/core/src/ptr/mod.rs:0` |
| 1.0 | 1.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:658` |
| 1.0 | 1.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1188` |
| 1.0 | 0.0 | -1.0 | `crates/bacnet-npdu/src/view.rs:282` |
| 1.0 | 0.0 | -1.0 | `crates/bacnet-npdu/src/view.rs:275` |
| 0.0 | 0.0 | 0.0 | `library/core/src/str/validations.rs:241` |
| 0.0 | 0.0 | 0.0 | `library/core/src/fmt/num.rs:248` |
| 0.0 | 0.0 | 0.0 | `library/core/src/num/mod.rs:1819` |
| 0.0 | 0.0 | 0.0 | `library/core/src/num/mod.rs:1801` |
| 0.0 | 0.0 | 0.0 | `library/core/src/fmt/num.rs:243` |
| 0.0 | 0.0 | 0.0 | `library/core/src/num/uint_macros.rs:2689` |
| 0.0 | 0.0 | 0.0 | `library/core/src/str/validations.rs:242` |
| 0.0 | 0.0 | 0.0 | `library/core/src/num/mod.rs:1821` |
| 0.0 | 0.0 | 0.0 | `library/core/src/num/mod.rs:1820` |
| 0.0 | 0.0 | 0.0 | `library/core/src/num/mod.rs:1822` |
| 0.0 | 0.0 | 0.0 | `library/core/src/mem/maybe_uninit.rs:575` |
| 0.0 | 0.0 | 0.0 | `./elf/./elf/get-dynamic-info.h:62` |
| 0.0 | 0.0 | 0.0 | `./libio/./libio/iofopen.c:75` |
| 0.0 | 0.0 | 0.0 | `./elf/./elf/dl-audit.c:78` |
| 0.0 | 0.0 | 0.0 | `./elf/./elf/dl-tunables.h:161` |
| 0.0 | 0.0 | 0.0 | `library/std/src/sys/pal/unix/stack_overflow.rs:0` |
