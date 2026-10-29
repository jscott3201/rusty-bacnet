000000010017a8a8 <bacnet_encoding::npdu::decode_npdu>:
10017a8a8:     	sub	sp, sp, #0x130
10017a8ac:     	stp	x28, x27, [sp, #0xd0]
10017a8b0:     	stp	x26, x25, [sp, #0xe0]
10017a8b4:     	stp	x24, x23, [sp, #0xf0]
10017a8b8:     	stp	x22, x21, [sp, #0x100]
10017a8bc:     	stp	x20, x19, [sp, #0x110]
10017a8c0:     	stp	x29, x30, [sp, #0x120]
10017a8c4:     	add	x29, sp, #0x120
10017a8c8:     	mov	x19, x0
10017a8cc:     	mov	x22, x8
10017a8d0:     	ldp	x21, x20, [x0, #0x8]
10017a8d4:     	cmp	x20, #0x2
10017a8d8:     	b.lo	0x10017aa04 <bacnet_encoding::npdu::decode_npdu+0x15c>
10017a8dc:     	ldrb	w8, [x21, #0x1]
10017a8e0:     	strb	w8, [sp, #0x5f]
10017a8e4:     	mov	w9, #0x50               ; =80
10017a8e8:     	tst	w8, w9
10017a8ec:     	b.eq	0x10017a984 <bacnet_encoding::npdu::decode_npdu+0xdc>
10017a8f0:     	adrp	x8, 0x100278000 <dyld_stub_binder+0x100278000>
10017a8f4:     	add	x8, x8, #0x568
10017a8f8:     	ldr	x8, [x8]
10017a8fc:     	cmp	x8, #0x3
10017a900:     	b.hi	0x10017a984 <bacnet_encoding::npdu::decode_npdu+0xdc>
10017a904:     	adrp	x8, 0x100278000 <dyld_stub_binder+0x100278000>
10017a908:     	ldrb	w1, [x8, #0x560]
10017a90c:     	sub	w8, w1, #0x1
10017a910:     	cmp	w8, #0x2
10017a914:     	b.hs	0x10017a980 <bacnet_encoding::npdu::decode_npdu+0xd8>
10017a918:     	adrp	x23, 0x100278000 <dyld_stub_binder+0x100278000>
10017a91c:     	ldr	x0, [x23, #0x550]
10017a920:     	bl	0x10017fb78 <tracing::__macro_support::__is_enabled>
10017a924:     	cbz	w0, 0x10017a984 <bacnet_encoding::npdu::decode_npdu+0xdc>
10017a928:     	ldr	x0, [x23, #0x550]
10017a92c:     	add	x8, x0, #0x30
10017a930:     	adrp	x9, 0x10020f000 <_anon.53300889abeb74b6fbdfa7dcf1d6cfd9.86+0x88>
10017a934:     	add	x9, x9, #0x7b0
10017a938:     	mov	w10, #0x6b              ; =107
10017a93c:     	stp	x9, x10, [sp, #0x60]
10017a940:     	add	x9, sp, #0x60
10017a944:     	adrp	x10, 0x100273000 <_anon.53300889abeb74b6fbdfa7dcf1d6cfd9.42+0xe8>
10017a948:     	add	x10, x10, #0xab8
10017a94c:     	stp	x9, x10, [x29, #-0x80]
10017a950:     	add	x9, sp, #0x5f
10017a954:     	adrp	x10, 0x100273000 <_anon.53300889abeb74b6fbdfa7dcf1d6cfd9.42+0xe8>
10017a958:     	add	x10, x10, #0xad8
10017a95c:     	stp	x9, x10, [x29, #-0x70]
10017a960:     	mov	w9, #0x1                ; =1
10017a964:     	sub	x10, x29, #0x80
10017a968:     	stp	x9, x10, [sp, #0x70]
10017a96c:     	mov	w9, #0x2                ; =2
10017a970:     	stp	x9, x8, [sp, #0x80]
10017a974:     	add	x1, sp, #0x70
10017a978:     	bl	0x100180020 <<tracing_core::event::Event>::dispatch>
10017a97c:     	b	0x10017a984 <bacnet_encoding::npdu::decode_npdu+0xdc>
10017a980:     	cbnz	w1, 0x10017ada8 <bacnet_encoding::npdu::decode_npdu+0x500>
10017a984:     	ldrb	w8, [x21]
10017a988:     	cmp	w8, #0x1
10017a98c:     	b.ne	0x10017a9f0 <bacnet_encoding::npdu::decode_npdu+0x148>
10017a990:     	ldrb	w27, [x21, #0x1]
10017a994:     	tbnz	w27, #0x5, 0x10017a9fc <bacnet_encoding::npdu::decode_npdu+0x154>
10017a998:     	mov	x25, #0x0               ; =0
10017a99c:     	mov	w24, #0x2               ; =2
10017a9a0:     	tbnz	w27, #0x3, 0x10017abec <bacnet_encoding::npdu::decode_npdu+0x344>
10017a9a4:     	mov	x11, #0x0               ; =0
10017a9a8:     	tbnz	w27, #0x5, 0x10017acf8 <bacnet_encoding::npdu::decode_npdu+0x450>
10017a9ac:     	mov	w26, #0xff              ; =255
10017a9b0:     	tbnz	w27, #0x7, 0x10017ad0c <bacnet_encoding::npdu::decode_npdu+0x464>
10017a9b4:     	str	x25, [sp, #0x48]
10017a9b8:     	mov	w16, #0x0               ; =0
10017a9bc:     	mov	w17, #0x0               ; =0
10017a9c0:     	str	x24, [sp, #0x60]
10017a9c4:     	stur	x20, [x29, #-0x80]
10017a9c8:     	subs	x25, x20, x24
10017a9cc:     	b.lo	0x10017adc0 <bacnet_encoding::npdu::decode_npdu+0x518>
10017a9d0:     	b.ne	0x10017aa94 <bacnet_encoding::npdu::decode_npdu+0x1ec>
10017a9d4:     	mov	x25, #0x0               ; =0
10017a9d8:     	add	x8, x21, x20
10017a9dc:     	str	xzr, [sp, #0x88]
10017a9e0:     	adrp	x9, 0x100273000 <_anon.53300889abeb74b6fbdfa7dcf1d6cfd9.42+0xe8>
10017a9e4:     	add	x9, x9, #0xd80
10017a9e8:     	stp	x9, x8, [sp, #0x70]
10017a9ec:     	b	0x10017aae8 <bacnet_encoding::npdu::decode_npdu+0x240>
10017a9f0:     	mov	w8, #0x1                ; =1
10017a9f4:     	strb	w8, [x22, #0x10]
10017a9f8:     	b	0x10017aa08 <bacnet_encoding::npdu::decode_npdu+0x160>
10017a9fc:     	cmp	x20, #0x5
10017aa00:     	b.hs	0x10017aa4c <bacnet_encoding::npdu::decode_npdu+0x1a4>
10017aa04:     	strb	wzr, [x22, #0x10]
10017aa08:     	mov	w8, #0x2                ; =2
10017aa0c:     	dup.2d	v0, x8
10017aa10:     	str	q0, [x22]
10017aa14:     	ldr	x0, [x19, #0x18]
10017aa18:     	ldr	x8, [x19]
10017aa1c:     	ldr	x8, [x8, #0x20]
10017aa20:     	mov	x1, x21
10017aa24:     	mov	x2, x20
10017aa28:     	blr	x8
10017aa2c:     	ldp	x29, x30, [sp, #0x120]
10017aa30:     	ldp	x20, x19, [sp, #0x110]
10017aa34:     	ldp	x22, x21, [sp, #0x100]
10017aa38:     	ldp	x24, x23, [sp, #0xf0]
10017aa3c:     	ldp	x26, x25, [sp, #0xe0]
10017aa40:     	ldp	x28, x27, [sp, #0xd0]
10017aa44:     	add	sp, sp, #0x130
10017aa48:     	ret
10017aa4c:     	ldrh	w8, [x21, #0x2]
10017aa50:     	cbz	w8, 0x10017aa88 <bacnet_encoding::npdu::decode_npdu+0x1e0>
10017aa54:     	rev16	w9, w8
10017aa58:     	ldrb	w1, [x21, #0x4]
10017aa5c:     	cmp	x1, #0x12
10017aa60:     	b.ls	0x10017ab58 <bacnet_encoding::npdu::decode_npdu+0x2b0>
10017aa64:     	mov	w8, #0x1                ; =1
10017aa68:     	strh	w8, [sp, #0x98]
10017aa6c:     	strh	w9, [sp, #0x9a]
10017aa70:     	strb	wzr, [sp, #0x9c]
10017aa74:     	strb	w1, [sp, #0x9d]
10017aa78:     	ldr	x23, [sp, #0x98]
10017aa7c:     	tbnz	w27, #0x3, 0x10017ab74 <bacnet_encoding::npdu::decode_npdu+0x2cc>
10017aa80:     	mov	x8, #0x0                ; =0
10017aa84:     	b	0x10017ab70 <bacnet_encoding::npdu::decode_npdu+0x2c8>
10017aa88:     	mov	w8, #0x2                ; =2
10017aa8c:     	strb	w8, [x22, #0x10]
10017aa90:     	b	0x10017aa0c <bacnet_encoding::npdu::decode_npdu+0x164>
10017aa94:     	stp	w0, w17, [sp, #0x8]
10017aa98:     	stp	w16, w15, [sp, #0x10]
10017aa9c:     	mov	x28, x13
10017aaa0:     	mov	x23, x12
10017aaa4:     	str	x11, [sp, #0x18]
10017aaa8:     	mov	x0, x19
10017aaac:     	ldr	x8, [x0], #0x18
10017aab0:     	ldr	x9, [x8]
10017aab4:     	add	x8, sp, #0x70
10017aab8:     	mov	x1, x21
10017aabc:     	mov	x2, x20
10017aac0:     	blr	x9
10017aac4:     	ldr	x8, [sp, #0x78]
10017aac8:     	add	x8, x8, x24
10017aacc:     	str	x8, [sp, #0x78]
10017aad0:     	ldp	x21, x20, [x19, #0x8]
10017aad4:     	ldr	x11, [sp, #0x18]
10017aad8:     	mov	x12, x23
10017aadc:     	mov	x13, x28
10017aae0:     	ldp	w16, w15, [sp, #0x10]
10017aae4:     	ldp	w0, w17, [sp, #0x8]
10017aae8:     	and	w8, w27, #0x3
10017aaec:     	str	x25, [sp, #0x80]
10017aaf0:     	ubfx	w9, w27, #2, #1
10017aaf4:     	ldp	q0, q1, [sp, #0x70]
10017aaf8:     	stp	q0, q1, [x22, #0x50]
10017aafc:     	ldr	x10, [sp, #0x48]
10017ab00:     	str	x10, [x22]
10017ab04:     	ldr	x10, [sp, #0x40]
10017ab08:     	str	x10, [x22, #0x8]
10017ab0c:     	ldr	x10, [sp, #0x20]
10017ab10:     	str	x10, [x22, #0x10]
10017ab14:     	ldr	x10, [sp, #0x50]
10017ab18:     	str	x10, [x22, #0x18]
10017ab1c:     	ldr	w10, [sp, #0x34]
10017ab20:     	strh	w10, [x22, #0x20]
10017ab24:     	ldr	x10, [sp, #0x28]
10017ab28:     	stp	x11, x10, [x22, #0x28]
10017ab2c:     	ldr	x10, [sp, #0x38]
10017ab30:     	stp	x12, x10, [x22, #0x38]
10017ab34:     	strh	w13, [x22, #0x48]
10017ab38:     	strh	w17, [x22, #0x70]
10017ab3c:     	strh	w0, [x22, #0x72]
10017ab40:     	strb	w16, [x22, #0x74]
10017ab44:     	strb	w15, [x22, #0x75]
10017ab48:     	strb	w9, [x22, #0x76]
10017ab4c:     	strb	w8, [x22, #0x77]
10017ab50:     	strb	w26, [x22, #0x78]
10017ab54:     	b	0x10017aa14 <bacnet_encoding::npdu::decode_npdu+0x16c>
10017ab58:     	add	x8, x1, #0x5
10017ab5c:     	cmp	x8, x20
10017ab60:     	b.ls	0x10017abac <bacnet_encoding::npdu::decode_npdu+0x304>
10017ab64:     	strb	wzr, [sp, #0x78]
10017ab68:     	mov	w8, #0x2                ; =2
10017ab6c:     	ldr	x9, [sp, #0x78]
10017ab70:     	b	0x10017ab94 <bacnet_encoding::npdu::decode_npdu+0x2ec>
10017ab74:     	sub	x0, x29, #0x80
10017ab78:     	add	x3, x1, #0x5
10017ab7c:     	mov	x1, x21
10017ab80:     	mov	x2, x20
10017ab84:     	bl	0x10017b238 <bacnet_encoding::npdu::source_behind>
10017ab88:     	ldp	x8, x9, [x29, #-0x80]
10017ab8c:     	ldur	q0, [x29, #-0x70]
10017ab90:     	ldur	x10, [x29, #-0x60]
10017ab94:     	stp	x8, x9, [x22, #0x8]
10017ab98:     	stur	q0, [x22, #0x18]
10017ab9c:     	stp	x10, x23, [x22, #0x28]
10017aba0:     	mov	w9, #0x2                ; =2
10017aba4:     	str	x9, [x22]
10017aba8:     	b	0x10017aa14 <bacnet_encoding::npdu::decode_npdu+0x16c>
10017abac:     	str	w9, [sp, #0x34]
10017abb0:     	add	x8, sp, #0x70
10017abb4:     	mov	x25, #0x0               ; =0
10017abb8:     	add	x8, x8, #0x8
10017abbc:     	add	x0, x21, #0x5
10017abc0:     	bl	0x10017c9d0 <<smallvec::SmallVec<[u8; 6]>>::from_slice>
10017abc4:     	ldp	x8, x9, [sp, #0x78]
10017abc8:     	str	x8, [sp, #0x40]
10017abcc:     	ldr	x8, [sp, #0x88]
10017abd0:     	cmp	x8, #0x6
10017abd4:     	str	x8, [sp, #0x50]
10017abd8:     	str	x9, [sp, #0x20]
10017abdc:     	csel	x8, x9, x8, hi
10017abe0:     	add	x24, x8, #0x5
10017abe4:     	mov	w25, #0x1               ; =1
10017abe8:     	tbz	w27, #0x3, 0x10017a9a4 <bacnet_encoding::npdu::decode_npdu+0xfc>
10017abec:     	add	x28, x24, #0x3
10017abf0:     	cmp	x28, x20
10017abf4:     	b.ls	0x10017ac04 <bacnet_encoding::npdu::decode_npdu+0x35c>
10017abf8:     	strb	wzr, [x22, #0x10]
10017abfc:     	mov	w8, #0x2                ; =2
10017ac00:     	b	0x10017ac48 <bacnet_encoding::npdu::decode_npdu+0x3a0>
10017ac04:     	cmp	x24, x20
10017ac08:     	b.hs	0x10017adf8 <bacnet_encoding::npdu::decode_npdu+0x550>
10017ac0c:     	add	x8, x24, #0x1
10017ac10:     	cmp	x8, x20
10017ac14:     	b.hs	0x10017ae0c <bacnet_encoding::npdu::decode_npdu+0x564>
10017ac18:     	ldrb	w9, [x21, x24]
10017ac1c:     	ldrb	w8, [x21, x8]
10017ac20:     	orr	w23, w9, w8, lsl #8
10017ac24:     	rev	w8, w23
10017ac28:     	mov	w9, #0xffff             ; =65535
10017ac2c:     	cmp	w9, w8, lsr #16
10017ac30:     	b.eq	0x10017ac40 <bacnet_encoding::npdu::decode_npdu+0x398>
10017ac34:     	and	w8, w23, #0xffff
10017ac38:     	rev16	w8, w8
10017ac3c:     	cbnz	w8, 0x10017ac70 <bacnet_encoding::npdu::decode_npdu+0x3c8>
10017ac40:     	mov	w8, #0x2                ; =2
10017ac44:     	strb	w8, [x22, #0x10]
10017ac48:     	dup.2d	v0, x8
10017ac4c:     	str	q0, [x22]
10017ac50:     	ldr	x1, [sp, #0x50]
10017ac54:     	tbz	w27, #0x5, 0x10017aa14 <bacnet_encoding::npdu::decode_npdu+0x16c>
10017ac58:     	cmp	x1, #0x7
10017ac5c:     	b.lo	0x10017aa14 <bacnet_encoding::npdu::decode_npdu+0x16c>
10017ac60:     	ldr	x0, [sp, #0x40]
10017ac64:     	mov	w2, #0x1                ; =1
10017ac68:     	bl	0x10003091c <__rustc::__rust_dealloc>
10017ac6c:     	b	0x10017aa14 <bacnet_encoding::npdu::decode_npdu+0x16c>
10017ac70:     	add	x24, x24, #0x2
10017ac74:     	cmp	x24, x20
10017ac78:     	b.hs	0x10017ae24 <bacnet_encoding::npdu::decode_npdu+0x57c>
10017ac7c:     	ldrb	w4, [x21, x24]
10017ac80:     	cbz	w4, 0x10017accc <bacnet_encoding::npdu::decode_npdu+0x424>
10017ac84:     	add	x0, sp, #0x70
10017ac88:     	mov	x1, x21
10017ac8c:     	mov	x2, x20
10017ac90:     	mov	x3, x28
10017ac94:     	mov	x5, x25
10017ac98:     	ldr	w6, [sp, #0x34]
10017ac9c:     	bl	0x10017a640 <bacnet_encoding::npdu::decode_address::<core::option::Option<u16>>>
10017aca0:     	ldp	x8, x10, [sp, #0x70]
10017aca4:     	ldp	x12, x9, [sp, #0x80]
10017aca8:     	cmn	x8, #0x1
10017acac:     	b.eq	0x10017acd8 <bacnet_encoding::npdu::decode_npdu+0x430>
10017acb0:     	ldr	q0, [sp, #0x90]
10017acb4:     	stur	q0, [x22, #0x28]
10017acb8:     	stp	x10, x12, [x22, #0x10]
10017acbc:     	str	x9, [x22, #0x20]
10017acc0:     	mov	w9, #0x2                ; =2
10017acc4:     	stp	x9, x8, [x22]
10017acc8:     	b	0x10017ac50 <bacnet_encoding::npdu::decode_npdu+0x3a8>
10017accc:     	mov	w8, #0x3                ; =3
10017acd0:     	strb	w8, [x22, #0x10]
10017acd4:     	b	0x10017abfc <bacnet_encoding::npdu::decode_npdu+0x354>
10017acd8:     	str	x10, [sp, #0x28]
10017acdc:     	rev16	w13, w23
10017ace0:     	cmp	x9, #0x6
10017ace4:     	str	x9, [sp, #0x38]
10017ace8:     	csel	x8, x12, x9, hi
10017acec:     	add	x24, x8, x28
10017acf0:     	mov	w11, #0x1               ; =1
10017acf4:     	tbz	w27, #0x5, 0x10017a9ac <bacnet_encoding::npdu::decode_npdu+0x104>
10017acf8:     	cmp	x24, x20
10017acfc:     	b.hs	0x10017ad40 <bacnet_encoding::npdu::decode_npdu+0x498>
10017ad00:     	ldrb	w26, [x21, x24]
10017ad04:     	add	x24, x24, #0x1
10017ad08:     	tbz	w27, #0x7, 0x10017a9b4 <bacnet_encoding::npdu::decode_npdu+0x10c>
10017ad0c:     	cmp	x24, x20
10017ad10:     	b.hs	0x10017ad40 <bacnet_encoding::npdu::decode_npdu+0x498>
10017ad14:     	ldrsb	w15, [x21, x24]
10017ad18:     	add	x8, x24, #0x1
10017ad1c:     	tbnz	w15, #0x1f, 0x10017ad34 <bacnet_encoding::npdu::decode_npdu+0x48c>
10017ad20:     	str	x25, [sp, #0x48]
10017ad24:     	mov	w17, #0x0               ; =0
10017ad28:     	mov	w16, #0x1               ; =1
10017ad2c:     	mov	x24, x8
10017ad30:     	b	0x10017a9c0 <bacnet_encoding::npdu::decode_npdu+0x118>
10017ad34:     	add	x9, x24, #0x3
10017ad38:     	cmp	x9, x20
10017ad3c:     	b.ls	0x10017ad70 <bacnet_encoding::npdu::decode_npdu+0x4c8>
10017ad40:     	strb	wzr, [x22, #0x10]
10017ad44:     	mov	w8, #0x2                ; =2
10017ad48:     	dup.2d	v0, x8
10017ad4c:     	str	q0, [x22]
10017ad50:     	ldr	x1, [sp, #0x38]
10017ad54:     	tbz	w27, #0x3, 0x10017ac50 <bacnet_encoding::npdu::decode_npdu+0x3a8>
10017ad58:     	cmp	x1, #0x7
10017ad5c:     	b.lo	0x10017ac50 <bacnet_encoding::npdu::decode_npdu+0x3a8>
10017ad60:     	ldr	x0, [sp, #0x28]
10017ad64:     	mov	w2, #0x1                ; =1
10017ad68:     	bl	0x10003091c <__rustc::__rust_dealloc>
10017ad6c:     	b	0x10017ac50 <bacnet_encoding::npdu::decode_npdu+0x3a8>
10017ad70:     	cmp	x8, x20
10017ad74:     	str	x25, [sp, #0x48]
10017ad78:     	b.hs	0x10017ae44 <bacnet_encoding::npdu::decode_npdu+0x59c>
10017ad7c:     	add	x24, x24, #0x2
10017ad80:     	cmp	x24, x20
10017ad84:     	b.hs	0x10017ae58 <bacnet_encoding::npdu::decode_npdu+0x5b0>
10017ad88:     	ldrb	w8, [x21, x8]
10017ad8c:     	ldrb	w10, [x21, x24]
10017ad90:     	orr	w8, w8, w10, lsl #8
10017ad94:     	rev16	w0, w8
10017ad98:     	mov	w16, #0x1               ; =1
10017ad9c:     	mov	w17, #0x1               ; =1
10017ada0:     	mov	x24, x9
10017ada4:     	b	0x10017a9c0 <bacnet_encoding::npdu::decode_npdu+0x118>
10017ada8:     	adrp	x0, 0x100278000 <dyld_stub_binder+0x100278000>
10017adac:     	add	x0, x0, #0x550
10017adb0:     	bl	0x1001d7380 <<tracing_core::callsite::DefaultCallsite>::register>
10017adb4:     	and	w1, w0, #0xff
10017adb8:     	cbnz	w1, 0x10017a918 <bacnet_encoding::npdu::decode_npdu+0x70>
10017adbc:     	b	0x10017a984 <bacnet_encoding::npdu::decode_npdu+0xdc>
10017adc0:     	str	x11, [sp, #0x18]
10017adc4:     	adrp	x8, 0x10017e000 <<&str as core::fmt::Display>::fmt+0x10>
10017adc8:     	add	x8, x8, #0x234
10017adcc:     	add	x9, sp, #0x60
10017add0:     	stp	x9, x8, [sp, #0x70]
10017add4:     	sub	x9, x29, #0x80
10017add8:     	stp	x9, x8, [sp, #0x80]
10017addc:     	adrp	x0, 0x1001f7000 <_anon.d7fb901a147db7d71026f30bc22a0fa2.3+0x71>
10017ade0:     	add	x0, x0, #0x7ac
10017ade4:     	adrp	x2, 0x100273000 <_anon.53300889abeb74b6fbdfa7dcf1d6cfd9.42+0xe8>
10017ade8:     	add	x2, x2, #0xd68
10017adec:     	add	x1, sp, #0x70
10017adf0:     	bl	0x1001dcbd0 <core::panicking::panic_fmt>
10017adf4:     	b	0x10017ae40 <bacnet_encoding::npdu::decode_npdu+0x598>
10017adf8:     	str	x25, [sp, #0x48]
10017adfc:     	str	xzr, [sp, #0x18]
10017ae00:     	adrp	x2, 0x100273000 <_anon.53300889abeb74b6fbdfa7dcf1d6cfd9.42+0xe8>
10017ae04:     	add	x2, x2, #0xaf8
10017ae08:     	b	0x10017ae34 <bacnet_encoding::npdu::decode_npdu+0x58c>
10017ae0c:     	str	x25, [sp, #0x48]
10017ae10:     	str	xzr, [sp, #0x18]
10017ae14:     	adrp	x2, 0x100273000 <_anon.53300889abeb74b6fbdfa7dcf1d6cfd9.42+0xe8>
10017ae18:     	add	x2, x2, #0xb10
10017ae1c:     	mov	x24, x8
10017ae20:     	b	0x10017ae34 <bacnet_encoding::npdu::decode_npdu+0x58c>
10017ae24:     	str	x25, [sp, #0x48]
10017ae28:     	str	xzr, [sp, #0x18]
10017ae2c:     	adrp	x2, 0x100273000 <_anon.53300889abeb74b6fbdfa7dcf1d6cfd9.42+0xe8>
10017ae30:     	add	x2, x2, #0xb28
10017ae34:     	mov	x0, x24
10017ae38:     	mov	x1, x20
10017ae3c:     	bl	0x1001dca38 <core::panicking::panic_bounds_check>
10017ae40:     	brk	#0x1
10017ae44:     	str	x11, [sp, #0x18]
10017ae48:     	adrp	x2, 0x100273000 <_anon.53300889abeb74b6fbdfa7dcf1d6cfd9.42+0xe8>
10017ae4c:     	add	x2, x2, #0xb40
10017ae50:     	mov	x24, x8
10017ae54:     	b	0x10017ae34 <bacnet_encoding::npdu::decode_npdu+0x58c>
10017ae58:     	str	x11, [sp, #0x18]
10017ae5c:     	adrp	x2, 0x100273000 <_anon.53300889abeb74b6fbdfa7dcf1d6cfd9.42+0xe8>
10017ae60:     	add	x2, x2, #0xb58
10017ae64:     	b	0x10017ae34 <bacnet_encoding::npdu::decode_npdu+0x58c>
10017ae68:     	b	0x10017ae78 <bacnet_encoding::npdu::decode_npdu+0x5d0>
10017ae6c:     	str	x25, [sp, #0x48]
10017ae70:     	mov	x20, x0
10017ae74:     	b	0x10017aea8 <bacnet_encoding::npdu::decode_npdu+0x600>
10017ae78:     	mov	x20, x0
10017ae7c:     	b	0x10017aecc <bacnet_encoding::npdu::decode_npdu+0x624>
10017ae80:     	mov	x20, x0
10017ae84:     	ldr	x8, [sp, #0x18]
10017ae88:     	cbz	x8, 0x10017aea8 <bacnet_encoding::npdu::decode_npdu+0x600>
10017ae8c:     	ldr	x8, [sp, #0x38]
10017ae90:     	cmp	x8, #0x7
10017ae94:     	b.lo	0x10017aea8 <bacnet_encoding::npdu::decode_npdu+0x600>
10017ae98:     	ldr	x0, [sp, #0x28]
10017ae9c:     	ldr	x1, [sp, #0x38]
10017aea0:     	mov	w2, #0x1                ; =1
10017aea4:     	bl	0x10003091c <__rustc::__rust_dealloc>
10017aea8:     	ldr	x8, [sp, #0x48]
10017aeac:     	cbz	x8, 0x10017aecc <bacnet_encoding::npdu::decode_npdu+0x624>
10017aeb0:     	ldr	x8, [sp, #0x50]
10017aeb4:     	cmp	x8, #0x7
10017aeb8:     	b.lo	0x10017aecc <bacnet_encoding::npdu::decode_npdu+0x624>
10017aebc:     	ldr	x0, [sp, #0x40]
10017aec0:     	ldr	x1, [sp, #0x50]
10017aec4:     	mov	w2, #0x1                ; =1
10017aec8:     	bl	0x10003091c <__rustc::__rust_dealloc>
10017aecc:     	ldp	x8, x1, [x19]
10017aed0:     	ldr	x8, [x8, #0x20]
10017aed4:     	ldp	x2, x0, [x19, #0x10]
10017aed8:     	blr	x8
10017aedc:     	mov	x0, x20
10017aee0:     	bl	0x1001dce98 <dyld_stub_binder+0x1001dce98>
10017aee4:     	bl	0x1001dca20 <core::panicking::panic_in_cleanup>

