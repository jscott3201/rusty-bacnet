# full_stack_encode_rp / none: self instructions per iteration

Median slope over the ordinals; the second column is B.
Calls in counts calls from other functions (a recursion level calling itself isn't one).

## Functions (callgrind_annotate, hash suffixes stripped)

| A | B | B - A | Calls in (A) | Function |
| --- | --- | --- | --- | --- |
| 325.0 | 325.0 | 0.0 | 5.00 | `_int_free` |
| 270.0 | 270.0 | 0.0 | 5.00 | `malloc` |
| 254.0 | 287.0 | 33.0 | 14.00 | `__GI_memcpy` |
| 188.0 | 188.0 | 0.0 | 5.00 | `free` |
| 171.0 | 171.0 | 0.0 | 1.00 | `<bytes::bytes_mut::BytesMut as bytes::buf::buf_mut::BufMut>::put_slice` |
| 147.0 | 147.0 | 0.0 | 2.00 | `<&mut [u8] as bacnet_encoding::sink::EncodeSink>::write` |
| 130.0 | 130.0 | 0.0 | 5.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 94.0 | 141.0 | 47.0 | 1.00 | `<bacnet_npdu::view::Npdu>::encode_into` |
| 88.0 | 88.0 | 0.0 | 3.00 | `codec_counts::run::{closure#12}` |
| 76.0 | 76.0 | 0.0 | 3.00 | `bacnet_services::read_property::encode_reference` |
| 69.0 | 69.0 | 0.0 | 1.00 | `bacnet_encoding::apdu::encode_apdu` |
| 52.0 | 53.0 | 1.0 | 1.00 | `bacnet_encoding::npdu::encode_npdu` |
| 50.0 | 50.0 | 0.0 | 10.00 | `__aarch64_ldadd8_relax` |
| 49.0 | 49.0 | 0.0 | 1.00 | `bacnet_encoding::primitives::scalar::write_tagged::<&mut [u8]>` |
| 46.0 | 46.0 | 0.0 | 2.00 | `<bytes::bytes::Bytes as core::convert::From<alloc::vec::Vec<u8>>>::from` |
| 45.0 | 45.0 | 0.0 | 3.00 | `<bytes::bytes_mut::BytesMut as core::ops::drop::Drop>::drop` |
| 37.0 | 0.0 | -37.0 | 1.00 | `<bytes::bytes_mut::BytesMut>::resize` |
| 34.0 | 34.0 | 0.0 | 1.00 | `<bacnet_services::read_property::ReadPropertyRequest>::encode` |
| 30.0 | 30.0 | 0.0 | 2.00 | `bytes::bytes::promotable_even_drop` |
| 30.0 | 30.0 | 0.0 | 5.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::dealloc` |
| 25.0 | 25.0 | 0.0 | 5.00 | `__rustc::__rust_dealloc` |
| 24.0 | 24.0 | 0.0 | 1.00 | `bacnet_services::who_is::staged::<bytes::bytes_mut::BytesMut, <bacnet_services::read_property::ReadPropertyRequest>::encode_to_sink<bytes::bytes_mut::BytesMut>:` |
| 22.0 | 0.0 | -22.0 | 1.00 | `<bacnet_npdu::view::Npdu>::encoded_len` |
| 12.0 | 0.0 | -12.0 | 1.00 | `__GI_memset` |
| 5.0 | 5.0 | 0.0 | 5.00 | `__rustc::__rust_no_alloc_shim_is_unstable_v2` |
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

## Largest function changes, B - A

| A | B | B - A | Function |
| --- | --- | --- | --- |
| 94.0 | 141.0 | 47.0 | `<bacnet_npdu::view::Npdu>::encode_into` |
| 37.0 | 0.0 | -37.0 | `<bytes::bytes_mut::BytesMut>::resize` |
| 254.0 | 287.0 | 33.0 | `__GI_memcpy` |
| 22.0 | 0.0 | -22.0 | `<bacnet_npdu::view::Npdu>::encoded_len` |
| 12.0 | 0.0 | -12.0 | `__GI_memset` |
| 52.0 | 53.0 | 1.0 | `bacnet_encoding::npdu::encode_npdu` |
| 0.0 | 0.0 | 0.0 | `<str as core::fmt::Display>::fmt` |
| 25.0 | 25.0 | 0.0 | `__rustc::__rust_dealloc` |
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
| 189.0 | 228.0 | 39.0 | `library/core/src/ptr/mod.rs:573` |
| 60.0 | 60.0 | 0.0 | `./malloc/./malloc/malloc.c:4518` |
| 55.0 | 55.0 | 0.0 | `library/core/src/sync/atomic.rs:4029` |
| 52.0 | 52.0 | 0.0 | `/wt/benchmarks/counts/src/bin/codec_counts.rs:21` |
| 45.0 | 45.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:31` |
| 40.0 | 40.0 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:250` |
| 30.0 | 30.0 | 0.0 | `./malloc/./malloc/malloc.c:3354` |
| 30.0 | 30.0 | 0.0 | `./malloc/./malloc/malloc.c:4495` |
| 30.0 | 30.0 | 0.0 | `./malloc/./malloc/malloc.c:3294` |
| 30.0 | 30.0 | 0.0 | `./malloc/./malloc/malloc.c:4527` |
| 30.0 | 30.0 | 0.0 | `./malloc/./malloc/malloc.c:3313` |
| 25.0 | 25.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:65` |
| 25.0 | 25.0 | 0.0 | `./malloc/./malloc/malloc.c:3402` |
| 25.0 | 25.0 | 0.0 | `./malloc/./malloc/malloc.c:4658` |
| 25.0 | 25.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:1323` |
| 25.0 | 25.0 | 0.0 | `./malloc/./malloc/malloc.c:3311` |
| 25.0 | 25.0 | 0.0 | `./malloc/./malloc/malloc.c:4547` |
| 25.0 | 25.0 | 0.0 | `./malloc/./malloc/malloc.c:3392` |
| 25.0 | 25.0 | 0.0 | `./malloc/./malloc/malloc.c:3359` |
| 20.0 | 20.0 | 0.0 | `./malloc/./malloc/malloc.c:3163` |
| 20.0 | 20.0 | 0.0 | `/wt/benchmarks/counts/src/lib.rs:36` |
| 20.0 | 20.0 | 0.0 | `library/core/src/ptr/mod.rs:847` |
| 20.0 | 20.0 | 0.0 | `./malloc/./malloc/malloc.c:3301` |
| 20.0 | 20.0 | 0.0 | `./malloc/./malloc/malloc.c:3316` |
| 20.0 | 20.0 | 0.0 | `./malloc/./malloc/malloc.c:3371` |
| 20.0 | 20.0 | 0.0 | `./malloc/./malloc/malloc.c:4510` |
| 20.0 | 20.0 | 0.0 | `./malloc/./malloc/malloc.c:1336` |
| 20.0 | 20.0 | 0.0 | `./malloc/./malloc/arena.c:153` |
| 20.0 | 20.0 | 0.0 | `./malloc/./malloc/malloc.c:3368` |
| 20.0 | 20.0 | 0.0 | `./malloc/./malloc/malloc.c:4505` |
| 19.0 | 18.0 | -1.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:196` |
| 16.0 | 16.0 | 0.0 | `crates/bacnet-encoding/src/sink.rs:174` |
| 16.0 | 16.0 | 0.0 | `crates/bacnet-encoding/src/sink.rs:173` |
| 15.0 | 15.0 | 0.0 | `./malloc/./malloc/malloc.c:3398` |
| 15.0 | 15.0 | 0.0 | `./malloc/./malloc/malloc.c:3186` |
| 15.0 | 15.0 | 0.0 | `./malloc/./malloc/malloc.c:3395` |
| 15.0 | 15.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:993` |
| 15.0 | 15.0 | 0.0 | `./malloc/./malloc/malloc.c:1461` |
| 14.0 | 16.0 | 2.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:614` |
| 14.0 | 16.0 | 2.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:86` |
| 14.0 | 16.0 | 2.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:77` |
| 14.0 | 16.0 | 2.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:79` |
| 14.0 | 7.0 | -7.0 | `crates/bacnet-npdu/src/encode.rs:52` |
| 14.0 | 16.0 | 2.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:81` |
| 14.0 | 16.0 | 2.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:72` |
| 14.0 | 16.0 | 2.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:85` |
| 14.0 | 16.0 | 2.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:78` |
| 14.0 | 16.0 | 2.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:80` |
| 14.0 | 16.0 | 2.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:82` |
| 14.0 | 16.0 | 2.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:95` |
| 13.0 | 0.0 | -13.0 | `library/core/src/ptr/mod.rs:735` |
| 13.0 | 13.0 | 0.0 | `crates/bacnet-encoding/src/tags/encode.rs:151` |
| 13.0 | 13.0 | 0.0 | `crates/bacnet-encoding/src/tags/encode.rs:65` |
| 12.0 | 6.0 | -6.0 | `crates/bacnet-npdu/src/encode.rs:57` |
| 12.0 | 6.0 | -6.0 | `library/core/src/option.rs:1227` |
| 12.0 | 12.0 | 0.0 | `crates/bacnet-encoding/src/apdu/mod.rs:338` |
| 12.0 | 12.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/buf/buf_mut.rs:332` |
| 12.0 | 14.0 | 2.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:104` |
| 11.0 | 11.0 | 0.0 | `crates/bacnet-encoding/src/apdu/mod.rs:0` |
| 11.0 | 11.0 | 0.0 | `library/core/src/num/uint_macros.rs:1092` |
