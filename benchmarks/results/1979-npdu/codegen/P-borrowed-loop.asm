0000000100024d58 <<criterion::bencher::Bencher>::iter::<(), encoding::npdu_attribution::diagnostics::{closure#0}::{closure#0}>>:
100024d58:     	sub	sp, sp, #0x120
100024d5c:     	stp	x28, x27, [sp, #0xc0]
100024d60:     	stp	x26, x25, [sp, #0xd0]
100024d64:     	stp	x24, x23, [sp, #0xe0]
100024d68:     	stp	x22, x21, [sp, #0xf0]
100024d6c:     	stp	x20, x19, [sp, #0x100]
100024d70:     	stp	x29, x30, [sp, #0x110]
100024d74:     	add	x29, sp, #0x110
100024d78:     	mov	x20, x1
100024d7c:     	mov	x19, x0
100024d80:     	mov	w8, #0x1                ; =1
100024d84:     	strb	w8, [x0, #0x30]
100024d88:     	bl	0x10018fc54 <<std::time::Instant>::now>
100024d8c:     	str	x0, [sp]
100024d90:     	str	w1, [sp, #0x8]
100024d94:     	ldr	x21, [x19, #0x20]
100024d98:     	mov	x0, x21
100024d9c:     	bl	0x100074e20 <<criterion::measurement::WallTime as criterion::measurement::Measurement>::start>
100024da0:     	mov	x22, x0
100024da4:     	mov	x23, x1
100024da8:     	ldr	x24, [x19, #0x28]
100024dac:     	cbz	x24, 0x100024dfc <<criterion::bencher::Bencher>::iter::<(), encoding::npdu_attribution::diagnostics::{closure#0}::{closure#0}>+0xa4>
100024db0:     	add	x25, sp, #0x68
100024db4:     	add	x26, sp, #0x10
100024db8:     	ldp	x0, x1, [x20, #0x8]
100024dbc:     	add	x8, sp, #0x68
100024dc0:     	bl	0x1000305dc <<bacnet_npdu::view::Npdu>::decode>
100024dc4:     	ldrh	w8, [sp, #0xb2]
100024dc8:     	cmp	w8, #0x2
100024dcc:     	b.eq	0x100024e44 <<criterion::bencher::Bencher>::iter::<(), encoding::npdu_attribution::diagnostics::{closure#0}::{closure#0}>+0xec>
100024dd0:     	ldr	q2, [x25, #0x40]
100024dd4:     	ldr	x8, [sp, #0xb8]
100024dd8:     	str	x8, [sp, #0x60]
100024ddc:     	ldur	q0, [sp, #0x68]
100024de0:     	ldur	q1, [sp, #0x78]
100024de4:     	stp	q0, q1, [sp, #0x10]
100024de8:     	ldp	q0, q1, [x25, #0x20]
100024dec:     	str	q0, [sp, #0x30]
100024df0:     	stp	q1, q2, [sp, #0x40]
100024df4:     	subs	x24, x24, #0x1
100024df8:     	b.ne	0x100024db8 <<criterion::bencher::Bencher>::iter::<(), encoding::npdu_attribution::diagnostics::{closure#0}::{closure#0}>+0x60>
100024dfc:     	mov	x0, x21
100024e00:     	mov	x1, x22
100024e04:     	mov	x2, x23
100024e08:     	bl	0x100074df8 <<criterion::measurement::WallTime as criterion::measurement::Measurement>::end>
100024e0c:     	str	x0, [x19]
100024e10:     	str	w1, [x19, #0x8]
100024e14:     	mov	x0, sp
100024e18:     	bl	0x10018fc80 <<std::time::Instant>::elapsed>
100024e1c:     	str	x0, [x19, #0x10]
100024e20:     	str	w1, [x19, #0x18]
100024e24:     	ldp	x29, x30, [sp, #0x110]
100024e28:     	ldp	x20, x19, [sp, #0x100]
100024e2c:     	ldp	x22, x21, [sp, #0xf0]
100024e30:     	ldp	x24, x23, [sp, #0xe0]
100024e34:     	ldp	x26, x25, [sp, #0xd0]
100024e38:     	ldp	x28, x27, [sp, #0xc0]
100024e3c:     	add	sp, sp, #0x120
100024e40:     	ret
100024e44:     	ldur	q0, [sp, #0x68]
100024e48:     	ldur	q1, [sp, #0x78]
100024e4c:     	stp	q0, q1, [sp, #0x10]
100024e50:     	adrp	x0, 0x1001fa000 <_anon.fcc5c5354414f4f0fa2aa6780c9c1888.17+0x611>
100024e54:     	add	x0, x0, #0xb95
100024e58:     	adrp	x3, 0x100268000 <dyld_stub_binder+0x100268000>
100024e5c:     	add	x3, x3, #0xdd8
100024e60:     	adrp	x4, 0x100268000 <dyld_stub_binder+0x100268000>
100024e64:     	add	x4, x4, #0xd68
100024e68:     	add	x2, sp, #0x10
100024e6c:     	mov	w1, #0x2b               ; =43
100024e70:     	bl	0x1001dc53c <core::result::unwrap_failed>

