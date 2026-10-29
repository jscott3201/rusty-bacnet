000000010018291c <<bacnet_npdu::view::Npdu>::decode>:
10018291c:     	sub	sp, sp, #0x50
100182920:     	stp	x22, x21, [sp, #0x20]
100182924:     	stp	x20, x19, [sp, #0x30]
100182928:     	stp	x29, x30, [sp, #0x40]
10018292c:     	add	x29, sp, #0x40
100182930:     	mov	x2, x1
100182934:     	cmp	x1, #0x1
100182938:     	b.ls	0x100182b28 <<bacnet_npdu::view::Npdu>::decode+0x20c>
10018293c:     	mov	x1, x0
100182940:     	ldrb	w9, [x0]
100182944:     	cmp	w9, #0x1
100182948:     	b.ne	0x10018297c <<bacnet_npdu::view::Npdu>::decode+0x60>
10018294c:     	ldrb	w9, [x1, #0x1]
100182950:     	tbnz	w9, #0x5, 0x100182988 <<bacnet_npdu::view::Npdu>::decode+0x6c>
100182954:     	mov	x10, #0x0               ; =0
100182958:     	mov	w0, #0x2                ; =2
10018295c:     	tbnz	w9, #0x3, 0x1001829c8 <<bacnet_npdu::view::Npdu>::decode+0xac>
100182960:     	mov	x13, #0x0               ; =0
100182964:     	cbz	x10, 0x100182a9c <<bacnet_npdu::view::Npdu>::decode+0x180>
100182968:     	cmp	x0, x2
10018296c:     	b.hs	0x100182b28 <<bacnet_npdu::view::Npdu>::decode+0x20c>
100182970:     	ldrb	w14, [x1, x0]
100182974:     	add	x0, x0, #0x1
100182978:     	b	0x100182aa0 <<bacnet_npdu::view::Npdu>::decode+0x184>
10018297c:     	mov	w9, #0x1                ; =1
100182980:     	strb	w9, [x8]
100182984:     	b	0x100182b2c <<bacnet_npdu::view::Npdu>::decode+0x210>
100182988:     	cmp	x2, #0x5
10018298c:     	b.lo	0x100182b28 <<bacnet_npdu::view::Npdu>::decode+0x20c>
100182990:     	ldrh	w10, [x1, #0x2]
100182994:     	cbz	w10, 0x100182a2c <<bacnet_npdu::view::Npdu>::decode+0x110>
100182998:     	rev16	w19, w10
10018299c:     	ldrb	w20, [x1, #0x4]
1001829a0:     	cmp	w20, #0x12
1001829a4:     	b.ls	0x1001829b4 <<bacnet_npdu::view::Npdu>::decode+0x98>
1001829a8:     	tbnz	w9, #0x3, 0x100182a38 <<bacnet_npdu::view::Npdu>::decode+0x11c>
1001829ac:     	str	xzr, [sp, #0x8]
1001829b0:     	b	0x100182a4c <<bacnet_npdu::view::Npdu>::decode+0x130>
1001829b4:     	add	x0, x20, #0x5
1001829b8:     	cmp	x0, x2
1001829bc:     	b.hi	0x100182b28 <<bacnet_npdu::view::Npdu>::decode+0x20c>
1001829c0:     	add	x10, x1, #0x5
1001829c4:     	tbz	w9, #0x3, 0x100182960 <<bacnet_npdu::view::Npdu>::decode+0x44>
1001829c8:     	add	x13, x0, #0x3
1001829cc:     	cmp	x13, x2
1001829d0:     	b.hi	0x100182b28 <<bacnet_npdu::view::Npdu>::decode+0x20c>
1001829d4:     	add	x11, x1, x0
1001829d8:     	ldrh	w12, [x11]
1001829dc:     	rev	w14, w12
1001829e0:     	lsr	w14, w14, #16
1001829e4:     	sub	w14, w14, #0x1
1001829e8:     	mov	w15, #0xfffe            ; =65534
1001829ec:     	cmp	w15, w14, uxth
1001829f0:     	b.ls	0x100182a2c <<bacnet_npdu::view::Npdu>::decode+0x110>
1001829f4:     	ldrb	w11, [x11, #0x2]
1001829f8:     	cbz	w11, 0x100182a78 <<bacnet_npdu::view::Npdu>::decode+0x15c>
1001829fc:     	cmp	w11, #0x12
100182a00:     	b.ls	0x100182a84 <<bacnet_npdu::view::Npdu>::decode+0x168>
100182a04:     	cmp	x10, #0x0
100182a08:     	str	xzr, [x8]
100182a0c:     	cset	w9, ne
100182a10:     	strh	w9, [x8, #0x18]
100182a14:     	strh	w19, [x8, #0x1a]
100182a18:     	mov	w9, #0x1                ; =1
100182a1c:     	strb	w9, [x8, #0x1c]
100182a20:     	strb	w11, [x8, #0x1d]
100182a24:     	mov	w9, #0x2                ; =2
100182a28:     	b	0x100182b34 <<bacnet_npdu::view::Npdu>::decode+0x218>
100182a2c:     	mov	w9, #0x2                ; =2
100182a30:     	strb	w9, [x8]
100182a34:     	b	0x100182b30 <<bacnet_npdu::view::Npdu>::decode+0x214>
100182a38:     	add	x0, sp, #0x8
100182a3c:     	add	x3, x20, #0x5
100182a40:     	mov	x21, x8
100182a44:     	bl	0x100182b70 <bacnet_npdu::view::source_behind>
100182a48:     	mov	x8, x21
100182a4c:     	ldur	q0, [sp, #0x8]
100182a50:     	str	q0, [x8]
100182a54:     	ldr	x9, [sp, #0x18]
100182a58:     	str	x9, [x8, #0x10]
100182a5c:     	mov	w9, #0x1                ; =1
100182a60:     	strh	w9, [x8, #0x18]
100182a64:     	strh	w19, [x8, #0x1a]
100182a68:     	strb	wzr, [x8, #0x1c]
100182a6c:     	strb	w20, [x8, #0x1d]
100182a70:     	mov	w9, #0x2                ; =2
100182a74:     	b	0x100182b34 <<bacnet_npdu::view::Npdu>::decode+0x218>
100182a78:     	mov	w9, #0x3                ; =3
100182a7c:     	strb	w9, [x8]
100182a80:     	b	0x100182b2c <<bacnet_npdu::view::Npdu>::decode+0x210>
100182a84:     	add	x0, x13, x11
100182a88:     	cmp	x0, x2
100182a8c:     	b.hi	0x100182b28 <<bacnet_npdu::view::Npdu>::decode+0x20c>
100182a90:     	rev16	w12, w12
100182a94:     	add	x13, x1, x13
100182a98:     	cbnz	x10, 0x100182968 <<bacnet_npdu::view::Npdu>::decode+0x4c>
100182a9c:     	mov	w14, #0xff              ; =255
100182aa0:     	tbnz	w9, #0x7, 0x100182af8 <<bacnet_npdu::view::Npdu>::decode+0x1dc>
100182aa4:     	mov	w16, #0x0               ; =0
100182aa8:     	mov	w3, #0x0                ; =0
100182aac:     	subs	x4, x2, x0
100182ab0:     	b.lo	0x100182b60 <<bacnet_npdu::view::Npdu>::decode+0x244>
100182ab4:     	and	w2, w9, #0x3
100182ab8:     	add	x1, x1, x0
100182abc:     	ubfx	w9, w9, #2, #1
100182ac0:     	stp	x1, x4, [x8]
100182ac4:     	stp	x10, x20, [x8, #0x10]
100182ac8:     	strh	w19, [x8, #0x20]
100182acc:     	stp	x13, x11, [x8, #0x28]
100182ad0:     	strh	w12, [x8, #0x38]
100182ad4:     	str	x0, [x8, #0x40]
100182ad8:     	strb	w3, [x8, #0x48]
100182adc:     	strb	w15, [x8, #0x49]
100182ae0:     	strh	w16, [x8, #0x4a]
100182ae4:     	strh	w17, [x8, #0x4c]
100182ae8:     	strb	w14, [x8, #0x4e]
100182aec:     	strb	w2, [x8, #0x4f]
100182af0:     	strb	w9, [x8, #0x50]
100182af4:     	b	0x100182b38 <<bacnet_npdu::view::Npdu>::decode+0x21c>
100182af8:     	cmp	x0, x2
100182afc:     	b.hs	0x100182b28 <<bacnet_npdu::view::Npdu>::decode+0x20c>
100182b00:     	ldrsb	w15, [x1, x0]
100182b04:     	add	x17, x0, #0x1
100182b08:     	tbnz	w15, #0x1f, 0x100182b1c <<bacnet_npdu::view::Npdu>::decode+0x200>
100182b0c:     	mov	w16, #0x0               ; =0
100182b10:     	mov	w3, #0x1                ; =1
100182b14:     	mov	x0, x17
100182b18:     	b	0x100182aac <<bacnet_npdu::view::Npdu>::decode+0x190>
100182b1c:     	add	x0, x0, #0x3
100182b20:     	cmp	x0, x2
100182b24:     	b.ls	0x100182b4c <<bacnet_npdu::view::Npdu>::decode+0x230>
100182b28:     	strb	wzr, [x8]
100182b2c:     	mov	w9, #0x2                ; =2
100182b30:     	strh	w9, [x8, #0x18]
100182b34:     	strh	w9, [x8, #0x4a]
100182b38:     	ldp	x29, x30, [sp, #0x40]
100182b3c:     	ldp	x20, x19, [sp, #0x30]
100182b40:     	ldp	x22, x21, [sp, #0x20]
100182b44:     	add	sp, sp, #0x50
100182b48:     	ret
100182b4c:     	ldrh	w16, [x1, x17]
100182b50:     	rev16	w17, w16
100182b54:     	mov	w16, #0x1               ; =1
100182b58:     	mov	w3, #0x1                ; =1
100182b5c:     	b	0x100182aac <<bacnet_npdu::view::Npdu>::decode+0x190>
100182b60:     	adrp	x3, 0x100274000 <_anon.7022b18dc9d78539c81a31f9b756beb4.7+0x20>
100182b64:     	add	x3, x3, #0x138
100182b68:     	mov	x1, x2
100182b6c:     	bl	0x1001dc838 <core::slice::index::slice_index_fail>

