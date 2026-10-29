0000000100028a40 <<criterion::bencher::Bencher>::iter::<(), encoding::bench_decode_npdu::{closure#0}::{closure#0}>>:
100028a40:     	sub	sp, sp, #0x170
100028a44:     	stp	x28, x27, [sp, #0x110]
100028a48:     	stp	x26, x25, [sp, #0x120]
100028a4c:     	stp	x24, x23, [sp, #0x130]
100028a50:     	stp	x22, x21, [sp, #0x140]
100028a54:     	stp	x20, x19, [sp, #0x150]
100028a58:     	stp	x29, x30, [sp, #0x160]
100028a5c:     	add	x29, sp, #0x160
100028a60:     	mov	x20, x1
100028a64:     	mov	x19, x0
100028a68:     	mov	w8, #0x1                ; =1
100028a6c:     	strb	w8, [x0, #0x30]
100028a70:     	bl	0x100190054 <<std::time::Instant>::now>
100028a74:     	str	x0, [sp]
100028a78:     	str	w1, [sp, #0x8]
100028a7c:     	ldr	x21, [x19, #0x20]
100028a80:     	mov	x0, x21
100028a84:     	bl	0x100074ee0 <<criterion::measurement::WallTime as criterion::measurement::Measurement>::start>
100028a88:     	mov	x22, x0
100028a8c:     	mov	x23, x1
100028a90:     	ldr	x25, [x19, #0x28]
100028a94:     	cbz	x25, 0x100028b50 <<criterion::bencher::Bencher>::iter::<(), encoding::bench_decode_npdu::{closure#0}::{closure#0}>+0x110>
100028a98:     	add	x24, sp, #0x90
100028a9c:     	add	x26, sp, #0x10
100028aa0:     	b	0x100028abc <<criterion::bencher::Bencher>::iter::<(), encoding::bench_decode_npdu::{closure#0}::{closure#0}>+0x7c>
100028aa4:     	ldp	x8, x1, [sp, #0x60]
100028aa8:     	ldr	x8, [x8, #0x20]
100028aac:     	ldp	x2, x0, [sp, #0x70]
100028ab0:     	blr	x8
100028ab4:     	subs	x25, x25, #0x1
100028ab8:     	b.eq	0x100028b50 <<criterion::bencher::Bencher>::iter::<(), encoding::bench_decode_npdu::{closure#0}::{closure#0}>+0x110>
100028abc:     	ldp	x8, x1, [x20]
100028ac0:     	ldr	x9, [x8]
100028ac4:     	ldr	x2, [x20, #0x10]
100028ac8:     	add	x8, sp, #0x10
100028acc:     	add	x0, x20, #0x18
100028ad0:     	blr	x9
100028ad4:     	add	x8, sp, #0x90
100028ad8:     	add	x0, sp, #0x10
100028adc:     	bl	0x10017a8a8 <bacnet_encoding::npdu::decode_npdu>
100028ae0:     	ldr	x8, [sp, #0x90]
100028ae4:     	cmp	x8, #0x2
100028ae8:     	b.eq	0x100028b98 <<criterion::bencher::Bencher>::iter::<(), encoding::bench_decode_npdu::{closure#0}::{closure#0}>+0x158>
100028aec:     	ldp	q0, q1, [sp, #0xd0]
100028af0:     	stp	q0, q1, [sp, #0x50]
100028af4:     	ldp	q0, q1, [sp, #0xf0]
100028af8:     	stp	q0, q1, [sp, #0x70]
100028afc:     	ldp	q0, q1, [sp, #0x90]
100028b00:     	stp	q0, q1, [sp, #0x10]
100028b04:     	ldp	q0, q1, [sp, #0xb0]
100028b08:     	stp	q0, q1, [sp, #0x30]
100028b0c:     	ldr	x8, [sp, #0x10]
100028b10:     	cbz	x8, 0x100028b2c <<criterion::bencher::Bencher>::iter::<(), encoding::bench_decode_npdu::{closure#0}::{closure#0}>+0xec>
100028b14:     	ldr	x1, [sp, #0x28]
100028b18:     	cmp	x1, #0x7
100028b1c:     	b.lo	0x100028b2c <<criterion::bencher::Bencher>::iter::<(), encoding::bench_decode_npdu::{closure#0}::{closure#0}>+0xec>
100028b20:     	ldr	x0, [sp, #0x18]
100028b24:     	mov	w2, #0x1                ; =1
100028b28:     	bl	0x10003091c <__rustc::__rust_dealloc>
100028b2c:     	ldr	x8, [sp, #0x38]
100028b30:     	cbz	x8, 0x100028aa4 <<criterion::bencher::Bencher>::iter::<(), encoding::bench_decode_npdu::{closure#0}::{closure#0}>+0x64>
100028b34:     	ldr	x1, [sp, #0x50]
100028b38:     	cmp	x1, #0x7
100028b3c:     	b.lo	0x100028aa4 <<criterion::bencher::Bencher>::iter::<(), encoding::bench_decode_npdu::{closure#0}::{closure#0}>+0x64>
100028b40:     	ldr	x0, [sp, #0x40]
100028b44:     	mov	w2, #0x1                ; =1
100028b48:     	bl	0x10003091c <__rustc::__rust_dealloc>
100028b4c:     	b	0x100028aa4 <<criterion::bencher::Bencher>::iter::<(), encoding::bench_decode_npdu::{closure#0}::{closure#0}>+0x64>
100028b50:     	mov	x0, x21
100028b54:     	mov	x1, x22
100028b58:     	mov	x2, x23
100028b5c:     	bl	0x100074eb8 <<criterion::measurement::WallTime as criterion::measurement::Measurement>::end>
100028b60:     	str	x0, [x19]
100028b64:     	str	w1, [x19, #0x8]
100028b68:     	mov	x0, sp
100028b6c:     	bl	0x100190080 <<std::time::Instant>::elapsed>
100028b70:     	str	x0, [x19, #0x10]
100028b74:     	str	w1, [x19, #0x18]
100028b78:     	ldp	x29, x30, [sp, #0x160]
100028b7c:     	ldp	x20, x19, [sp, #0x150]
100028b80:     	ldp	x22, x21, [sp, #0x140]
100028b84:     	ldp	x24, x23, [sp, #0x130]
100028b88:     	ldp	x26, x25, [sp, #0x120]
100028b8c:     	ldp	x28, x27, [sp, #0x110]
100028b90:     	add	sp, sp, #0x170
100028b94:     	ret
100028b98:     	ldur	q0, [x24, #0x8]
100028b9c:     	ldur	q1, [x24, #0x18]
100028ba0:     	stp	q0, q1, [sp, #0x10]
100028ba4:     	ldur	q0, [x24, #0x28]
100028ba8:     	str	q0, [sp, #0x30]
100028bac:     	adrp	x0, 0x1001fb000 <_anon.f4425d42b702ce31c9ff391ac740e4a9.15+0x86>
100028bb0:     	add	x0, x0, #0x584
100028bb4:     	adrp	x3, 0x100269000 <_anon.d4ccd2679ef62a4a7bc83d38024e4828.8+0xe8>
100028bb8:     	add	x3, x3, #0x258
100028bbc:     	adrp	x4, 0x100269000 <_anon.d4ccd2679ef62a4a7bc83d38024e4828.8+0xe8>
100028bc0:     	add	x4, x4, #0x140
100028bc4:     	add	x2, sp, #0x10
100028bc8:     	mov	w1, #0x2b               ; =43
100028bcc:     	bl	0x1001dc97c <core::result::unwrap_failed>
100028bd0:     	brk	#0x1
100028bd4:     	ldr	x8, [sp, #0x10]
100028bd8:     	cmp	x8, #0x1
100028bdc:     	b.ne	0x100028c04 <<criterion::bencher::Bencher>::iter::<(), encoding::bench_decode_npdu::{closure#0}::{closure#0}>+0x1c4>
100028be0:     	ldr	x1, [sp, #0x28]
100028be4:     	cmp	x1, #0x7
100028be8:     	b.lo	0x100028c04 <<criterion::bencher::Bencher>::iter::<(), encoding::bench_decode_npdu::{closure#0}::{closure#0}>+0x1c4>
100028bec:     	ldr	x8, [sp, #0x18]
100028bf0:     	mov	x19, x0
100028bf4:     	mov	x0, x8
100028bf8:     	mov	w2, #0x1                ; =1
100028bfc:     	bl	0x10003091c <__rustc::__rust_dealloc>
100028c00:     	mov	x0, x19
100028c04:     	bl	0x1001dce98 <dyld_stub_binder+0x1001dce98>

