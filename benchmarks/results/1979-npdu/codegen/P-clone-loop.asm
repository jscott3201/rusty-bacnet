0000000100024e74 <<criterion::bencher::Bencher>::iter::<(), encoding::npdu_attribution::diagnostics::{closure#1}::{closure#0}>>:
100024e74:     	sub	sp, sp, #0x80
100024e78:     	stp	x26, x25, [sp, #0x30]
100024e7c:     	stp	x24, x23, [sp, #0x40]
100024e80:     	stp	x22, x21, [sp, #0x50]
100024e84:     	stp	x20, x19, [sp, #0x60]
100024e88:     	stp	x29, x30, [sp, #0x70]
100024e8c:     	add	x29, sp, #0x70
100024e90:     	mov	x20, x1
100024e94:     	mov	x19, x0
100024e98:     	mov	w8, #0x1                ; =1
100024e9c:     	strb	w8, [x0, #0x30]
100024ea0:     	bl	0x10018fc54 <<std::time::Instant>::now>
100024ea4:     	str	x0, [sp]
100024ea8:     	str	w1, [sp, #0x8]
100024eac:     	ldr	x21, [x19, #0x20]
100024eb0:     	mov	x0, x21
100024eb4:     	bl	0x100074e20 <<criterion::measurement::WallTime as criterion::measurement::Measurement>::start>
100024eb8:     	mov	x22, x0
100024ebc:     	mov	x23, x1
100024ec0:     	ldr	x24, [x19, #0x28]
100024ec4:     	cbz	x24, 0x100024efc <<criterion::bencher::Bencher>::iter::<(), encoding::npdu_attribution::diagnostics::{closure#1}::{closure#0}>+0x88>
100024ec8:     	add	x25, sp, #0x10
100024ecc:     	ldp	x8, x1, [x20]
100024ed0:     	ldr	x9, [x8]
100024ed4:     	ldr	x2, [x20, #0x10]
100024ed8:     	add	x8, sp, #0x10
100024edc:     	add	x0, x20, #0x18
100024ee0:     	blr	x9
100024ee4:     	ldp	x8, x1, [sp, #0x10]
100024ee8:     	ldr	x8, [x8, #0x20]
100024eec:     	ldp	x2, x0, [sp, #0x20]
100024ef0:     	blr	x8
100024ef4:     	subs	x24, x24, #0x1
100024ef8:     	b.ne	0x100024ecc <<criterion::bencher::Bencher>::iter::<(), encoding::npdu_attribution::diagnostics::{closure#1}::{closure#0}>+0x58>
100024efc:     	mov	x0, x21
100024f00:     	mov	x1, x22
100024f04:     	mov	x2, x23
100024f08:     	bl	0x100074df8 <<criterion::measurement::WallTime as criterion::measurement::Measurement>::end>
100024f0c:     	str	x0, [x19]
100024f10:     	str	w1, [x19, #0x8]
100024f14:     	mov	x0, sp
100024f18:     	bl	0x10018fc80 <<std::time::Instant>::elapsed>
100024f1c:     	str	x0, [x19, #0x10]
100024f20:     	str	w1, [x19, #0x18]
100024f24:     	ldp	x29, x30, [sp, #0x70]
100024f28:     	ldp	x20, x19, [sp, #0x60]
100024f2c:     	ldp	x22, x21, [sp, #0x50]
100024f30:     	ldp	x24, x23, [sp, #0x40]
100024f34:     	ldp	x26, x25, [sp, #0x30]
100024f38:     	add	sp, sp, #0x80
100024f3c:     	ret
		...

