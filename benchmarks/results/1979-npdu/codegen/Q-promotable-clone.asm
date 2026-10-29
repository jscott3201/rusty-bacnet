0000000100181470 <bytes::bytes::promotable_even_clone>:
100181470:     	mov	x5, x2
100181474:     	mov	x4, x1
100181478:     	ldapr	x2, [x0]
10018147c:     	tbnz	w2, #0x0, 0x1001814a4 <bytes::bytes::promotable_even_clone+0x34>
100181480:     	add	x9, x2, #0x10
100181484:     	mov	w10, #0x1               ; =1
100181488:     	ldadd	x10, x9, [x9]
10018148c:     	tbnz	x9, #0x3f, 0x1001814b4 <bytes::bytes::promotable_even_clone+0x44>
100181490:     	stp	x5, x2, [x8, #0x10]
100181494:     	adrp	x9, 0x100274000 <_anon.7022b18dc9d78539c81a31f9b756beb4.7+0x20>
100181498:     	add	x9, x9, #0x18
10018149c:     	stp	x9, x4, [x8]
1001814a0:     	ret
1001814a4:     	mov	x1, x0
1001814a8:     	and	x3, x2, #0xfffffffffffffffe
1001814ac:     	mov	x0, x8
1001814b0:     	b	0x1001d73d8 <bytes::bytes::shallow_clone_vec>
1001814b4:     	stp	x29, x30, [sp, #-0x10]!
1001814b8:     	mov	x29, sp
1001814bc:     	bl	0x1001d73cc <bytes::abort>

