000000010017e480 <<bytes::bytes::Bytes>::slice::<core::ops::range::RangeFrom<usize>>>:
10017e480:     	sub	sp, sp, #0x60
10017e484:     	stp	x22, x21, [sp, #0x30]
10017e488:     	stp	x20, x19, [sp, #0x40]
10017e48c:     	stp	x29, x30, [sp, #0x50]
10017e490:     	add	x29, sp, #0x50
10017e494:     	ldr	x2, [x0, #0x10]
10017e498:     	stp	x1, x2, [sp]
10017e49c:     	subs	x21, x2, x1
10017e4a0:     	b.lo	0x10017e504 <<bytes::bytes::Bytes>::slice::<core::ops::range::RangeFrom<usize>>+0x84>
10017e4a4:     	mov	x20, x1
10017e4a8:     	mov	x19, x8
10017e4ac:     	b.ne	0x10017e4d0 <<bytes::bytes::Bytes>::slice::<core::ops::range::RangeFrom<usize>>+0x50>
10017e4b0:     	mov	x21, #0x0               ; =0
10017e4b4:     	ldr	x8, [x0, #0x8]
10017e4b8:     	add	x8, x8, x20
10017e4bc:     	str	xzr, [x19, #0x18]
10017e4c0:     	adrp	x9, 0x100273000 <_anon.53300889abeb74b6fbdfa7dcf1d6cfd9.42+0x30>
10017e4c4:     	add	x9, x9, #0xe20
10017e4c8:     	stp	x9, x8, [x19]
10017e4cc:     	b	0x10017e4ec <<bytes::bytes::Bytes>::slice::<core::ops::range::RangeFrom<usize>>+0x6c>
10017e4d0:     	ldp	x8, x1, [x0], #0x18
10017e4d4:     	ldr	x9, [x8]
10017e4d8:     	mov	x8, x19
10017e4dc:     	blr	x9
10017e4e0:     	ldr	x8, [x19, #0x8]
10017e4e4:     	add	x8, x8, x20
10017e4e8:     	str	x8, [x19, #0x8]
10017e4ec:     	str	x21, [x19, #0x10]
10017e4f0:     	ldp	x29, x30, [sp, #0x50]
10017e4f4:     	ldp	x20, x19, [sp, #0x40]
10017e4f8:     	ldp	x22, x21, [sp, #0x30]
10017e4fc:     	add	sp, sp, #0x60
10017e500:     	ret
10017e504:     	adrp	x8, 0x10017e000 <bacnet_encoding::tags::extract_context_value+0x1ac>
10017e508:     	add	x8, x8, #0x574
10017e50c:     	mov	x9, sp
10017e510:     	stp	x9, x8, [sp, #0x10]
10017e514:     	add	x9, sp, #0x8
10017e518:     	stp	x9, x8, [sp, #0x20]
10017e51c:     	adrp	x0, 0x1001f7000 <_anon.e81895284fd2357c11254d988870f03b.3+0x3b>
10017e520:     	add	x0, x0, #0x3ca
10017e524:     	adrp	x2, 0x100273000 <_anon.53300889abeb74b6fbdfa7dcf1d6cfd9.42+0x30>
10017e528:     	add	x2, x2, #0xe08
10017e52c:     	add	x1, sp, #0x10
10017e530:     	bl	0x1001dc790 <core::panicking::panic_fmt>
10017e534:     	udf	#0x0

