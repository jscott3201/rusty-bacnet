# full_stack_encode_rp / none: self instructions per iteration

Median slope over the ordinals; the second column is B.
Calls in counts calls from other functions (a recursion level calling itself isn't one).

## Functions (callgrind_annotate, hash suffixes stripped)

| A | B | B - A | Calls in (A) | Function |
| --- | --- | --- | --- | --- |
| 514.0 | 0.0 | -514.0 | 5.00 | `<bytes::bytes_mut::BytesMut as bytes::buf::buf_mut::BufMut>::put_slice` |
| 325.0 | 325.0 | 0.0 | 5.00 | `_int_free` |
| 270.0 | 270.0 | 0.0 | 5.00 | `malloc` |
| 258.0 | 118.0 | -140.0 | 14.00 | `__GI_memcpy` |
| 195.0 | 196.0 | 1.0 | 5.00 | `free` |
| 125.0 | 125.0 | 0.0 | 5.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 83.0 | 83.0 | 0.0 | 5.00 | `codec_counts::run::{closure#12}` |
| 78.0 | 126.0 | 48.0 | 1.00 | `bacnet_encoding::apdu::encode_apdu` |
| 50.0 | 50.0 | 0.0 | 10.00 | `__aarch64_ldadd8_relax` |
| 41.0 | 97.0 | 56.0 | 2.00 | `<bacnet_services::read_property::ReadPropertyRequest>::encode` |
| 28.0 | 28.0 | 0.0 | 2.00 | `bytes::bytes::promotable_even_drop` |
| 28.0 | 156.0 | 128.0 | 1.00 | `bacnet_encoding::npdu::encode_npdu` |
| 25.0 | 25.0 | 0.0 | 5.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::dealloc` |
| 17.0 | 0.0 | -17.0 | 1.00 | `bacnet_encoding::primitives::encode_unsigned` |
| 1.0 | 0.0 | -1.0 | 1.00 | `codec_counts::run` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<u64 as core::fmt::Display>::fmt` |
| 0.0 | 1.0 | 1.0 | 0.00 | `codec_counts::main` |
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
| 0.0 | 0.0 | 0.0 | 0.00 | `core::num::imp::flt2dec::strategy::dragon::mul_pow10` |
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
| 0.0 | 12.0 | 12.0 | 0.00 | `__GI_memset` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__init_cpu_features` |
| 0.0 | 0.0 | 0.0 | 0.00 | `memset` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_IO_file_close_it@@GLIBC_2.17` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x0000000000003758` |
| 0.0 | 0.0 | 0.0 | 0.00 | `pthread_getattr_np@@GLIBC_2.32` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__minimal_malloc` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__new_exitfn` |
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
| 0.0 | 0.0 | 0.0 | 0.00 | `0x000000000001b580` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__call_tls_dtors` |

## Largest function changes, B - A

| A | B | B - A | Function |
| --- | --- | --- | --- |
| 514.0 | 0.0 | -514.0 | `<bytes::bytes_mut::BytesMut as bytes::buf::buf_mut::BufMut>::put_slice` |
| 258.0 | 118.0 | -140.0 | `__GI_memcpy` |
| 28.0 | 156.0 | 128.0 | `bacnet_encoding::npdu::encode_npdu` |
| 41.0 | 97.0 | 56.0 | `<bacnet_services::read_property::ReadPropertyRequest>::encode` |
| 78.0 | 126.0 | 48.0 | `bacnet_encoding::apdu::encode_apdu` |
| 17.0 | 0.0 | -17.0 | `bacnet_encoding::primitives::encode_unsigned` |
| 0.0 | 12.0 | 12.0 | `__GI_memset` |
| 195.0 | 196.0 | 1.0 | `free` |
| 0.0 | 1.0 | 1.0 | `codec_counts::main` |
| 1.0 | 0.0 | -1.0 | `codec_counts::run` |
| 0.0 | 0.0 | 0.0 | `do_tunable_update_val` |
| 0.0 | 0.0 | 0.0 | `__getauxval2` |
| 0.0 | 0.0 | 0.0 | `bcmp` |
| 0.0 | 0.0 | 0.0 | `tcache_init.part.0` |
| 0.0 | 0.0 | 0.0 | `0x0000000000000590` |
| 0.0 | 0.0 | 0.0 | `_dl_check_all_versions` |
| 0.0 | 0.0 | 0.0 | `poll` |
| 0.0 | 0.0 | 0.0 | `check_match` |
| 0.0 | 0.0 | 0.0 | `_dl_debug_initialize` |
| 0.0 | 0.0 | 0.0 | `core::num::imp::flt2dec::strategy::dragon::mul_pow10` |
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

## Source lines (line tables; inlined code counts at its own line)

| A | B | B - A | File:line |
| --- | --- | --- | --- |
| 240.0 | 70.0 | -170.0 | `library/core/src/ptr/mod.rs:573` |
| 96.0 | 0.0 | -96.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:1323` |
| 60.0 | 60.0 | 0.0 | `./malloc/./malloc/malloc.c:4518` |
| 50.0 | 50.0 | 0.0 | `library/core/src/sync/atomic.rs:4029` |
| 45.0 | 45.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:31` |
| 40.0 | 40.0 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:250` |
| 36.0 | 15.0 | -21.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:614` |
| 36.0 | 2.0 | -34.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:196` |
| 30.0 | 30.0 | 0.0 | `./malloc/./malloc/malloc.c:4527` |
| 30.0 | 30.0 | 0.0 | `./malloc/./malloc/malloc.c:3313` |
| 30.0 | 30.0 | 0.0 | `./malloc/./malloc/malloc.c:3354` |
| 30.0 | 30.0 | 0.0 | `./malloc/./malloc/malloc.c:4495` |
| 30.0 | 30.0 | 0.0 | `./malloc/./malloc/malloc.c:3294` |
| 30.0 | 0.0 | -30.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/buf/buf_mut.rs:332` |
| 25.0 | 25.0 | 0.0 | `./malloc/./malloc/malloc.c:4547` |
| 25.0 | 25.0 | 0.0 | `./malloc/./malloc/malloc.c:3392` |
| 25.0 | 25.0 | 0.0 | `./malloc/./malloc/malloc.c:3359` |
| 25.0 | 25.0 | 0.0 | `./malloc/./malloc/malloc.c:3402` |
| 25.0 | 25.0 | 0.0 | `./malloc/./malloc/malloc.c:4658` |
| 25.0 | 25.0 | 0.0 | `library/std/src/sys/alloc/unix.rs:65` |
| 25.0 | 25.0 | 0.0 | `./malloc/./malloc/malloc.c:3311` |
| 20.0 | 20.0 | 0.0 | `./malloc/./malloc/malloc.c:3368` |
| 20.0 | 20.0 | 0.0 | `./malloc/./malloc/malloc.c:4505` |
| 20.0 | 20.0 | 0.0 | `./malloc/./malloc/malloc.c:3163` |
| 20.0 | 20.0 | 0.0 | `/wt/benchmarks/counts/src/lib.rs:36` |
| 20.0 | 20.0 | 0.0 | `./malloc/./malloc/malloc.c:3301` |
| 20.0 | 20.0 | 0.0 | `./malloc/./malloc/malloc.c:3371` |
| 20.0 | 20.0 | 0.0 | `./malloc/./malloc/arena.c:153` |
| 20.0 | 20.0 | 0.0 | `./malloc/./malloc/malloc.c:3316` |
| 20.0 | 20.0 | 0.0 | `./malloc/./malloc/malloc.c:4510` |
| 20.0 | 20.0 | 0.0 | `/wt/benchmarks/counts/src/bin/codec_counts.rs:21` |
| 20.0 | 20.0 | 0.0 | `./malloc/./malloc/malloc.c:1336` |
| 15.0 | 15.0 | 0.0 | `./malloc/./malloc/malloc.c:1461` |
| 15.0 | 15.0 | 0.0 | `./malloc/./malloc/malloc.c:3398` |
| 15.0 | 15.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:993` |
| 15.0 | 15.0 | 0.0 | `./malloc/./malloc/malloc.c:3186` |
| 15.0 | 15.0 | 0.0 | `./malloc/./malloc/malloc.c:3395` |
| 14.0 | 7.0 | -7.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:78` |
| 14.0 | 7.0 | -7.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:80` |
| 14.0 | 7.0 | -7.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:82` |
| 14.0 | 7.0 | -7.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:95` |
| 14.0 | 7.0 | -7.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:86` |
| 14.0 | 7.0 | -7.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:77` |
| 14.0 | 7.0 | -7.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:79` |
| 14.0 | 7.0 | -7.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:81` |
| 14.0 | 7.0 | -7.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:72` |
| 14.0 | 7.0 | -7.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:85` |
| 12.0 | 1.0 | -11.0 | `library/core/src/ptr/mut_ptr.rs:970` |
| 12.0 | 5.0 | -7.0 | `./string/../sysdeps/aarch64/multiarch/../memcpy.S:104` |
| 12.0 | 0.0 | -12.0 | `crates/bacnet-encoding/src/apdu/mod.rs:335` |
| 12.0 | 4.0 | -8.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:1208` |
| 12.0 | 3.0 | -9.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes_mut.rs:612` |
| 11.0 | 34.0 | 23.0 | `crates/bacnet-encoding/src/apdu/mod.rs:0` |
| 10.0 | 10.0 | 0.0 | `./malloc/./malloc/malloc.c:3317` |
| 10.0 | 10.0 | 0.0 | `./malloc/./malloc/malloc.c:3165` |
| 10.0 | 10.0 | 0.0 | `./malloc/./malloc/malloc.c:3167` |
| 10.0 | 10.0 | 0.0 | `./malloc/./malloc/malloc.c:3178` |
| 10.0 | 10.0 | 0.0 | `./malloc/./malloc/malloc.c:4517` |
| 10.0 | 10.0 | 0.0 | `./malloc/./malloc/malloc.c:3182` |
| 10.0 | 10.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1262` |
