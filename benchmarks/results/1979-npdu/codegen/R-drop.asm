0000000100181014 <bytes::bytes::shared_drop>:
100181014:     	sub	sp, sp, #0x30
100181018:     	stp	x20, x19, [sp, #0x10]
10018101c:     	stp	x29, x30, [sp, #0x20]
100181020:     	add	x29, sp, #0x20
100181024:     	add	x8, x0, #0x10
100181028:     	mov	x9, #-0x1               ; =-1
10018102c:     	ldaddl	x9, x8, [x8]
100181030:     	cmp	x8, #0x1
100181034:     	b.ne	0x100181070 <bytes::bytes::shared_drop+0x5c>
100181038:     	mov	x19, x0
10018103c:     	ldapur	xzr, [x0, #0x10]
100181040:     	ldr	x1, [x0, #0x8]
100181044:     	tbnz	x1, #0x3f, 0x100181080 <bytes::bytes::shared_drop+0x6c>
100181048:     	ldr	x0, [x19]
10018104c:     	mov	w2, #0x1                ; =1
100181050:     	bl	0x10003091c <__rustc::__rust_dealloc>
100181054:     	mov	x0, x19
100181058:     	mov	w1, #0x18               ; =24
10018105c:     	mov	w2, #0x8                ; =8
100181060:     	ldp	x29, x30, [sp, #0x20]
100181064:     	ldp	x20, x19, [sp, #0x10]
100181068:     	add	sp, sp, #0x30
10018106c:     	b	0x10003091c <__rustc::__rust_dealloc>
100181070:     	ldp	x29, x30, [sp, #0x20]
100181074:     	ldp	x20, x19, [sp, #0x10]
100181078:     	add	sp, sp, #0x30
10018107c:     	ret
100181080:     	adrp	x0, 0x100210000 <_anon.0f96debfcf33b16ac11c1398d1a7ca42.20+0x2a9>
100181084:     	add	x0, x0, #0x2a2
100181088:     	adrp	x3, 0x100274000 <tracing_core::dispatcher::NONE>
10018108c:     	add	x3, x3, #0x88
100181090:     	adrp	x4, 0x100274000 <tracing_core::dispatcher::NONE>
100181094:     	add	x4, x4, #0x138
100181098:     	add	x2, sp, #0xf
10018109c:     	mov	w1, #0x2b               ; =43
1001810a0:     	bl	0x1001dc97c <core::result::unwrap_failed>
1001810a4:     	brk	#0x1
1001810a8:     	mov	x20, x0
1001810ac:     	mov	x0, x19
1001810b0:     	mov	w1, #0x18               ; =24
1001810b4:     	mov	w2, #0x8                ; =8
1001810b8:     	bl	0x10003091c <__rustc::__rust_dealloc>
1001810bc:     	mov	x0, x20
1001810c0:     	bl	0x1001dce98 <dyld_stub_binder+0x1001dce98>

