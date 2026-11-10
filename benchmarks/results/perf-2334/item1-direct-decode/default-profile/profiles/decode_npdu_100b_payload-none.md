# decode_npdu_100b_payload / none: self instructions per iteration

Median slope over the ordinals; the second column is B.
Calls in counts calls from other functions (a recursion level calling itself isn't one).

## Functions (callgrind_annotate, hash suffixes stripped)

| A | B | B - A | Calls in (A) | Function |
| --- | --- | --- | --- | --- |
| 127.0 | 108.0 | -19.0 | 0.00 | `bacnet_encoding::npdu::decode_npdu` |
| 46.0 | 0.0 | -46.0 | 1.00 | `<bacnet_npdu::view::Npdu>::decode` |
| 21.0 | 21.0 | 0.0 | 1.00 | `bytes::bytes::promotable_even_clone` |
| 15.0 | 15.0 | 0.0 | 1.00 | `bytes::bytes::shared_drop` |
| 5.0 | 5.0 | 0.0 | 1.00 | `__aarch64_ldadd8_relax` |
| 5.0 | 5.0 | 0.0 | 1.00 | `__aarch64_ldadd8_rel` |
| 0.0 | 0.0 | 0.0 | 0.00 | `bacnet_counts::parse_n` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<u64 as core::fmt::Display>::fmt` |
| 0.0 | 0.0 | 0.0 | 0.00 | `core::str::converts::from_utf8` |
| 0.0 | 0.0 | 0.0 | 0.00 | `getdelim` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<core::fmt::Formatter>::pad_integral` |
| 0.0 | 0.0 | 0.0 | 0.00 | `std::rt::lang_start::<()>::{closure#0}` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_map_object_from_fd` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__GI___close_nocancel` |
| 0.0 | 0.0 | 0.0 | 0.00 | `calloc` |
| 0.0 | 0.0 | 0.0 | 0.00 | `init_have_lse_atomics` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x0000000004883190` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__minimal_free` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x00000000000038a4` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_allocate_tls_init` |
| 0.0 | 0.0 | 0.0 | 0.00 | `fclose@@GLIBC_2.17` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x0000000000009a80` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_IO_new_file_init_internal` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__sigjmp_save` |
| 0.0 | 0.0 | 0.0 | 0.00 | `0x000000000000cb60` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__libc_sigaction` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<&str as core::fmt::Display>::fmt` |
| 0.0 | 0.0 | 0.0 | 0.00 | `write` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__aarch64_cas8_acq` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_bti_check` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_add_to_slotinfo` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_fixup` |
| 0.0 | 0.0 | 0.0 | 0.00 | `std::io::stdio::_eprint` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__strtoul_internal` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__glibc_morecore` |
| 0.0 | 0.0 | 0.0 | 0.00 | `bacnet_encoding::npdu::encode_npdu` |
| 0.0 | 0.0 | 0.0 | 0.00 | `core::fmt::write` |
| 0.0 | 0.0 | 0.0 | 0.00 | `strstr` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_IO_unsave_markers` |
| 0.0 | 0.0 | 0.0 | 0.00 | `codec_counts::run::{closure#7}` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<f64 as core::fmt::Display>::fmt` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_IO_file_close_it@@GLIBC_2.17` |
| 0.0 | 0.0 | 0.0 | 0.00 | `openaux` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<bacnet_counts::CountingAlloc as core::alloc::global::GlobalAlloc>::alloc` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_IO_file_open` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__GI__IO_un_link.part.0` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__read_nocancel` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dlfo_process_initial` |
| 0.0 | 0.0 | 0.0 | 0.00 | `strcspn` |
| 0.0 | 0.0 | 0.0 | 0.00 | `std::sys::pal::unix::stack_overflow::imp::make_handler` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_audit_preinit` |
| 0.0 | 0.0 | 0.0 | 0.00 | `getpagesize` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_next_ld_env_entry` |
| 0.0 | 0.0 | 0.0 | 0.00 | `__errno_location` |
| 0.0 | 0.0 | 0.0 | 0.00 | `<str as core::fmt::Display>::fmt` |
| 0.0 | 0.0 | 0.0 | 0.00 | `alloc_perturb` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_unload_cache` |
| 0.0 | 0.0 | 0.0 | 0.00 | `_dl_map_object` |
| 0.0 | 0.0 | 0.0 | 0.00 | `fstat` |
| 0.0 | 0.0 | 0.0 | 0.00 | `fstatat` |

## Largest function changes, B - A

| A | B | B - A | Function |
| --- | --- | --- | --- |
| 46.0 | 0.0 | -46.0 | `<bacnet_npdu::view::Npdu>::decode` |
| 127.0 | 108.0 | -19.0 | `bacnet_encoding::npdu::decode_npdu` |
| 0.0 | 0.0 | 0.0 | `getdelim` |
| 0.0 | 0.0 | 0.0 | `<core::fmt::Formatter>::pad_integral` |
| 0.0 | 0.0 | 0.0 | `std::rt::lang_start::<()>::{closure#0}` |
| 0.0 | 0.0 | 0.0 | `_dl_map_object_from_fd` |
| 0.0 | 0.0 | 0.0 | `__GI___close_nocancel` |
| 0.0 | 0.0 | 0.0 | `calloc` |
| 0.0 | 0.0 | 0.0 | `init_have_lse_atomics` |
| 0.0 | 0.0 | 0.0 | `0x0000000004883190` |
| 0.0 | 0.0 | 0.0 | `__minimal_free` |
| 0.0 | 0.0 | 0.0 | `0x00000000000038a4` |
| 0.0 | 0.0 | 0.0 | `_dl_allocate_tls_init` |
| 0.0 | 0.0 | 0.0 | `fclose@@GLIBC_2.17` |
| 0.0 | 0.0 | 0.0 | `0x0000000000009a80` |
| 0.0 | 0.0 | 0.0 | `_IO_new_file_init_internal` |
| 0.0 | 0.0 | 0.0 | `__sigjmp_save` |
| 0.0 | 0.0 | 0.0 | `0x000000000000cb60` |
| 0.0 | 0.0 | 0.0 | `__libc_sigaction` |
| 0.0 | 0.0 | 0.0 | `<&str as core::fmt::Display>::fmt` |
| 0.0 | 0.0 | 0.0 | `write` |
| 0.0 | 0.0 | 0.0 | `__aarch64_cas8_acq` |
| 0.0 | 0.0 | 0.0 | `_dl_bti_check` |
| 0.0 | 0.0 | 0.0 | `_dl_add_to_slotinfo` |
| 0.0 | 0.0 | 0.0 | `_dl_fixup` |
| 0.0 | 0.0 | 0.0 | `std::io::stdio::_eprint` |
| 0.0 | 0.0 | 0.0 | `__strtoul_internal` |
| 0.0 | 0.0 | 0.0 | `__glibc_morecore` |
| 0.0 | 0.0 | 0.0 | `bacnet_encoding::npdu::encode_npdu` |
| 0.0 | 0.0 | 0.0 | `core::fmt::write` |

## Source lines (line tables; inlined code counts at its own line)

| A | B | B - A | File:line |
| --- | --- | --- | --- |
| 25.0 | 0.0 | -25.0 | `crates/bacnet-encoding/src/npdu.rs:109` |
| 15.0 | 0.0 | -15.0 | `crates/bacnet-encoding/src/npdu.rs:232` |
| 14.0 | 0.0 | -14.0 | `crates/bacnet-npdu/src/view.rs:293` |
| 9.0 | 0.0 | -9.0 | `crates/bacnet-encoding/src/npdu.rs:257` |
| 8.0 | 0.0 | -8.0 | `crates/bacnet-encoding/src/npdu.rs:258` |
| 8.0 | 8.0 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:250` |
| 8.0 | 0.0 | -8.0 | `crates/bacnet-encoding/src/npdu.rs:220` |
| 8.0 | 8.0 | 0.0 | `library/core/src/result.rs:1183` |
| 7.0 | 8.0 | 1.0 | `crates/bacnet-encoding/src/npdu.rs:231` |
| 7.0 | 0.0 | -7.0 | `crates/bacnet-npdu/src/view.rs:0` |
| 6.0 | 6.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:677` |
| 6.0 | 0.0 | -6.0 | `crates/bacnet-npdu/src/view.rs:217` |
| 5.0 | 0.0 | -5.0 | `crates/bacnet-npdu/src/view.rs:304` |
| 5.0 | 5.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1448` |
| 4.0 | 4.0 | 0.0 | `library/core/src/ptr/mod.rs:847` |
| 4.0 | 4.0 | 0.0 | `library/core/src/sync/atomic.rs:3995` |
| 4.0 | 0.0 | -4.0 | `crates/bacnet-npdu/src/view.rs:218` |
| 4.0 | 4.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1195` |
| 4.0 | 4.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1450` |
| 4.0 | 4.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1459` |
| 4.0 | 4.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1184` |
| 3.0 | 3.0 | 0.0 | `library/core/src/result.rs:1182` |
| 3.0 | 3.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:670` |
| 3.0 | 3.0 | 0.0 | `/wt/benchmarks/counts/src/bin/codec_counts.rs:154` |
| 3.0 | 0.0 | -3.0 | `crates/bacnet-encoding/src/npdu.rs:224` |
| 3.0 | 4.0 | 1.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:645` |
| 3.0 | 3.0 | 0.0 | `library/core/src/sync/atomic.rs:4049` |
| 3.0 | 0.0 | -3.0 | `library/core/src/option.rs:1162` |
| 3.0 | 0.0 | -3.0 | `crates/bacnet-encoding/src/npdu.rs:254` |
| 2.0 | 2.0 | 0.0 | `library/core/src/slice/index.rs:184` |
| 2.0 | 0.0 | -2.0 | `crates/bacnet-npdu/src/view.rs:221` |
| 2.0 | 2.0 | 0.0 | `library/compiler-builtins/compiler-builtins/./lib/builtins/aarch64/lse.S:252` |
| 2.0 | 2.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1543` |
| 2.0 | 0.0 | -2.0 | `library/core/src/slice/index.rs:544` |
| 2.0 | 2.0 | 0.0 | `library/core/src/cell.rs:2437` |
| 2.0 | 2.0 | 0.0 | `library/core/src/sync/atomic.rs:4029` |
| 2.0 | 2.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:695` |
| 2.0 | 0.0 | -2.0 | `library/core/src/option.rs:0` |
| 2.0 | 0.0 | -2.0 | `crates/bacnet-encoding/src/npdu.rs:256` |
| 2.0 | 0.0 | -2.0 | `crates/bacnet-encoding/src/npdu.rs:223` |
| 1.0 | 1.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1455` |
| 1.0 | 0.0 | -1.0 | `crates/bacnet-npdu/src/view.rs:282` |
| 1.0 | 6.0 | 5.0 | `crates/bacnet-encoding/src/npdu.rs:0` |
| 1.0 | 1.0 | 0.0 | `library/core/src/iter/range.rs:1100` |
| 1.0 | 1.0 | 0.0 | `library/core/src/ptr/const_ptr.rs:871` |
| 1.0 | 0.0 | -1.0 | `crates/bacnet-npdu/src/view.rs:275` |
| 1.0 | 1.0 | 0.0 | `library/core/src/cmp.rs:2144` |
| 1.0 | 0.0 | -1.0 | `index.crates.io-1949cf8c6b5b557f/smallvec-1.15.2/src/lib.rs:0` |
| 1.0 | 1.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:658` |
| 1.0 | 1.0 | 0.0 | `index.crates.io-1949cf8c6b5b557f/bytes-1.12.1/src/bytes.rs:1188` |
| 1.0 | 0.0 | -1.0 | `library/core/src/slice/index.rs:90` |
| 1.0 | 1.0 | 0.0 | `crates/bacnet-npdu/src/priority.rs:51` |
| 1.0 | 0.0 | -1.0 | `crates/bacnet-npdu/src/view.rs:228` |
| 1.0 | 0.0 | -1.0 | `crates/bacnet-npdu/src/view.rs:252` |
| 0.0 | 0.0 | 0.0 | `library/core/src/str/validations.rs:241` |
| 0.0 | 0.0 | 0.0 | `library/core/src/fmt/num.rs:248` |
| 0.0 | 0.0 | 0.0 | `library/core/src/num/mod.rs:1819` |
| 0.0 | 0.0 | 0.0 | `library/core/src/num/mod.rs:1801` |
| 0.0 | 0.0 | 0.0 | `library/core/src/num/mod.rs:1822` |
| 0.0 | 0.0 | 0.0 | `library/core/src/num/mod.rs:1821` |
