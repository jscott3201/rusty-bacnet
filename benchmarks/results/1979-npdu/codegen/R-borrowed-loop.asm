0000000100029ab4 <<criterion::bencher::Bencher>::iter::<(), encoding::npdu_attribution::diagnostics::{closure#0}::{closure#0}>>:
100029ab4:     	sub	sp, sp, #0x120
100029ab8:     	stp	x28, x27, [sp, #0xc0]
100029abc:     	stp	x26, x25, [sp, #0xd0]
100029ac0:     	stp	x24, x23, [sp, #0xe0]
100029ac4:     	stp	x22, x21, [sp, #0xf0]
100029ac8:     	stp	x20, x19, [sp, #0x100]
100029acc:     	stp	x29, x30, [sp, #0x110]
100029ad0:     	add	x29, sp, #0x110
100029ad4:     	mov	x20, x1
100029ad8:     	mov	x19, x0
100029adc:     	mov	w8, #0x1                ; =1
100029ae0:     	strb	w8, [x0, #0x30]
100029ae4:     	bl	0x100190054 <<std::time::Instant>::now>
100029ae8:     	str	x0, [sp]
100029aec:     	str	w1, [sp, #0x8]
100029af0:     	ldr	x21, [x19, #0x20]
100029af4:     	mov	x0, x21
100029af8:     	bl	0x100074ee0 <<criterion::measurement::WallTime as criterion::measurement::Measurement>::start>
100029afc:     	mov	x22, x0
100029b00:     	mov	x23, x1
100029b04:     	ldr	x24, [x19, #0x28]
100029b08:     	cbz	x24, 0x100029b58 <<criterion::bencher::Bencher>::iter::<(), encoding::npdu_attribution::diagnostics::{closure#0}::{closure#0}>+0xa4>
100029b0c:     	add	x25, sp, #0x68
100029b10:     	add	x26, sp, #0x10
100029b14:     	ldp	x0, x1, [x20, #0x8]
100029b18:     	add	x8, sp, #0x68
100029b1c:     	bl	0x100182d1c <<bacnet_npdu::view::Npdu>::decode>
100029b20:     	ldrh	w8, [sp, #0xb2]
100029b24:     	cmp	w8, #0x2
100029b28:     	b.eq	0x100029ba0 <<criterion::bencher::Bencher>::iter::<(), encoding::npdu_attribution::diagnostics::{closure#0}::{closure#0}>+0xec>
100029b2c:     	ldr	q2, [x25, #0x40]
100029b30:     	ldr	x8, [sp, #0xb8]
100029b34:     	str	x8, [sp, #0x60]
100029b38:     	ldur	q0, [sp, #0x68]
100029b3c:     	ldur	q1, [sp, #0x78]
100029b40:     	stp	q0, q1, [sp, #0x10]
100029b44:     	ldp	q0, q1, [x25, #0x20]
100029b48:     	str	q0, [sp, #0x30]
100029b4c:     	stp	q1, q2, [sp, #0x40]
100029b50:     	subs	x24, x24, #0x1
100029b54:     	b.ne	0x100029b14 <<criterion::bencher::Bencher>::iter::<(), encoding::npdu_attribution::diagnostics::{closure#0}::{closure#0}>+0x60>
100029b58:     	mov	x0, x21
100029b5c:     	mov	x1, x22
100029b60:     	mov	x2, x23
100029b64:     	bl	0x100074eb8 <<criterion::measurement::WallTime as criterion::measurement::Measurement>::end>
100029b68:     	str	x0, [x19]
100029b6c:     	str	w1, [x19, #0x8]
100029b70:     	mov	x0, sp
100029b74:     	bl	0x100190080 <<std::time::Instant>::elapsed>
100029b78:     	str	x0, [x19, #0x10]
100029b7c:     	str	w1, [x19, #0x18]
100029b80:     	ldp	x29, x30, [sp, #0x110]
100029b84:     	ldp	x20, x19, [sp, #0x100]
100029b88:     	ldp	x22, x21, [sp, #0xf0]
100029b8c:     	ldp	x24, x23, [sp, #0xe0]
100029b90:     	ldp	x26, x25, [sp, #0xd0]
100029b94:     	ldp	x28, x27, [sp, #0xc0]
100029b98:     	add	sp, sp, #0x120
100029b9c:     	ret
100029ba0:     	ldur	q0, [sp, #0x68]
100029ba4:     	ldur	q1, [sp, #0x78]
100029ba8:     	stp	q0, q1, [sp, #0x10]
100029bac:     	adrp	x0, 0x1001fb000 <_anon.f4425d42b702ce31c9ff391ac740e4a9.15+0x86>
100029bb0:     	add	x0, x0, #0x584
100029bb4:     	adrp	x3, 0x100269000 <_anon.d4ccd2679ef62a4a7bc83d38024e4828.8+0xe8>
100029bb8:     	add	x3, x3, #0x218
100029bbc:     	adrp	x4, 0x100269000 <_anon.d4ccd2679ef62a4a7bc83d38024e4828.8+0xe8>
100029bc0:     	add	x4, x4, #0x1e8
100029bc4:     	add	x2, sp, #0x10
100029bc8:     	mov	w1, #0x2b               ; =43
100029bcc:     	bl	0x1001dc97c <core::result::unwrap_failed>

