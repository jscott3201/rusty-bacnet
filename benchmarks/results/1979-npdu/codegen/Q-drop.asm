0000000100180c14 <bytes::bytes::shared_drop>:
100180c14:     	sub	sp, sp, #0x30
100180c18:     	stp	x20, x19, [sp, #0x10]
100180c1c:     	stp	x29, x30, [sp, #0x20]
100180c20:     	add	x29, sp, #0x20
100180c24:     	add	x8, x0, #0x10
100180c28:     	mov	x9, #-0x1               ; =-1
100180c2c:     	ldaddl	x9, x8, [x8]
100180c30:     	cmp	x8, #0x1
100180c34:     	b.ne	0x100180c70 <bytes::bytes::shared_drop+0x5c>
100180c38:     	mov	x19, x0
100180c3c:     	ldapur	xzr, [x0, #0x10]
100180c40:     	ldr	x1, [x0, #0x8]
100180c44:     	tbnz	x1, #0x3f, 0x100180c80 <bytes::bytes::shared_drop+0x6c>
100180c48:     	ldr	x0, [x19]
100180c4c:     	mov	w2, #0x1                ; =1
100180c50:     	bl	0x10003091c <__rustc::__rust_dealloc>
100180c54:     	mov	x0, x19
100180c58:     	mov	w1, #0x18               ; =24
100180c5c:     	mov	w2, #0x8                ; =8
100180c60:     	ldp	x29, x30, [sp, #0x20]
100180c64:     	ldp	x20, x19, [sp, #0x10]
100180c68:     	add	sp, sp, #0x30
100180c6c:     	b	0x10003091c <__rustc::__rust_dealloc>
100180c70:     	ldp	x29, x30, [sp, #0x20]
100180c74:     	ldp	x20, x19, [sp, #0x10]
100180c78:     	add	sp, sp, #0x30
100180c7c:     	ret
100180c80:     	adrp	x0, 0x10020f000 <_anon.53300889abeb74b6fbdfa7dcf1d6cfd9.86+0x488>
100180c84:     	add	x0, x0, #0xea2
100180c88:     	adrp	x3, 0x100273000 <_anon.53300889abeb74b6fbdfa7dcf1d6cfd9.42+0xe8>
100180c8c:     	add	x3, x3, #0xff8
100180c90:     	adrp	x4, 0x100274000 <_anon.7022b18dc9d78539c81a31f9b756beb4.7+0x20>
100180c94:     	add	x4, x4, #0xa8
100180c98:     	add	x2, sp, #0xf
100180c9c:     	mov	w1, #0x2b               ; =43
100180ca0:     	bl	0x1001dc57c <core::result::unwrap_failed>
100180ca4:     	brk	#0x1
100180ca8:     	mov	x20, x0
100180cac:     	mov	x0, x19
100180cb0:     	mov	w1, #0x18               ; =24
100180cb4:     	mov	w2, #0x8                ; =8
100180cb8:     	bl	0x10003091c <__rustc::__rust_dealloc>
100180cbc:     	mov	x0, x20
100180cc0:     	bl	0x1001dca98 <dyld_stub_binder+0x1001dca98>

