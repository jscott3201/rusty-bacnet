00000001001810c8 <bytes::bytes::shared_clone>:
1001810c8:     	ldr	x9, [x0]
1001810cc:     	add	x10, x9, #0x10
1001810d0:     	mov	w11, #0x1               ; =1
1001810d4:     	ldadd	x11, x10, [x10]
1001810d8:     	tbnz	x10, #0x3f, 0x1001810f0 <bytes::bytes::shared_clone+0x28>
1001810dc:     	stp	x2, x9, [x8, #0x10]
1001810e0:     	adrp	x9, 0x100274000 <tracing_core::dispatcher::NONE>
1001810e4:     	add	x9, x9, #0xa8
1001810e8:     	stp	x9, x1, [x8]
1001810ec:     	ret
1001810f0:     	stp	x29, x30, [sp, #-0x10]!
1001810f4:     	mov	x29, sp
1001810f8:     	bl	0x1001d77cc <bytes::abort>

