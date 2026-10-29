0000000100181b70 <bytes::bytes::promotable_even_clone>:
100181b70:     	mov	x5, x2
100181b74:     	mov	x4, x1
100181b78:     	ldapr	x2, [x0]
100181b7c:     	tbnz	w2, #0x0, 0x100181ba4 <bytes::bytes::promotable_even_clone+0x34>
100181b80:     	add	x9, x2, #0x10
100181b84:     	mov	w10, #0x1               ; =1
100181b88:     	ldadd	x10, x9, [x9]
100181b8c:     	tbnz	x9, #0x3f, 0x100181bb4 <bytes::bytes::promotable_even_clone+0x44>
100181b90:     	stp	x5, x2, [x8, #0x10]
100181b94:     	adrp	x9, 0x100274000 <_anon.1c2af9719348963797acc74271e67ff2.6+0x10>
100181b98:     	add	x9, x9, #0x130
100181b9c:     	stp	x9, x4, [x8]
100181ba0:     	ret
100181ba4:     	mov	x1, x0
100181ba8:     	and	x3, x2, #0xfffffffffffffffe
100181bac:     	mov	x0, x8
100181bb0:     	b	0x1001d7398 <bytes::bytes::shallow_clone_vec>
100181bb4:     	stp	x29, x30, [sp, #-0x10]!
100181bb8:     	mov	x29, sp
100181bbc:     	bl	0x1001d738c <bytes::abort>

