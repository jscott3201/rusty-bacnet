000000010017bc80 <<bytes::bytes::Bytes>::slice::<core::ops::range::RangeFrom<usize>>>:
10017bc80:     	sub	sp, sp, #0x60
10017bc84:     	stp	x22, x21, [sp, #0x30]
10017bc88:     	stp	x20, x19, [sp, #0x40]
10017bc8c:     	stp	x29, x30, [sp, #0x50]
10017bc90:     	add	x29, sp, #0x50
10017bc94:     	ldr	x2, [x0, #0x10]
10017bc98:     	stp	x1, x2, [sp]
10017bc9c:     	subs	x21, x2, x1
10017bca0:     	b.lo	0x10017bd04 <<bytes::bytes::Bytes>::slice::<core::ops::range::RangeFrom<usize>>+0x84>
10017bca4:     	mov	x20, x1
10017bca8:     	mov	x19, x8
10017bcac:     	b.ne	0x10017bcd0 <<bytes::bytes::Bytes>::slice::<core::ops::range::RangeFrom<usize>>+0x50>
10017bcb0:     	mov	x21, #0x0               ; =0
10017bcb4:     	ldr	x8, [x0, #0x8]
10017bcb8:     	add	x8, x8, x20
10017bcbc:     	str	xzr, [x19, #0x18]
10017bcc0:     	adrp	x9, 0x100273000 <_anon.53300889abeb74b6fbdfa7dcf1d6cfd9.42+0xe8>
10017bcc4:     	add	x9, x9, #0xba8
10017bcc8:     	stp	x9, x8, [x19]
10017bccc:     	b	0x10017bcec <<bytes::bytes::Bytes>::slice::<core::ops::range::RangeFrom<usize>>+0x6c>
10017bcd0:     	ldp	x8, x1, [x0], #0x18
10017bcd4:     	ldr	x9, [x8]
10017bcd8:     	mov	x8, x19
10017bcdc:     	blr	x9
10017bce0:     	ldr	x8, [x19, #0x8]
10017bce4:     	add	x8, x8, x20
10017bce8:     	str	x8, [x19, #0x8]
10017bcec:     	str	x21, [x19, #0x10]
10017bcf0:     	ldp	x29, x30, [sp, #0x50]
10017bcf4:     	ldp	x20, x19, [sp, #0x40]
10017bcf8:     	ldp	x22, x21, [sp, #0x30]
10017bcfc:     	add	sp, sp, #0x60
10017bd00:     	ret
10017bd04:     	adrp	x8, 0x10017b000 <bacnet_encoding::constructed::tagged::next_is_context+0x28>
10017bd08:     	add	x8, x8, #0xdb4
10017bd0c:     	mov	x9, sp
10017bd10:     	stp	x9, x8, [sp, #0x10]
10017bd14:     	add	x9, sp, #0x8
10017bd18:     	stp	x9, x8, [sp, #0x20]
10017bd1c:     	adrp	x0, 0x1001f7000 <_anon.53300889abeb74b6fbdfa7dcf1d6cfd9.1+0x25a>
10017bd20:     	add	x0, x0, #0x2f4
10017bd24:     	adrp	x2, 0x100273000 <_anon.53300889abeb74b6fbdfa7dcf1d6cfd9.42+0xe8>
10017bd28:     	add	x2, x2, #0xb90
10017bd2c:     	add	x1, sp, #0x10
10017bd30:     	bl	0x1001dc7d0 <core::panicking::panic_fmt>
10017bd34:     	nop
10017bd38:     	nop
10017bd3c:     	nop
		...

