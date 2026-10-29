00000001001813c8 <bytes::bytes::shared_clone>:
1001813c8:     	ldr	x9, [x0]
1001813cc:     	add	x10, x9, #0x10
1001813d0:     	mov	w11, #0x1               ; =1
1001813d4:     	ldadd	x11, x10, [x10]
1001813d8:     	tbnz	x10, #0x3f, 0x1001813f0 <bytes::bytes::shared_clone+0x28>
1001813dc:     	stp	x2, x9, [x8, #0x10]
1001813e0:     	adrp	x9, 0x100274000 <_anon.1c2af9719348963797acc74271e67ff2.6+0x10>
1001813e4:     	add	x9, x9, #0x130
1001813e8:     	stp	x9, x1, [x8]
1001813ec:     	ret
1001813f0:     	stp	x29, x30, [sp, #-0x10]!
1001813f4:     	mov	x29, sp
1001813f8:     	bl	0x1001d738c <bytes::abort>

