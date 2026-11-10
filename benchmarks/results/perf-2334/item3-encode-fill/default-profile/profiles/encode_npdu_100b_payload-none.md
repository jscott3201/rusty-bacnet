# encode_npdu_100b_payload / none: self instructions per iteration

Median slope over the ordinals; the second column is B.
Calls in counts calls from other functions (a recursion level calling itself isn't one).

## Functions (callgrind_annotate, hash suffixes stripped)

| A | B | B - A | Calls in (A) | Function |
| --- | --- | --- | --- | --- |
| 94.0 | 141.0 | 47.0 | 1.00 | `<bacnet_npdu::view::Npdu>::encode_into` |
| 65.0 | 65.0 | 0.0 | 1.00 | `_int_free` |
| 54.0 | 54.0 | 0.0 | 1.00 | `malloc` |
| 48.0 | 49.0 | 1.0 | 1.00 | `bacnet_encoding::npdu::encode_npdu` |
| 37.0 | 37.0 | 0.0 | 1.00 | `free` |
| 37.0 | 0.0 | -37.0 | 1.00 | `<bytes::bytes_mut::BytesMut>::resize` |
| 27.0 | 27.0 | 0.0 | 1.00 | `codec_counts::run::{closure#4}` |
| 26.0 | 26.0 | 0.0 | 1.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 22.0 | 0.0 | -22.0 | 1.00 | `<bacnet_npdu::view::Npdu>::encoded_len` |
| 21.0 | 0.0 | -21.0 | 1.00 | `__GI_memset` |
| 20.0 | 53.0 | 33.0 | 1.00 | `__GI_memcpy` |
| 15.0 | 15.0 | 0.0 | 1.00 | `<bytes::bytes_mut::BytesMut as core::ops::drop::Drop>::drop` |
| 10.0 | 10.0 | 0.0 | 2.00 | `__aarch64_ldadd8_relax` |
| 6.0 | 6.0 | 0.0 | 1.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::dealloc` |
| 5.0 | 5.0 | 0.0 | 1.00 | `__rustc::__rust_dealloc` |
| 1.0 | 1.0 | 0.0 | 1.00 | `__rustc::__rust_no_alloc_shim_is_unstable_v2` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<u64 as core::fmt::Display>::fmt` |
| 0.0 | 0.0 | 0.0 | 0.00 | `bacnet_counts::parse_n` |
| 0.0 | 0.0 | 0.0 | 0.00 | `core::str::converts::from_utf8` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<str as core::fmt::Display>::fmt` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<core::fmt::Formatter>::pad_integral` |
| 0.0 | 0.0 | 0.0 | 0.00 | `sigaction` |
| 0.0 | 0.0 | 0.0 | 0.00 | `do_lookup_x` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x0000000000000570` |
| 0.0 | 0.0 | 0.0 | 0.00 | `brk` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x000000000000bc50` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__rtld_mutex_init` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_IO_old_init` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_map_object_from_fd` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__sigsetjmp` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__libc_cleanup_push_defer` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__strlen_asimd` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_assign_tls_modid` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__GI_memchr` |
| 0.0 | 0.0 | 0.0 | 0.00 | `core::num::imp::flt2dec::strategy::dragon::format_exact` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<core::io::write::default_write_fmt::Adapter<std::io::stdio::StderrLock> as core::fmt::Write>::write_str` |
| 0.0 | 0.0 | 0.0 | 0.00 | `std::sys::pal::unix::stack_overflow::thread_info::set_current_info` |
| 0.0 | 0.0 | 0.0 | 0.00 | `memmove` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__GI__IO_un_link.part.0` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_audit_objopen` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_tunable_set_top_pad` |
| 0.0 | 0.0 | 0.0 | 0.00 | `strdup` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_hwcaps_split_masked` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_determine_tlsoffset` |
| 0.0 | 0.0 | 0.0 | 0.00 | `std::env::args_os` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_IO_new_file_init_internal` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__libc_start_main@@GLIBC_2.34` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_next_ld_env_entry` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__aarch64_cas8_acq` |
| 0.0 | 0.0 | 0.0 | 0.00 | `access` |
| 0.0 | 0.0 | 0.0 | 0.00 | `rindex` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__open_nocancel` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__minimal_calloc` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_count_modids` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__minimal_free` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__sigjmp_save` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__new_exitfn` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<core::fmt::Formatter>::pad_formatted_parts` |
| 0.0 | 0.0 | 0.0 | 0.00 | `strsep` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__close_nocancel` |

## Largest function changes, B - A

| A | B | B - A | Function |
| --- | --- | --- | --- |
| 94.0 | 141.0 | 47.0 | `<bacnet_npdu::view::Npdu>::encode_into` |
| 37.0 | 0.0 | -37.0 | `<bytes::bytes_mut::BytesMut>::resize` |
| 20.0 | 53.0 | 33.0 | `__GI_memcpy` |
| 22.0 | 0.0 | -22.0 | `<bacnet_npdu::view::Npdu>::encoded_len` |
| 21.0 | 0.0 | -21.0 | `__GI_memset` |
| 48.0 | 49.0 | 1.0 | `bacnet_encoding::npdu::encode_npdu` |
| 0.0 | 0.0 | 0.0 | `<str as core::fmt::Display>::fmt` |
| 5.0 | 5.0 | 0.0 | `__rustc::__rust_dealloc` |
| 0.0 | 0.0 | 0.0 | `<core::fmt::Formatter>::pad_integral` |
| 0.0 | 0.0 | 0.0 | `sigaction` |
| 0.0 | 0.0 | 0.0 | `do_lookup_x` |
| 0.0 | 0.0 | 0.0 | `0x0000000000000570` |
| 0.0 | 0.0 | 0.0 | `brk` |
| 0.0 | 0.0 | 0.0 | `0x000000000000bc50` |
| 0.0 | 0.0 | 0.0 | `__rtld_mutex_init` |
| 0.0 | 0.0 | 0.0 | `_IO_old_init` |
| 0.0 | 0.0 | 0.0 | `_dl_map_object_from_fd` |
| 0.0 | 0.0 | 0.0 | `__sigsetjmp` |
| 0.0 | 0.0 | 0.0 | `__libc_cleanup_push_defer` |
| 0.0 | 0.0 | 0.0 | `__strlen_asimd` |
| 0.0 | 0.0 | 0.0 | `_dl_assign_tls_modid` |
| 0.0 | 0.0 | 0.0 | `__GI_memchr` |
| 0.0 | 0.0 | 0.0 | `core::num::imp::flt2dec::strategy::dragon::format_exact` |
| 0.0 | 0.0 | 0.0 | `<core::io::write::default_write_fmt::Adapter<std::io::stdio::StderrLock> as core::fmt::Write>::write_str` |
| 0.0 | 0.0 | 0.0 | `std::sys::pal::unix::stack_overflow::thread_info::set_current_info` |
| 0.0 | 0.0 | 0.0 | `memmove` |
| 0.0 | 0.0 | 0.0 | `__GI__IO_un_link.part.0` |
| 0.0 | 0.0 | 0.0 | `_dl_audit_objopen` |
| 0.0 | 0.0 | 0.0 | `_dl_tunable_set_top_pad` |
| 0.0 | 0.0 | 0.0 | `strdup` |

## Source lines (line tables; inlined code counts at its own line)

| A | B | B - A | File:line |
| --- | --- | --- | --- |
| 14.0 | 7.0 | -7.0 | `crates/bacnet-npdu/src/encode.rs:52` |
| 13.0 | 0.0 | -13.0 | `library/core/src/ptr/mod.rs:735` |
| 12.0 | 6.0 | -6.0 | `crates/bacnet-npdu/src/encode.rs:57` |
| 12.0 | 6.0 | -6.0 | `library/core/src/option.rs:1227` |
| 12.0 | 51.0 | 39.0 | `library/core/src/ptr/mod.rs:573` |
| 12.0 | 12.0 | 0.0 | `./malloc/./malloc/malloc.c:4518` |
| 11.0 | 10.0 | -1.0 | `crates/bacnet-encoding/src/npdu.rs:125` |
| 11.0 | 11.0 | 0.0 | `library/core/src/sync/atomic.rs:4029` |
| 10.0 | 10.0 | 0.0 | `/wt/benchmarks/counts/src/bin/codec_counts.rs:21` |
| 9.0 | 9.0 | 0.0 | `library/core/src/num/uint_macros.rs:1092` |
| 9.0 | 9.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:31` |
| 8.0 | 8.0 | 0.0 | `crates/bacnet-npdu/src/encode.rs:70` |
| 8.0 | 8.0 | 0.0 | `library/core/src/option.rs:743` |
| 8.0 | 8.0 | 0.0 | `crates/bacnet-npdu/src/encode.rs:129` |
| 8.0 | 8.0 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:250` |
| 7.0 | 7.0 | 0.0 | `/wt/benchmarks/counts/src/bin/codec_counts.rs:139` |
| 6.0 | 6.0 | 0.0 | `./malloc/./malloc/malloc.c:3354` |
| 6.0 | 0.0 | -6.0 | `crates/bacnet-encoding/src/npdu.rs:167` |
| 6.0 | 6.0 | 0.0 | `./malloc/./malloc/malloc.c:4495` |
| 6.0 | 6.0 | 0.0 | `./malloc/./malloc/malloc.c:3294` |
| 6.0 | 0.0 | -6.0 | `crates/bacnet-encoding/src/npdu.rs:166` |
| 6.0 | 6.0 | 0.0 | `./malloc/./malloc/malloc.c:4527` |
| 6.0 | 6.0 | 0.0 | `./malloc/./malloc/malloc.c:3313` |
| 5.0 | 5.0 | 0.0 | `crates/bacnet-npdu/src/encode.rs:0` |
| 5.0 | 1.0 | -4.0 | `crates/bacnet-encoding/src/npdu.rs:174` |
| 5.0 | 5.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:65` |
| 5.0 | 5.0 | 0.0 | `crates/bacnet-encoding/src/npdu.rs:163` |
| 5.0 | 5.0 | 0.0 | `./malloc/./malloc/malloc.c:3402` |
| 5.0 | 5.0 | 0.0 | `./malloc/./malloc/malloc.c:4658` |
| 5.0 | 5.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:993` |
| 5.0 | 5.0 | 0.0 | `./malloc/./malloc/malloc.c:3311` |
| 5.0 | 5.0 | 0.0 | `./malloc/./malloc/malloc.c:4547` |
| 5.0 | 5.0 | 0.0 | `./malloc/./malloc/malloc.c:3392` |
| 5.0 | 5.0 | 0.0 | `./malloc/./malloc/malloc.c:3359` |
| 4.0 | 4.0 | 0.0 | `./malloc/./malloc/malloc.c:3163` |
| 4.0 | 4.0 | 0.0 | `/wt/benchmarks/counts/src/lib.rs:36` |
| 4.0 | 0.0 | -4.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:495` |
| 4.0 | 4.0 | 0.0 | `crates/bacnet-npdu/src/encode.rs:103` |
| 4.0 | 2.0 | -2.0 | `crates/bacnet-encoding/src/npdu.rs:0` |
| 4.0 | 4.0 | 0.0 | `./malloc/./malloc/malloc.c:3301` |
| 4.0 | 4.0 | 0.0 | `./malloc/./malloc/malloc.c:3316` |
| 4.0 | 4.0 | 0.0 | `./malloc/./malloc/malloc.c:3371` |
| 4.0 | 4.0 | 0.0 | `./malloc/./malloc/malloc.c:4510` |
| 4.0 | 4.0 | 0.0 | `./malloc/./malloc/malloc.c:1336` |
| 4.0 | 4.0 | 0.0 | `./malloc/./malloc/arena.c:153` |
| 4.0 | 4.0 | 0.0 | `/wt/benchmarks/counts/src/bin/codec_counts.rs:141` |
| 4.0 | 4.0 | 0.0 | `/wt/benchmarks/counts/src/bin/codec_counts.rs:143` |
| 4.0 | 4.0 | 0.0 | `./malloc/./malloc/malloc.c:3368` |
| 4.0 | 4.0 | 0.0 | `./malloc/./malloc/malloc.c:4505` |
| 3.0 | 3.0 | 0.0 | `/wt/benchmarks/counts/src/bin/codec_counts.rs:94` |
| 3.0 | 2.0 | -1.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:196` |
| 3.0 | 3.0 | 0.0 | `./malloc/./malloc/malloc.c:3398` |
| 3.0 | 3.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:0` |
| 3.0 | 3.0 | 0.0 | `library/core/src/result.rs:1182` |
| 3.0 | 3.0 | 0.0 | `./malloc/./malloc/malloc.c:3186` |
| 3.0 | 3.0 | 0.0 | `./malloc/./malloc/malloc.c:3395` |
| 3.0 | 3.0 | 0.0 | `crates/bacnet-npdu/src/encode.rs:100` |
| 3.0 | 3.0 | 0.0 | `crates/bacnet-npdu/src/encode.rs:106` |
| 3.0 | 3.0 | 0.0 | `./malloc/./malloc/malloc.c:1461` |
| 3.0 | 3.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:1217` |
