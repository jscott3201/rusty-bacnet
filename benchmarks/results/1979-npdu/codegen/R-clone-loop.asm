0000000100029bd0 <<criterion::bencher::Bencher>::iter::<(), encoding::npdu_attribution::diagnostics::{closure#1}::{closure#0}>>:
100029bd0:     	sub	sp, sp, #0x80
100029bd4:     	stp	x26, x25, [sp, #0x30]
100029bd8:     	stp	x24, x23, [sp, #0x40]
100029bdc:     	stp	x22, x21, [sp, #0x50]
100029be0:     	stp	x20, x19, [sp, #0x60]
100029be4:     	stp	x29, x30, [sp, #0x70]
100029be8:     	add	x29, sp, #0x70
100029bec:     	mov	x20, x1
100029bf0:     	mov	x19, x0
100029bf4:     	mov	w8, #0x1                ; =1
100029bf8:     	strb	w8, [x0, #0x30]
100029bfc:     	bl	0x100190054 <<std::time::Instant>::now>
100029c00:     	str	x0, [sp]
100029c04:     	str	w1, [sp, #0x8]
100029c08:     	ldr	x21, [x19, #0x20]
100029c0c:     	mov	x0, x21
100029c10:     	bl	0x100074ee0 <<criterion::measurement::WallTime as criterion::measurement::Measurement>::start>
100029c14:     	mov	x22, x0
100029c18:     	mov	x23, x1
100029c1c:     	ldr	x24, [x19, #0x28]
100029c20:     	cbz	x24, 0x100029c58 <<criterion::bencher::Bencher>::iter::<(), encoding::npdu_attribution::diagnostics::{closure#1}::{closure#0}>+0x88>
100029c24:     	add	x25, sp, #0x10
100029c28:     	ldp	x8, x1, [x20]
100029c2c:     	ldr	x9, [x8]
100029c30:     	ldr	x2, [x20, #0x10]
100029c34:     	add	x8, sp, #0x10
100029c38:     	add	x0, x20, #0x18
100029c3c:     	blr	x9
100029c40:     	ldp	x8, x1, [sp, #0x10]
100029c44:     	ldr	x8, [x8, #0x20]
100029c48:     	ldp	x2, x0, [sp, #0x20]
100029c4c:     	blr	x8
100029c50:     	subs	x24, x24, #0x1
100029c54:     	b.ne	0x100029c28 <<criterion::bencher::Bencher>::iter::<(), encoding::npdu_attribution::diagnostics::{closure#1}::{closure#0}>+0x58>
100029c58:     	mov	x0, x21
100029c5c:     	mov	x1, x22
100029c60:     	mov	x2, x23
100029c64:     	bl	0x100074eb8 <<criterion::measurement::WallTime as criterion::measurement::Measurement>::end>
100029c68:     	str	x0, [x19]
100029c6c:     	str	w1, [x19, #0x8]
100029c70:     	mov	x0, sp
100029c74:     	bl	0x100190080 <<std::time::Instant>::elapsed>
100029c78:     	str	x0, [x19, #0x10]
100029c7c:     	str	w1, [x19, #0x18]
100029c80:     	ldp	x29, x30, [sp, #0x70]
100029c84:     	ldp	x20, x19, [sp, #0x60]
100029c88:     	ldp	x22, x21, [sp, #0x50]
100029c8c:     	ldp	x24, x23, [sp, #0x40]
100029c90:     	ldp	x26, x25, [sp, #0x30]
100029c94:     	add	sp, sp, #0x80
100029c98:     	ret
		...

