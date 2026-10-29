0000000100181314 <bytes::bytes::shared_drop>:
100181314:     	sub	sp, sp, #0x30
100181318:     	stp	x20, x19, [sp, #0x10]
10018131c:     	stp	x29, x30, [sp, #0x20]
100181320:     	add	x29, sp, #0x20
100181324:     	add	x8, x0, #0x10
100181328:     	mov	x9, #-0x1               ; =-1
10018132c:     	ldaddl	x9, x8, [x8]
100181330:     	cmp	x8, #0x1
100181334:     	b.ne	0x100181370 <bytes::bytes::shared_drop+0x5c>
100181338:     	mov	x19, x0
10018133c:     	ldapur	xzr, [x0, #0x10]
100181340:     	ldr	x1, [x0, #0x8]
100181344:     	tbnz	x1, #0x3f, 0x100181380 <bytes::bytes::shared_drop+0x6c>
100181348:     	ldr	x0, [x19]
10018134c:     	mov	w2, #0x1                ; =1
100181350:     	bl	0x1000305cc <__rustc::__rust_dealloc>
100181354:     	mov	x0, x19
100181358:     	mov	w1, #0x18               ; =24
10018135c:     	mov	w2, #0x8                ; =8
100181360:     	ldp	x29, x30, [sp, #0x20]
100181364:     	ldp	x20, x19, [sp, #0x10]
100181368:     	add	sp, sp, #0x30
10018136c:     	b	0x1000305cc <__rustc::__rust_dealloc>
100181370:     	ldp	x29, x30, [sp, #0x20]
100181374:     	ldp	x20, x19, [sp, #0x10]
100181378:     	add	sp, sp, #0x30
10018137c:     	ret
100181380:     	adrp	x0, 0x10020f000 <_anon.53300889abeb74b6fbdfa7dcf1d6cfd9.86+0x508>
100181384:     	add	x0, x0, #0xfda
100181388:     	adrp	x3, 0x100274000 <_anon.1c2af9719348963797acc74271e67ff2.6+0x10>
10018138c:     	add	x3, x3, #0x110
100181390:     	adrp	x4, 0x100274000 <_anon.1c2af9719348963797acc74271e67ff2.6+0x10>
100181394:     	add	x4, x4, #0x1c0
100181398:     	add	x2, sp, #0xf
10018139c:     	mov	w1, #0x2b               ; =43
1001813a0:     	bl	0x1001dc53c <core::result::unwrap_failed>
1001813a4:     	brk	#0x1
1001813a8:     	mov	x20, x0
1001813ac:     	mov	x0, x19
1001813b0:     	mov	w1, #0x18               ; =24
1001813b4:     	mov	w2, #0x8                ; =8
1001813b8:     	bl	0x1000305cc <__rustc::__rust_dealloc>
1001813bc:     	mov	x0, x20
1001813c0:     	bl	0x1001dca58 <dyld_stub_binder+0x1001dca58>

