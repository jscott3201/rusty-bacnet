0000000100180cc8 <bytes::bytes::shared_clone>:
100180cc8:     	ldr	x9, [x0]
100180ccc:     	add	x10, x9, #0x10
100180cd0:     	mov	w11, #0x1               ; =1
100180cd4:     	ldadd	x11, x10, [x10]
100180cd8:     	tbnz	x10, #0x3f, 0x100180cf0 <bytes::bytes::shared_clone+0x28>
100180cdc:     	stp	x2, x9, [x8, #0x10]
100180ce0:     	adrp	x9, 0x100274000 <_anon.7022b18dc9d78539c81a31f9b756beb4.7+0x20>
100180ce4:     	add	x9, x9, #0x18
100180ce8:     	stp	x9, x1, [x8]
100180cec:     	ret
100180cf0:     	stp	x29, x30, [sp, #-0x10]!
100180cf4:     	mov	x29, sp
100180cf8:     	bl	0x1001d73cc <bytes::abort>

