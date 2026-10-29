000000010017c128 <bacnet_encoding::npdu::decode_npdu>:
10017c128:     	sub	sp, sp, #0x170
10017c12c:     	stp	x28, x27, [sp, #0x110]
10017c130:     	stp	x26, x25, [sp, #0x120]
10017c134:     	stp	x24, x23, [sp, #0x130]
10017c138:     	stp	x22, x21, [sp, #0x140]
10017c13c:     	stp	x20, x19, [sp, #0x150]
10017c140:     	stp	x29, x30, [sp, #0x160]
10017c144:     	add	x29, sp, #0x160
10017c148:     	mov	x19, x0
10017c14c:     	mov	x21, x8
10017c150:     	ldp	x22, x20, [x0, #0x8]
10017c154:     	cmp	x20, #0x2
10017c158:     	b.lo	0x10017c204 <bacnet_encoding::npdu::decode_npdu+0xdc>
10017c15c:     	ldrb	w8, [x22, #0x1]
10017c160:     	strb	w8, [sp, #0x67]
10017c164:     	mov	w9, #0x50               ; =80
10017c168:     	tst	w8, w9
10017c16c:     	b.eq	0x10017c204 <bacnet_encoding::npdu::decode_npdu+0xdc>
10017c170:     	adrp	x8, 0x100278000 <dyld_stub_binder+0x100278000>
10017c174:     	add	x8, x8, #0x568
10017c178:     	ldr	x8, [x8]
10017c17c:     	cmp	x8, #0x3
10017c180:     	b.hi	0x10017c204 <bacnet_encoding::npdu::decode_npdu+0xdc>
10017c184:     	adrp	x8, 0x100278000 <dyld_stub_binder+0x100278000>
10017c188:     	ldrb	w1, [x8, #0x560]
10017c18c:     	sub	w8, w1, #0x1
10017c190:     	cmp	w8, #0x2
10017c194:     	b.hs	0x10017c200 <bacnet_encoding::npdu::decode_npdu+0xd8>
10017c198:     	adrp	x23, 0x100278000 <dyld_stub_binder+0x100278000>
10017c19c:     	ldr	x0, [x23, #0x550]
10017c1a0:     	bl	0x10017f778 <tracing::__macro_support::__is_enabled>
10017c1a4:     	cbz	w0, 0x10017c204 <bacnet_encoding::npdu::decode_npdu+0xdc>
10017c1a8:     	ldr	x0, [x23, #0x550]
10017c1ac:     	add	x8, x0, #0x30
10017c1b0:     	adrp	x9, 0x10020f000 <_anon.53300889abeb74b6fbdfa7dcf1d6cfd9.86+0x488>
10017c1b4:     	add	x9, x9, #0x740
10017c1b8:     	mov	w10, #0x6b              ; =107
10017c1bc:     	stp	x9, x10, [sp, #0x88]
10017c1c0:     	add	x9, sp, #0x88
10017c1c4:     	adrp	x10, 0x100273000 <_anon.53300889abeb74b6fbdfa7dcf1d6cfd9.42+0xe8>
10017c1c8:     	add	x10, x10, #0xbd0
10017c1cc:     	stp	x9, x10, [sp, #0x68]
10017c1d0:     	add	x9, sp, #0x67
10017c1d4:     	adrp	x10, 0x100273000 <_anon.53300889abeb74b6fbdfa7dcf1d6cfd9.42+0xe8>
10017c1d8:     	add	x10, x10, #0xbf0
10017c1dc:     	stp	x9, x10, [sp, #0x78]
10017c1e0:     	mov	w9, #0x1                ; =1
10017c1e4:     	add	x10, sp, #0x68
10017c1e8:     	stp	x9, x10, [sp, #0x98]
10017c1ec:     	mov	w9, #0x2                ; =2
10017c1f0:     	stp	x9, x8, [sp, #0xa8]
10017c1f4:     	add	x1, sp, #0x98
10017c1f8:     	bl	0x10017fc20 <<tracing_core::event::Event>::dispatch>
10017c1fc:     	b	0x10017c204 <bacnet_encoding::npdu::decode_npdu+0xdc>
10017c200:     	cbnz	w1, 0x10017c584 <bacnet_encoding::npdu::decode_npdu+0x45c>
10017c204:     	add	x8, sp, #0x98
10017c208:     	mov	x0, x22
10017c20c:     	mov	x1, x20
10017c210:     	bl	0x10018291c <<bacnet_npdu::view::Npdu>::decode>
10017c214:     	ldrh	w25, [sp, #0xe2]
10017c218:     	cmp	w25, #0x2
10017c21c:     	b.ne	0x10017c240 <bacnet_encoding::npdu::decode_npdu+0x118>
10017c220:     	ldrh	w26, [sp, #0xb0]
10017c224:     	cmp	w26, #0x2
10017c228:     	b.ne	0x10017c2b8 <bacnet_encoding::npdu::decode_npdu+0x190>
10017c22c:     	ldrb	w8, [sp, #0x98]
10017c230:     	mov	w9, #0x2                ; =2
10017c234:     	str	x9, [x21, #0x8]
10017c238:     	strb	w8, [x21, #0x10]
10017c23c:     	b	0x10017c508 <bacnet_encoding::npdu::decode_npdu+0x3e0>
10017c240:     	ldrb	w8, [sp, #0xe8]
10017c244:     	ldrb	w10, [sp, #0xe7]
10017c248:     	ldr	x28, [sp, #0xa8]
10017c24c:     	ldp	x27, x24, [sp, #0xc0]
10017c250:     	ldrh	w9, [sp, #0xd0]
10017c254:     	str	w9, [sp, #0x30]
10017c258:     	ldrb	w9, [sp, #0xe6]
10017c25c:     	stp	w9, w10, [sp, #0x58]
10017c260:     	ldrb	w10, [sp, #0xe0]
10017c264:     	ldrb	w9, [sp, #0xe1]
10017c268:     	stp	w9, w10, [sp, #0x50]
10017c26c:     	ldrh	w9, [sp, #0xe4]
10017c270:     	str	w9, [sp, #0x4c]
10017c274:     	ldr	x26, [sp, #0xd8]
10017c278:     	str	w8, [sp, #0x60]
10017c27c:     	str	x27, [sp, #0x38]
10017c280:     	cbz	x28, 0x10017c304 <bacnet_encoding::npdu::decode_npdu+0x1dc>
10017c284:     	ldr	x23, [sp, #0xb0]
10017c288:     	ldrh	w27, [sp, #0xb8]
10017c28c:     	cmp	x23, #0x7
10017c290:     	b.hs	0x10017c318 <bacnet_encoding::npdu::decode_npdu+0x1f0>
10017c294:     	add	x0, sp, #0x98
10017c298:     	mov	x1, x28
10017c29c:     	mov	x2, x23
10017c2a0:     	bl	0x1001dcd2c <dyld_stub_binder+0x1001dcd2c>
10017c2a4:     	str	xzr, [sp, #0x28]
10017c2a8:     	str	x23, [sp, #0xa0]
10017c2ac:     	ldr	x8, [sp, #0x98]
10017c2b0:     	str	x8, [sp, #0x18]
10017c2b4:     	b	0x10017c344 <bacnet_encoding::npdu::decode_npdu+0x21c>
10017c2b8:     	sub	x28, x29, #0x70
10017c2bc:     	ldrb	w9, [sp, #0xb4]
10017c2c0:     	ldrb	w10, [sp, #0xb5]
10017c2c4:     	ldrh	w27, [sp, #0xb2]
10017c2c8:     	ldr	x25, [sp, #0x98]
10017c2cc:     	cbz	x25, 0x10017c4a4 <bacnet_encoding::npdu::decode_npdu+0x37c>
10017c2d0:     	stp	w10, w9, [sp, #0x5c]
10017c2d4:     	ldrh	w8, [sp, #0xa8]
10017c2d8:     	str	w8, [sp, #0x58]
10017c2dc:     	ldr	x23, [sp, #0xa0]
10017c2e0:     	cmp	x23, #0x7
10017c2e4:     	b.hs	0x10017c4ac <bacnet_encoding::npdu::decode_npdu+0x384>
10017c2e8:     	sub	x0, x29, #0x70
10017c2ec:     	mov	x1, x25
10017c2f0:     	mov	x2, x23
10017c2f4:     	bl	0x1001dcd2c <dyld_stub_binder+0x1001dcd2c>
10017c2f8:     	mov	x24, #0x0               ; =0
10017c2fc:     	str	x23, [x28, #0x8]
10017c300:     	b	0x10017c4d4 <bacnet_encoding::npdu::decode_npdu+0x3ac>
10017c304:     	str	xzr, [sp, #0x40]
10017c308:     	str	x28, [sp, #0x8]
10017c30c:     	cbnz	x27, 0x10017c360 <bacnet_encoding::npdu::decode_npdu+0x238>
10017c310:     	mov	x28, #0x0               ; =0
10017c314:     	b	0x10017c3c4 <bacnet_encoding::npdu::decode_npdu+0x29c>
10017c318:     	bl	0x100030928 <__rustc::__rust_no_alloc_shim_is_unstable_v2>
10017c31c:     	mov	x0, x23
10017c320:     	mov	w1, #0x1                ; =1
10017c324:     	bl	0x100030918 <__rustc::__rust_alloc>
10017c328:     	cbz	x0, 0x10017c59c <bacnet_encoding::npdu::decode_npdu+0x474>
10017c32c:     	str	x0, [sp, #0x28]
10017c330:     	mov	x1, x28
10017c334:     	mov	x2, x23
10017c338:     	bl	0x1001dcd2c <dyld_stub_binder+0x1001dcd2c>
10017c33c:     	stp	x23, x23, [sp, #0x98]
10017c340:     	str	x23, [sp, #0x18]
10017c344:     	strh	w27, [sp, #0xa8]
10017c348:     	ldr	x8, [sp, #0xa8]
10017c34c:     	stp	x28, x8, [sp, #0x8]
10017c350:     	mov	w8, #0x1                ; =1
10017c354:     	str	x8, [sp, #0x40]
10017c358:     	ldr	x27, [sp, #0x38]
10017c35c:     	cbz	x27, 0x10017c310 <bacnet_encoding::npdu::decode_npdu+0x1e8>
10017c360:     	cmp	x24, #0x7
10017c364:     	b.hs	0x10017c388 <bacnet_encoding::npdu::decode_npdu+0x260>
10017c368:     	add	x0, sp, #0x98
10017c36c:     	mov	x1, x27
10017c370:     	mov	x2, x24
10017c374:     	bl	0x1001dcd2c <dyld_stub_binder+0x1001dcd2c>
10017c378:     	str	xzr, [sp, #0x20]
10017c37c:     	str	x24, [sp, #0xa0]
10017c380:     	ldr	x10, [sp, #0x98]
10017c384:     	b	0x10017c3b4 <bacnet_encoding::npdu::decode_npdu+0x28c>
10017c388:     	bl	0x100030928 <__rustc::__rust_no_alloc_shim_is_unstable_v2>
10017c38c:     	mov	x0, x24
10017c390:     	mov	w1, #0x1                ; =1
10017c394:     	bl	0x100030918 <__rustc::__rust_alloc>
10017c398:     	cbz	x0, 0x10017c5ac <bacnet_encoding::npdu::decode_npdu+0x484>
10017c39c:     	str	x0, [sp, #0x20]
10017c3a0:     	mov	x1, x27
10017c3a4:     	mov	x2, x24
10017c3a8:     	bl	0x1001dcd2c <dyld_stub_binder+0x1001dcd2c>
10017c3ac:     	stp	x24, x24, [sp, #0x98]
10017c3b0:     	mov	x10, x24
10017c3b4:     	ldr	w8, [sp, #0x30]
10017c3b8:     	strh	w8, [sp, #0xa8]
10017c3bc:     	ldr	x11, [sp, #0xa8]
10017c3c0:     	mov	w28, #0x1               ; =1
10017c3c4:     	str	x26, [sp, #0x88]
10017c3c8:     	str	x20, [sp, #0x68]
10017c3cc:     	subs	x27, x20, x26
10017c3d0:     	b.lo	0x10017c548 <bacnet_encoding::npdu::decode_npdu+0x420>
10017c3d4:     	b.ne	0x10017c3f4 <bacnet_encoding::npdu::decode_npdu+0x2cc>
10017c3d8:     	mov	x27, #0x0               ; =0
10017c3dc:     	add	x8, x22, x20
10017c3e0:     	str	xzr, [sp, #0xb0]
10017c3e4:     	adrp	x9, 0x100273000 <_anon.53300889abeb74b6fbdfa7dcf1d6cfd9.42+0xe8>
10017c3e8:     	add	x9, x9, #0xba8
10017c3ec:     	stp	x9, x8, [sp, #0x98]
10017c3f0:     	b	0x10017c440 <bacnet_encoding::npdu::decode_npdu+0x318>
10017c3f4:     	str	x23, [sp, #0x30]
10017c3f8:     	mov	x23, x11
10017c3fc:     	str	x24, [sp]
10017c400:     	mov	x24, x10
10017c404:     	mov	x0, x19
10017c408:     	ldr	x8, [x0], #0x18
10017c40c:     	ldr	x9, [x8]
10017c410:     	add	x8, sp, #0x98
10017c414:     	mov	x1, x22
10017c418:     	mov	x2, x20
10017c41c:     	blr	x9
10017c420:     	ldr	x8, [sp, #0xa0]
10017c424:     	add	x8, x8, x26
10017c428:     	str	x8, [sp, #0xa0]
10017c42c:     	ldp	x22, x20, [x19, #0x8]
10017c430:     	mov	x10, x24
10017c434:     	ldr	x24, [sp]
10017c438:     	mov	x11, x23
10017c43c:     	ldr	x23, [sp, #0x30]
10017c440:     	str	x27, [sp, #0xa8]
10017c444:     	ldur	q0, [sp, #0x98]
10017c448:     	ldur	q1, [sp, #0xa8]
10017c44c:     	stp	q0, q1, [x21, #0x50]
10017c450:     	ldr	x9, [sp, #0x40]
10017c454:     	ldr	x8, [sp, #0x28]
10017c458:     	stp	x9, x8, [x21]
10017c45c:     	ldp	x8, x9, [sp, #0x10]
10017c460:     	stp	x9, x23, [x21, #0x10]
10017c464:     	stp	x8, x28, [x21, #0x20]
10017c468:     	ldr	x8, [sp, #0x20]
10017c46c:     	stp	x8, x10, [x21, #0x30]
10017c470:     	stp	x24, x11, [x21, #0x40]
10017c474:     	strh	w25, [x21, #0x70]
10017c478:     	ldr	w8, [sp, #0x4c]
10017c47c:     	strh	w8, [x21, #0x72]
10017c480:     	ldp	w8, w9, [sp, #0x50]
10017c484:     	strb	w9, [x21, #0x74]
10017c488:     	strb	w8, [x21, #0x75]
10017c48c:     	ldp	w8, w9, [sp, #0x5c]
10017c490:     	strb	w9, [x21, #0x76]
10017c494:     	strb	w8, [x21, #0x77]
10017c498:     	ldr	w8, [sp, #0x58]
10017c49c:     	strb	w8, [x21, #0x78]
10017c4a0:     	b	0x10017c510 <bacnet_encoding::npdu::decode_npdu+0x3e8>
10017c4a4:     	mov	x8, #0x0                ; =0
10017c4a8:     	b	0x10017c4e4 <bacnet_encoding::npdu::decode_npdu+0x3bc>
10017c4ac:     	bl	0x100030928 <__rustc::__rust_no_alloc_shim_is_unstable_v2>
10017c4b0:     	mov	x0, x23
10017c4b4:     	mov	w1, #0x1                ; =1
10017c4b8:     	bl	0x100030918 <__rustc::__rust_alloc>
10017c4bc:     	cbz	x0, 0x10017c59c <bacnet_encoding::npdu::decode_npdu+0x474>
10017c4c0:     	mov	x24, x0
10017c4c4:     	mov	x1, x25
10017c4c8:     	mov	x2, x23
10017c4cc:     	bl	0x1001dcd2c <dyld_stub_binder+0x1001dcd2c>
10017c4d0:     	stp	x23, x23, [x28]
10017c4d4:     	ldp	w8, w10, [sp, #0x58]
10017c4d8:     	strh	w8, [x28, #0x10]
10017c4dc:     	mov	w8, #0x1                ; =1
10017c4e0:     	ldr	w9, [sp, #0x60]
10017c4e4:     	stp	x8, x24, [x21, #0x8]
10017c4e8:     	ldr	q0, [x28]
10017c4ec:     	stur	q0, [x21, #0x18]
10017c4f0:     	ldr	x8, [x28, #0x10]
10017c4f4:     	str	x8, [x21, #0x28]
10017c4f8:     	strh	w26, [x21, #0x30]
10017c4fc:     	strh	w27, [x21, #0x32]
10017c500:     	strb	w9, [x21, #0x34]
10017c504:     	strb	w10, [x21, #0x35]
10017c508:     	mov	w8, #0x2                ; =2
10017c50c:     	str	x8, [x21]
10017c510:     	ldr	x0, [x19, #0x18]
10017c514:     	ldr	x8, [x19]
10017c518:     	ldr	x8, [x8, #0x20]
10017c51c:     	mov	x1, x22
10017c520:     	mov	x2, x20
10017c524:     	blr	x8
10017c528:     	ldp	x29, x30, [sp, #0x160]
10017c52c:     	ldp	x20, x19, [sp, #0x150]
10017c530:     	ldp	x22, x21, [sp, #0x140]
10017c534:     	ldp	x24, x23, [sp, #0x130]
10017c538:     	ldp	x26, x25, [sp, #0x120]
10017c53c:     	ldp	x28, x27, [sp, #0x110]
10017c540:     	add	sp, sp, #0x170
10017c544:     	ret
10017c548:     	str	x23, [sp, #0x30]
10017c54c:     	str	x24, [sp]
10017c550:     	adrp	x8, 0x10017b000 <bacnet_encoding::constructed::tagged::next_is_context+0x28>
10017c554:     	add	x8, x8, #0xdb4
10017c558:     	add	x9, sp, #0x88
10017c55c:     	stp	x9, x8, [sp, #0x98]
10017c560:     	add	x9, sp, #0x68
10017c564:     	stp	x9, x8, [sp, #0xa8]
10017c568:     	adrp	x0, 0x1001f7000 <_anon.53300889abeb74b6fbdfa7dcf1d6cfd9.1+0x25a>
10017c56c:     	add	x0, x0, #0x2f4
10017c570:     	adrp	x2, 0x100273000 <_anon.53300889abeb74b6fbdfa7dcf1d6cfd9.42+0xe8>
10017c574:     	add	x2, x2, #0xb90
10017c578:     	add	x1, sp, #0x98
10017c57c:     	bl	0x1001dc7d0 <core::panicking::panic_fmt>
10017c580:     	b	0x10017c5b8 <bacnet_encoding::npdu::decode_npdu+0x490>
10017c584:     	adrp	x0, 0x100278000 <dyld_stub_binder+0x100278000>
10017c588:     	add	x0, x0, #0x550
10017c58c:     	bl	0x1001d6f80 <<tracing_core::callsite::DefaultCallsite>::register>
10017c590:     	and	w1, w0, #0xff
10017c594:     	cbnz	w1, 0x10017c198 <bacnet_encoding::npdu::decode_npdu+0x70>
10017c598:     	b	0x10017c204 <bacnet_encoding::npdu::decode_npdu+0xdc>
10017c59c:     	mov	x1, x23
10017c5a0:     	mov	w0, #0x1                ; =1
10017c5a4:     	bl	0x1001dc2ec <alloc::raw_vec::handle_error>
10017c5a8:     	b	0x10017c5b8 <bacnet_encoding::npdu::decode_npdu+0x490>
10017c5ac:     	mov	w0, #0x1                ; =1
10017c5b0:     	mov	x1, x24
10017c5b4:     	bl	0x1001dc2ec <alloc::raw_vec::handle_error>
10017c5b8:     	brk	#0x1
10017c5bc:     	mov	x20, x0
10017c5c0:     	b	0x10017c600 <bacnet_encoding::npdu::decode_npdu+0x4d8>
10017c5c4:     	mov	x20, x0
10017c5c8:     	b	0x10017c61c <bacnet_encoding::npdu::decode_npdu+0x4f4>
10017c5cc:     	mov	x20, x0
10017c5d0:     	ldr	x8, [sp, #0x38]
10017c5d4:     	cbnz	x8, 0x10017c5e4 <bacnet_encoding::npdu::decode_npdu+0x4bc>
10017c5d8:     	ldr	x28, [sp, #0x8]
10017c5dc:     	ldr	x23, [sp, #0x30]
10017c5e0:     	b	0x10017c600 <bacnet_encoding::npdu::decode_npdu+0x4d8>
10017c5e4:     	ldp	x1, x28, [sp]
10017c5e8:     	cmp	x1, #0x7
10017c5ec:     	ldr	x23, [sp, #0x30]
10017c5f0:     	b.lo	0x10017c600 <bacnet_encoding::npdu::decode_npdu+0x4d8>
10017c5f4:     	ldr	x0, [sp, #0x20]
10017c5f8:     	mov	w2, #0x1                ; =1
10017c5fc:     	bl	0x10003091c <__rustc::__rust_dealloc>
10017c600:     	cbz	x28, 0x10017c61c <bacnet_encoding::npdu::decode_npdu+0x4f4>
10017c604:     	cmp	x23, #0x7
10017c608:     	b.lo	0x10017c61c <bacnet_encoding::npdu::decode_npdu+0x4f4>
10017c60c:     	ldr	x0, [sp, #0x28]
10017c610:     	mov	x1, x23
10017c614:     	mov	w2, #0x1                ; =1
10017c618:     	bl	0x10003091c <__rustc::__rust_dealloc>
10017c61c:     	ldp	x8, x1, [x19]
10017c620:     	ldr	x8, [x8, #0x20]
10017c624:     	ldp	x2, x0, [x19, #0x10]
10017c628:     	blr	x8
10017c62c:     	mov	x0, x20
10017c630:     	bl	0x1001dca98 <dyld_stub_binder+0x1001dca98>
10017c634:     	bl	0x1001dc620 <core::panicking::panic_in_cleanup>

