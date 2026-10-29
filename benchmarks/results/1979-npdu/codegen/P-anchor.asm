0000000100023d00 <<criterion::bencher::Bencher>::iter::<(), encoding::bench_decode_npdu::{closure#0}::{closure#0}>>:
100023d00:     	sub	sp, sp, #0x170
100023d04:     	stp	x28, x27, [sp, #0x110]
100023d08:     	stp	x26, x25, [sp, #0x120]
100023d0c:     	stp	x24, x23, [sp, #0x130]
100023d10:     	stp	x22, x21, [sp, #0x140]
100023d14:     	stp	x20, x19, [sp, #0x150]
100023d18:     	stp	x29, x30, [sp, #0x160]
100023d1c:     	add	x29, sp, #0x160
100023d20:     	mov	x20, x1
100023d24:     	mov	x19, x0
100023d28:     	mov	w8, #0x1                ; =1
100023d2c:     	strb	w8, [x0, #0x30]
100023d30:     	bl	0x10018fc54 <<std::time::Instant>::now>
100023d34:     	str	x0, [sp]
100023d38:     	str	w1, [sp, #0x8]
100023d3c:     	ldr	x21, [x19, #0x20]
100023d40:     	mov	x0, x21
100023d44:     	bl	0x100074e20 <<criterion::measurement::WallTime as criterion::measurement::Measurement>::start>
100023d48:     	mov	x22, x0
100023d4c:     	mov	x23, x1
100023d50:     	ldr	x25, [x19, #0x28]
100023d54:     	cbz	x25, 0x100023e10 <<criterion::bencher::Bencher>::iter::<(), encoding::bench_decode_npdu::{closure#0}::{closure#0}>+0x110>
100023d58:     	add	x24, sp, #0x90
100023d5c:     	add	x26, sp, #0x10
100023d60:     	b	0x100023d7c <<criterion::bencher::Bencher>::iter::<(), encoding::bench_decode_npdu::{closure#0}::{closure#0}>+0x7c>
100023d64:     	ldp	x8, x1, [sp, #0x60]
100023d68:     	ldr	x8, [x8, #0x20]
100023d6c:     	ldp	x2, x0, [sp, #0x70]
100023d70:     	blr	x8
100023d74:     	subs	x25, x25, #0x1
100023d78:     	b.eq	0x100023e10 <<criterion::bencher::Bencher>::iter::<(), encoding::bench_decode_npdu::{closure#0}::{closure#0}>+0x110>
100023d7c:     	ldp	x8, x1, [x20]
100023d80:     	ldr	x9, [x8]
100023d84:     	ldr	x2, [x20, #0x10]
100023d88:     	add	x8, sp, #0x10
100023d8c:     	add	x0, x20, #0x18
100023d90:     	blr	x9
100023d94:     	add	x8, sp, #0x90
100023d98:     	add	x0, sp, #0x10
100023d9c:     	bl	0x10017ac6c <bacnet_encoding::npdu::decode_npdu>
100023da0:     	ldr	x8, [sp, #0x90]
100023da4:     	cmp	x8, #0x2
100023da8:     	b.eq	0x100023e58 <<criterion::bencher::Bencher>::iter::<(), encoding::bench_decode_npdu::{closure#0}::{closure#0}>+0x158>
100023dac:     	ldp	q0, q1, [sp, #0xd0]
100023db0:     	stp	q0, q1, [sp, #0x50]
100023db4:     	ldp	q0, q1, [sp, #0xf0]
100023db8:     	stp	q0, q1, [sp, #0x70]
100023dbc:     	ldp	q0, q1, [sp, #0x90]
100023dc0:     	stp	q0, q1, [sp, #0x10]
100023dc4:     	ldp	q0, q1, [sp, #0xb0]
100023dc8:     	stp	q0, q1, [sp, #0x30]
100023dcc:     	ldr	x8, [sp, #0x10]
100023dd0:     	cbz	x8, 0x100023dec <<criterion::bencher::Bencher>::iter::<(), encoding::bench_decode_npdu::{closure#0}::{closure#0}>+0xec>
100023dd4:     	ldr	x1, [sp, #0x28]
100023dd8:     	cmp	x1, #0x7
100023ddc:     	b.lo	0x100023dec <<criterion::bencher::Bencher>::iter::<(), encoding::bench_decode_npdu::{closure#0}::{closure#0}>+0xec>
100023de0:     	ldr	x0, [sp, #0x18]
100023de4:     	mov	w2, #0x1                ; =1
100023de8:     	bl	0x1000305cc <__rustc::__rust_dealloc>
100023dec:     	ldr	x8, [sp, #0x38]
100023df0:     	cbz	x8, 0x100023d64 <<criterion::bencher::Bencher>::iter::<(), encoding::bench_decode_npdu::{closure#0}::{closure#0}>+0x64>
100023df4:     	ldr	x1, [sp, #0x50]
100023df8:     	cmp	x1, #0x7
100023dfc:     	b.lo	0x100023d64 <<criterion::bencher::Bencher>::iter::<(), encoding::bench_decode_npdu::{closure#0}::{closure#0}>+0x64>
100023e00:     	ldr	x0, [sp, #0x40]
100023e04:     	mov	w2, #0x1                ; =1
100023e08:     	bl	0x1000305cc <__rustc::__rust_dealloc>
100023e0c:     	b	0x100023d64 <<criterion::bencher::Bencher>::iter::<(), encoding::bench_decode_npdu::{closure#0}::{closure#0}>+0x64>
100023e10:     	mov	x0, x21
100023e14:     	mov	x1, x22
100023e18:     	mov	x2, x23
100023e1c:     	bl	0x100074df8 <<criterion::measurement::WallTime as criterion::measurement::Measurement>::end>
100023e20:     	str	x0, [x19]
100023e24:     	str	w1, [x19, #0x8]
100023e28:     	mov	x0, sp
100023e2c:     	bl	0x10018fc80 <<std::time::Instant>::elapsed>
100023e30:     	str	x0, [x19, #0x10]
100023e34:     	str	w1, [x19, #0x18]
100023e38:     	ldp	x29, x30, [sp, #0x160]
100023e3c:     	ldp	x20, x19, [sp, #0x150]
100023e40:     	ldp	x22, x21, [sp, #0x140]
100023e44:     	ldp	x24, x23, [sp, #0x130]
100023e48:     	ldp	x26, x25, [sp, #0x120]
100023e4c:     	ldp	x28, x27, [sp, #0x110]
100023e50:     	add	sp, sp, #0x170
100023e54:     	ret
100023e58:     	ldur	q0, [x24, #0x8]
100023e5c:     	ldur	q1, [x24, #0x18]
100023e60:     	stp	q0, q1, [sp, #0x10]
100023e64:     	ldur	q0, [x24, #0x28]
100023e68:     	ldur	q1, [x24, #0x38]
100023e6c:     	stp	q0, q1, [sp, #0x30]
100023e70:     	adrp	x0, 0x1001fa000 <_anon.fcc5c5354414f4f0fa2aa6780c9c1888.17+0x611>
100023e74:     	add	x0, x0, #0xb95
100023e78:     	adrp	x3, 0x100268000 <dyld_stub_binder+0x100268000>
100023e7c:     	add	x3, x3, #0xdb8
100023e80:     	adrp	x4, 0x100268000 <dyld_stub_binder+0x100268000>
100023e84:     	add	x4, x4, #0xcc0
100023e88:     	add	x2, sp, #0x10
100023e8c:     	mov	w1, #0x2b               ; =43
100023e90:     	bl	0x1001dc53c <core::result::unwrap_failed>
100023e94:     	brk	#0x1
100023e98:     	mov	x19, x0
100023e9c:     	add	x0, sp, #0x10
100023ea0:     	bl	0x100025174 <core::ptr::drop_glue::<bacnet_encoding::npdu::decode_error::NpduDecodeError>>
100023ea4:     	mov	x0, x19
100023ea8:     	bl	0x1001dca58 <dyld_stub_binder+0x1001dca58>
100023eac:     	bl	0x1001dc5e0 <core::panicking::panic_in_cleanup>

