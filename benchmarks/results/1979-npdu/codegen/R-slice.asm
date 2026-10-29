000000010017e100 <<bytes::bytes::Bytes>::slice::<core::ops::range::RangeFrom<usize>>>:
10017e100:     	sub	sp, sp, #0x60
10017e104:     	stp	x22, x21, [sp, #0x30]
10017e108:     	stp	x20, x19, [sp, #0x40]
10017e10c:     	stp	x29, x30, [sp, #0x50]
10017e110:     	add	x29, sp, #0x50
10017e114:     	ldr	x2, [x0, #0x10]
10017e118:     	stp	x1, x2, [sp]
10017e11c:     	subs	x21, x2, x1
10017e120:     	b.lo	0x10017e184 <<bytes::bytes::Bytes>::slice::<core::ops::range::RangeFrom<usize>>+0x84>
10017e124:     	mov	x20, x1
10017e128:     	mov	x19, x8
10017e12c:     	b.ne	0x10017e150 <<bytes::bytes::Bytes>::slice::<core::ops::range::RangeFrom<usize>>+0x50>
10017e130:     	mov	x21, #0x0               ; =0
10017e134:     	ldr	x8, [x0, #0x8]
10017e138:     	add	x8, x8, x20
10017e13c:     	str	xzr, [x19, #0x18]
10017e140:     	adrp	x9, 0x100273000 <_anon.53300889abeb74b6fbdfa7dcf1d6cfd9.42+0xe8>
10017e144:     	add	x9, x9, #0xd80
10017e148:     	stp	x9, x8, [x19]
10017e14c:     	b	0x10017e16c <<bytes::bytes::Bytes>::slice::<core::ops::range::RangeFrom<usize>>+0x6c>
10017e150:     	ldp	x8, x1, [x0], #0x18
10017e154:     	ldr	x9, [x8]
10017e158:     	mov	x8, x19
10017e15c:     	blr	x9
10017e160:     	ldr	x8, [x19, #0x8]
10017e164:     	add	x8, x8, x20
10017e168:     	str	x8, [x19, #0x8]
10017e16c:     	str	x21, [x19, #0x10]
10017e170:     	ldp	x29, x30, [sp, #0x50]
10017e174:     	ldp	x20, x19, [sp, #0x40]
10017e178:     	ldp	x22, x21, [sp, #0x30]
10017e17c:     	add	sp, sp, #0x60
10017e180:     	ret
10017e184:     	adrp	x8, 0x10017e000 <<&str as core::fmt::Display>::fmt+0x10>
10017e188:     	add	x8, x8, #0x234
10017e18c:     	mov	x9, sp
10017e190:     	stp	x9, x8, [sp, #0x10]
10017e194:     	add	x9, sp, #0x8
10017e198:     	stp	x9, x8, [sp, #0x20]
10017e19c:     	adrp	x0, 0x1001f7000 <_anon.d7fb901a147db7d71026f30bc22a0fa2.3+0x71>
10017e1a0:     	add	x0, x0, #0x7ac
10017e1a4:     	adrp	x2, 0x100273000 <_anon.53300889abeb74b6fbdfa7dcf1d6cfd9.42+0xe8>
10017e1a8:     	add	x2, x2, #0xd68
10017e1ac:     	add	x1, sp, #0x10
10017e1b0:     	bl	0x1001dcbd0 <core::panicking::panic_fmt>
10017e1b4:     	nop
10017e1b8:     	nop
10017e1bc:     	nop
		...

