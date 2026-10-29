0000000100182d1c <<bacnet_npdu::view::Npdu>::decode>:
100182d1c:     	sub	sp, sp, #0x50
100182d20:     	stp	x22, x21, [sp, #0x20]
100182d24:     	stp	x20, x19, [sp, #0x30]
100182d28:     	stp	x29, x30, [sp, #0x40]
100182d2c:     	add	x29, sp, #0x40
100182d30:     	mov	x2, x1
100182d34:     	cmp	x1, #0x1
100182d38:     	b.ls	0x100182f28 <<bacnet_npdu::view::Npdu>::decode+0x20c>
100182d3c:     	mov	x1, x0
100182d40:     	ldrb	w9, [x0]
100182d44:     	cmp	w9, #0x1
100182d48:     	b.ne	0x100182d7c <<bacnet_npdu::view::Npdu>::decode+0x60>
100182d4c:     	ldrb	w9, [x1, #0x1]
100182d50:     	tbnz	w9, #0x5, 0x100182d88 <<bacnet_npdu::view::Npdu>::decode+0x6c>
100182d54:     	mov	x10, #0x0               ; =0
100182d58:     	mov	w0, #0x2                ; =2
100182d5c:     	tbnz	w9, #0x3, 0x100182dc8 <<bacnet_npdu::view::Npdu>::decode+0xac>
100182d60:     	mov	x13, #0x0               ; =0
100182d64:     	cbz	x10, 0x100182e9c <<bacnet_npdu::view::Npdu>::decode+0x180>
100182d68:     	cmp	x0, x2
100182d6c:     	b.hs	0x100182f28 <<bacnet_npdu::view::Npdu>::decode+0x20c>
100182d70:     	ldrb	w14, [x1, x0]
100182d74:     	add	x0, x0, #0x1
100182d78:     	b	0x100182ea0 <<bacnet_npdu::view::Npdu>::decode+0x184>
100182d7c:     	mov	w9, #0x1                ; =1
100182d80:     	strb	w9, [x8]
100182d84:     	b	0x100182f2c <<bacnet_npdu::view::Npdu>::decode+0x210>
100182d88:     	cmp	x2, #0x5
100182d8c:     	b.lo	0x100182f28 <<bacnet_npdu::view::Npdu>::decode+0x20c>
100182d90:     	ldrh	w10, [x1, #0x2]
100182d94:     	cbz	w10, 0x100182e2c <<bacnet_npdu::view::Npdu>::decode+0x110>
100182d98:     	rev16	w19, w10
100182d9c:     	ldrb	w20, [x1, #0x4]
100182da0:     	cmp	w20, #0x12
100182da4:     	b.ls	0x100182db4 <<bacnet_npdu::view::Npdu>::decode+0x98>
100182da8:     	tbnz	w9, #0x3, 0x100182e38 <<bacnet_npdu::view::Npdu>::decode+0x11c>
100182dac:     	str	xzr, [sp, #0x8]
100182db0:     	b	0x100182e4c <<bacnet_npdu::view::Npdu>::decode+0x130>
100182db4:     	add	x0, x20, #0x5
100182db8:     	cmp	x0, x2
100182dbc:     	b.hi	0x100182f28 <<bacnet_npdu::view::Npdu>::decode+0x20c>
100182dc0:     	add	x10, x1, #0x5
100182dc4:     	tbz	w9, #0x3, 0x100182d60 <<bacnet_npdu::view::Npdu>::decode+0x44>
100182dc8:     	add	x13, x0, #0x3
100182dcc:     	cmp	x13, x2
100182dd0:     	b.hi	0x100182f28 <<bacnet_npdu::view::Npdu>::decode+0x20c>
100182dd4:     	add	x11, x1, x0
100182dd8:     	ldrh	w12, [x11]
100182ddc:     	rev	w14, w12
100182de0:     	lsr	w14, w14, #16
100182de4:     	sub	w14, w14, #0x1
100182de8:     	mov	w15, #0xfffe            ; =65534
100182dec:     	cmp	w15, w14, uxth
100182df0:     	b.ls	0x100182e2c <<bacnet_npdu::view::Npdu>::decode+0x110>
100182df4:     	ldrb	w11, [x11, #0x2]
100182df8:     	cbz	w11, 0x100182e78 <<bacnet_npdu::view::Npdu>::decode+0x15c>
100182dfc:     	cmp	w11, #0x12
100182e00:     	b.ls	0x100182e84 <<bacnet_npdu::view::Npdu>::decode+0x168>
100182e04:     	cmp	x10, #0x0
100182e08:     	str	xzr, [x8]
100182e0c:     	cset	w9, ne
100182e10:     	strh	w9, [x8, #0x18]
100182e14:     	strh	w19, [x8, #0x1a]
100182e18:     	mov	w9, #0x1                ; =1
100182e1c:     	strb	w9, [x8, #0x1c]
100182e20:     	strb	w11, [x8, #0x1d]
100182e24:     	mov	w9, #0x2                ; =2
100182e28:     	b	0x100182f34 <<bacnet_npdu::view::Npdu>::decode+0x218>
100182e2c:     	mov	w9, #0x2                ; =2
100182e30:     	strb	w9, [x8]
100182e34:     	b	0x100182f30 <<bacnet_npdu::view::Npdu>::decode+0x214>
100182e38:     	add	x0, sp, #0x8
100182e3c:     	add	x3, x20, #0x5
100182e40:     	mov	x21, x8
100182e44:     	bl	0x100182f70 <bacnet_npdu::view::source_behind>
100182e48:     	mov	x8, x21
100182e4c:     	ldur	q0, [sp, #0x8]
100182e50:     	str	q0, [x8]
100182e54:     	ldr	x9, [sp, #0x18]
100182e58:     	str	x9, [x8, #0x10]
100182e5c:     	mov	w9, #0x1                ; =1
100182e60:     	strh	w9, [x8, #0x18]
100182e64:     	strh	w19, [x8, #0x1a]
100182e68:     	strb	wzr, [x8, #0x1c]
100182e6c:     	strb	w20, [x8, #0x1d]
100182e70:     	mov	w9, #0x2                ; =2
100182e74:     	b	0x100182f34 <<bacnet_npdu::view::Npdu>::decode+0x218>
100182e78:     	mov	w9, #0x3                ; =3
100182e7c:     	strb	w9, [x8]
100182e80:     	b	0x100182f2c <<bacnet_npdu::view::Npdu>::decode+0x210>
100182e84:     	add	x0, x13, x11
100182e88:     	cmp	x0, x2
100182e8c:     	b.hi	0x100182f28 <<bacnet_npdu::view::Npdu>::decode+0x20c>
100182e90:     	rev16	w12, w12
100182e94:     	add	x13, x1, x13
100182e98:     	cbnz	x10, 0x100182d68 <<bacnet_npdu::view::Npdu>::decode+0x4c>
100182e9c:     	mov	w14, #0xff              ; =255
100182ea0:     	tbnz	w9, #0x7, 0x100182ef8 <<bacnet_npdu::view::Npdu>::decode+0x1dc>
100182ea4:     	mov	w16, #0x0               ; =0
100182ea8:     	mov	w3, #0x0                ; =0
100182eac:     	subs	x4, x2, x0
100182eb0:     	b.lo	0x100182f60 <<bacnet_npdu::view::Npdu>::decode+0x244>
100182eb4:     	and	w2, w9, #0x3
100182eb8:     	add	x1, x1, x0
100182ebc:     	ubfx	w9, w9, #2, #1
100182ec0:     	stp	x1, x4, [x8]
100182ec4:     	stp	x10, x20, [x8, #0x10]
100182ec8:     	strh	w19, [x8, #0x20]
100182ecc:     	stp	x13, x11, [x8, #0x28]
100182ed0:     	strh	w12, [x8, #0x38]
100182ed4:     	str	x0, [x8, #0x40]
100182ed8:     	strb	w3, [x8, #0x48]
100182edc:     	strb	w15, [x8, #0x49]
100182ee0:     	strh	w16, [x8, #0x4a]
100182ee4:     	strh	w17, [x8, #0x4c]
100182ee8:     	strb	w14, [x8, #0x4e]
100182eec:     	strb	w2, [x8, #0x4f]
100182ef0:     	strb	w9, [x8, #0x50]
100182ef4:     	b	0x100182f38 <<bacnet_npdu::view::Npdu>::decode+0x21c>
100182ef8:     	cmp	x0, x2
100182efc:     	b.hs	0x100182f28 <<bacnet_npdu::view::Npdu>::decode+0x20c>
100182f00:     	ldrsb	w15, [x1, x0]
100182f04:     	add	x17, x0, #0x1
100182f08:     	tbnz	w15, #0x1f, 0x100182f1c <<bacnet_npdu::view::Npdu>::decode+0x200>
100182f0c:     	mov	w16, #0x0               ; =0
100182f10:     	mov	w3, #0x1                ; =1
100182f14:     	mov	x0, x17
100182f18:     	b	0x100182eac <<bacnet_npdu::view::Npdu>::decode+0x190>
100182f1c:     	add	x0, x0, #0x3
100182f20:     	cmp	x0, x2
100182f24:     	b.ls	0x100182f4c <<bacnet_npdu::view::Npdu>::decode+0x230>
100182f28:     	strb	wzr, [x8]
100182f2c:     	mov	w9, #0x2                ; =2
100182f30:     	strh	w9, [x8, #0x18]
100182f34:     	strh	w9, [x8, #0x4a]
100182f38:     	ldp	x29, x30, [sp, #0x40]
100182f3c:     	ldp	x20, x19, [sp, #0x30]
100182f40:     	ldp	x22, x21, [sp, #0x20]
100182f44:     	add	sp, sp, #0x50
100182f48:     	ret
100182f4c:     	ldrh	w16, [x1, x17]
100182f50:     	rev16	w17, w16
100182f54:     	mov	w16, #0x1               ; =1
100182f58:     	mov	w3, #0x1                ; =1
100182f5c:     	b	0x100182eac <<bacnet_npdu::view::Npdu>::decode+0x190>
100182f60:     	adrp	x3, 0x100274000 <tracing_core::dispatcher::NONE>
100182f64:     	add	x3, x3, #0x1c8
100182f68:     	mov	x1, x2
100182f6c:     	bl	0x1001dcc38 <core::slice::index::slice_index_fail>

