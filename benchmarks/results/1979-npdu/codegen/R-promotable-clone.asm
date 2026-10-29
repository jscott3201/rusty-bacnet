0000000100181870 <bytes::bytes::promotable_even_clone>:
100181870:     	mov	x5, x2
100181874:     	mov	x4, x1
100181878:     	ldapr	x2, [x0]
10018187c:     	tbnz	w2, #0x0, 0x1001818a4 <bytes::bytes::promotable_even_clone+0x34>
100181880:     	add	x9, x2, #0x10
100181884:     	mov	w10, #0x1               ; =1
100181888:     	ldadd	x10, x9, [x9]
10018188c:     	tbnz	x9, #0x3f, 0x1001818b4 <bytes::bytes::promotable_even_clone+0x44>
100181890:     	stp	x5, x2, [x8, #0x10]
100181894:     	adrp	x9, 0x100274000 <tracing_core::dispatcher::NONE>
100181898:     	add	x9, x9, #0xa8
10018189c:     	stp	x9, x4, [x8]
1001818a0:     	ret
1001818a4:     	mov	x1, x0
1001818a8:     	and	x3, x2, #0xfffffffffffffffe
1001818ac:     	mov	x0, x8
1001818b0:     	b	0x1001d77d8 <bytes::bytes::shallow_clone_vec>
1001818b4:     	stp	x29, x30, [sp, #-0x10]!
1001818b8:     	mov	x29, sp
1001818bc:     	bl	0x1001d77cc <bytes::abort>

