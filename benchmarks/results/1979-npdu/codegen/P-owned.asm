000000010017ac6c <bacnet_encoding::npdu::decode_npdu>:
10017ac6c:     	sub	sp, sp, #0x150
10017ac70:     	stp	x28, x27, [sp, #0xf0]
10017ac74:     	stp	x26, x25, [sp, #0x100]
10017ac78:     	stp	x24, x23, [sp, #0x110]
10017ac7c:     	stp	x22, x21, [sp, #0x120]
10017ac80:     	stp	x20, x19, [sp, #0x130]
10017ac84:     	stp	x29, x30, [sp, #0x140]
10017ac88:     	add	x29, sp, #0x140
10017ac8c:     	mov	x19, x0
10017ac90:     	mov	x22, x8
10017ac94:     	ldr	x20, [x0, #0x10]
10017ac98:     	cmp	x20, #0x2
10017ac9c:     	b.hs	0x10017acc0 <bacnet_encoding::npdu::decode_npdu+0x54>
10017aca0:     	str	x20, [x22, #0x10]
10017aca4:     	mov	w8, #0x15               ; =21
10017aca8:     	strb	w8, [x22, #0x40]
10017acac:     	mov	w8, #0x2                ; =2
10017acb0:     	dup.2d	v0, x8
10017acb4:     	str	q0, [x22]
10017acb8:     	ldr	x21, [x19, #0x8]
10017acbc:     	b	0x10017b298 <bacnet_encoding::npdu::decode_npdu+0x62c>
10017acc0:     	ldr	x21, [x19, #0x8]
10017acc4:     	ldrb	w8, [x21]
10017acc8:     	cmp	x8, #0x1
10017accc:     	b.ne	0x10017ad74 <bacnet_encoding::npdu::decode_npdu+0x108>
10017acd0:     	ldrb	w26, [x21, #0x1]
10017acd4:     	strb	w26, [sp, #0x67]
10017acd8:     	mov	w8, #0x50               ; =80
10017acdc:     	tst	w26, w8
10017ace0:     	b.eq	0x10017ada8 <bacnet_encoding::npdu::decode_npdu+0x13c>
10017ace4:     	adrp	x8, 0x100278000 <dyld_stub_binder+0x100278000>
10017ace8:     	add	x8, x8, #0x568
10017acec:     	ldr	x8, [x8]
10017acf0:     	cmp	x8, #0x3
10017acf4:     	b.hi	0x10017ada8 <bacnet_encoding::npdu::decode_npdu+0x13c>
10017acf8:     	adrp	x8, 0x100278000 <dyld_stub_binder+0x100278000>
10017acfc:     	ldrb	w1, [x8, #0x560]
10017ad00:     	sub	w8, w1, #0x1
10017ad04:     	cmp	w8, #0x2
10017ad08:     	b.hs	0x10017ada4 <bacnet_encoding::npdu::decode_npdu+0x138>
10017ad0c:     	adrp	x23, 0x100278000 <dyld_stub_binder+0x100278000>
10017ad10:     	ldr	x0, [x23, #0x550]
10017ad14:     	bl	0x10017fe50 <tracing::__macro_support::__is_enabled>
10017ad18:     	cbz	w0, 0x10017ada8 <bacnet_encoding::npdu::decode_npdu+0x13c>
10017ad1c:     	ldr	x0, [x23, #0x550]
10017ad20:     	add	x8, x0, #0x30
10017ad24:     	adrp	x9, 0x10020f000 <_anon.53300889abeb74b6fbdfa7dcf1d6cfd9.86+0x508>
10017ad28:     	add	x9, x9, #0x401
10017ad2c:     	mov	w10, #0x6b              ; =107
10017ad30:     	stp	x9, x10, [sp, #0x70]
10017ad34:     	add	x9, sp, #0x70
10017ad38:     	adrp	x10, 0x100273000 <_anon.53300889abeb74b6fbdfa7dcf1d6cfd9.42+0x30>
10017ad3c:     	add	x10, x10, #0xbe8
10017ad40:     	stp	x9, x10, [x29, #-0x80]
10017ad44:     	add	x9, sp, #0x67
10017ad48:     	adrp	x10, 0x100273000 <_anon.53300889abeb74b6fbdfa7dcf1d6cfd9.42+0x30>
10017ad4c:     	add	x10, x10, #0xc08
10017ad50:     	stp	x9, x10, [x29, #-0x70]
10017ad54:     	mov	w9, #0x1                ; =1
10017ad58:     	sub	x10, x29, #0x80
10017ad5c:     	stp	x9, x10, [sp, #0x80]
10017ad60:     	mov	w9, #0x2                ; =2
10017ad64:     	stp	x9, x8, [sp, #0x90]
10017ad68:     	add	x1, sp, #0x80
10017ad6c:     	bl	0x100180320 <<tracing_core::event::Event>::dispatch>
10017ad70:     	b	0x10017ada8 <bacnet_encoding::npdu::decode_npdu+0x13c>
10017ad74:     	adrp	x9, 0x10020f000 <_anon.53300889abeb74b6fbdfa7dcf1d6cfd9.86+0x508>
10017ad78:     	add	x9, x9, #0x56d
10017ad7c:     	mov	w10, #0x1               ; =1
10017ad80:     	stp	xzr, x9, [x22, #0x10]
10017ad84:     	adrp	x9, 0x10020f000 <_anon.53300889abeb74b6fbdfa7dcf1d6cfd9.86+0x508>
10017ad88:     	ldr	q0, [x9, #0x370]
10017ad8c:     	str	q0, [x22, #0x20]
10017ad90:     	stp	x8, xzr, [x22, #0x30]
10017ad94:     	strb	wzr, [x22, #0x40]
10017ad98:     	mov	w8, #0x2                ; =2
10017ad9c:     	stp	x8, x10, [x22]
10017ada0:     	b	0x10017b298 <bacnet_encoding::npdu::decode_npdu+0x62c>
10017ada4:     	cbnz	w1, 0x10017b388 <bacnet_encoding::npdu::decode_npdu+0x71c>
10017ada8:     	tbnz	w26, #0x5, 0x10017ae04 <bacnet_encoding::npdu::decode_npdu+0x198>
10017adac:     	mov	x24, #0x0               ; =0
10017adb0:     	mov	w28, #0x2               ; =2
10017adb4:     	tbnz	w26, #0x3, 0x10017b00c <bacnet_encoding::npdu::decode_npdu+0x3a0>
10017adb8:     	mov	x12, #0x0               ; =0
10017adbc:     	tbnz	w26, #0x5, 0x10017b130 <bacnet_encoding::npdu::decode_npdu+0x4c4>
10017adc0:     	mov	w15, #0xff              ; =255
10017adc4:     	tbnz	w26, #0x7, 0x10017b144 <bacnet_encoding::npdu::decode_npdu+0x4d8>
10017adc8:     	str	x24, [sp, #0x50]
10017adcc:     	mov	w16, #0x0               ; =0
10017add0:     	mov	w17, #0x0               ; =0
10017add4:     	str	x28, [sp, #0x70]
10017add8:     	stur	x20, [x29, #-0x80]
10017addc:     	subs	x25, x20, x28
10017ade0:     	b.lo	0x10017b17c <bacnet_encoding::npdu::decode_npdu+0x510>
10017ade4:     	b.ne	0x10017ae88 <bacnet_encoding::npdu::decode_npdu+0x21c>
10017ade8:     	mov	x25, #0x0               ; =0
10017adec:     	add	x8, x21, x20
10017adf0:     	str	xzr, [sp, #0x98]
10017adf4:     	adrp	x9, 0x100273000 <_anon.53300889abeb74b6fbdfa7dcf1d6cfd9.42+0x30>
10017adf8:     	add	x9, x9, #0xe20
10017adfc:     	stp	x9, x8, [sp, #0x80]
10017ae00:     	b	0x10017aee4 <bacnet_encoding::npdu::decode_npdu+0x278>
10017ae04:     	cmp	x20, #0x5
10017ae08:     	b.hs	0x10017ae20 <bacnet_encoding::npdu::decode_npdu+0x1b4>
10017ae0c:     	stp	xzr, xzr, [x22, #0x30]
10017ae10:     	mov	w8, #0x1                ; =1
10017ae14:     	adrp	x9, 0x10020f000 <_anon.53300889abeb74b6fbdfa7dcf1d6cfd9.86+0x508>
10017ae18:     	add	x9, x9, #0x548
10017ae1c:     	b	0x10017ae68 <bacnet_encoding::npdu::decode_npdu+0x1fc>
10017ae20:     	ldrh	w8, [x21, #0x2]
10017ae24:     	cbz	w8, 0x10017ae58 <bacnet_encoding::npdu::decode_npdu+0x1ec>
10017ae28:     	rev16	w9, w8
10017ae2c:     	ldrb	w1, [x21, #0x4]
10017ae30:     	cmp	x1, #0x12
10017ae34:     	b.ls	0x10017af60 <bacnet_encoding::npdu::decode_npdu+0x2f4>
10017ae38:     	str	xzr, [sp, #0x80]
10017ae3c:     	mov	w8, #0x1                ; =1
10017ae40:     	strh	w8, [sp, #0xa8]
10017ae44:     	strh	w9, [sp, #0xaa]
10017ae48:     	strb	wzr, [sp, #0xac]
10017ae4c:     	mov	w25, #0xff              ; =255
10017ae50:     	strb	w1, [sp, #0xad]
10017ae54:     	b	0x10017af94 <bacnet_encoding::npdu::decode_npdu+0x328>
10017ae58:     	stp	xzr, xzr, [x22, #0x30]
10017ae5c:     	mov	w8, #0x1                ; =1
10017ae60:     	adrp	x9, 0x10020f000 <_anon.53300889abeb74b6fbdfa7dcf1d6cfd9.86+0x508>
10017ae64:     	add	x9, x9, #0x436
10017ae68:     	stp	xzr, x9, [x22, #0x10]
10017ae6c:     	adrp	x9, 0x10020f000 <_anon.53300889abeb74b6fbdfa7dcf1d6cfd9.86+0x508>
10017ae70:     	ldr	q0, [x9, #0x390]
10017ae74:     	str	q0, [x22, #0x20]
10017ae78:     	strb	wzr, [x22, #0x40]
10017ae7c:     	mov	w9, #0x2                ; =2
10017ae80:     	stp	x9, x8, [x22]
10017ae84:     	b	0x10017b298 <bacnet_encoding::npdu::decode_npdu+0x62c>
10017ae88:     	stp	w0, w17, [sp, #0xc]
10017ae8c:     	str	w16, [sp, #0x14]
10017ae90:     	mov	x24, x15
10017ae94:     	mov	x27, x14
10017ae98:     	mov	x23, x13
10017ae9c:     	str	x12, [sp, #0x18]
10017aea0:     	mov	x0, x19
10017aea4:     	ldr	x8, [x0], #0x18
10017aea8:     	ldr	x9, [x8]
10017aeac:     	add	x8, sp, #0x80
10017aeb0:     	mov	x1, x21
10017aeb4:     	mov	x2, x20
10017aeb8:     	blr	x9
10017aebc:     	ldr	x8, [sp, #0x88]
10017aec0:     	add	x8, x8, x28
10017aec4:     	str	x8, [sp, #0x88]
10017aec8:     	ldp	x21, x20, [x19, #0x8]
10017aecc:     	ldr	x12, [sp, #0x18]
10017aed0:     	mov	x13, x23
10017aed4:     	mov	x14, x27
10017aed8:     	mov	x15, x24
10017aedc:     	ldp	w17, w16, [sp, #0x10]
10017aee0:     	ldr	w0, [sp, #0xc]
10017aee4:     	and	w8, w26, #0x3
10017aee8:     	str	x25, [sp, #0x90]
10017aeec:     	lsr	w9, w26, #7
10017aef0:     	ubfx	w10, w26, #2, #1
10017aef4:     	ldp	q0, q1, [sp, #0x80]
10017aef8:     	stp	q0, q1, [x22, #0x50]
10017aefc:     	ldr	x11, [sp, #0x50]
10017af00:     	str	x11, [x22]
10017af04:     	ldr	x11, [sp, #0x48]
10017af08:     	str	x11, [x22, #0x8]
10017af0c:     	ldr	x11, [sp, #0x28]
10017af10:     	str	x11, [x22, #0x10]
10017af14:     	ldr	x11, [sp, #0x58]
10017af18:     	str	x11, [x22, #0x18]
10017af1c:     	ldr	w11, [sp, #0x3c]
10017af20:     	strh	w11, [x22, #0x20]
10017af24:     	ldr	x11, [sp, #0x30]
10017af28:     	stp	x12, x11, [x22, #0x28]
10017af2c:     	ldr	x11, [sp, #0x40]
10017af30:     	stp	x13, x11, [x22, #0x38]
10017af34:     	strh	w14, [x22, #0x48]
10017af38:     	strh	w17, [x22, #0x70]
10017af3c:     	strh	w0, [x22, #0x72]
10017af40:     	strb	w16, [x22, #0x74]
10017af44:     	ldr	w11, [sp, #0x24]
10017af48:     	strb	w11, [x22, #0x75]
10017af4c:     	strb	w9, [x22, #0x76]
10017af50:     	strb	w10, [x22, #0x77]
10017af54:     	strb	w8, [x22, #0x78]
10017af58:     	strb	w15, [x22, #0x79]
10017af5c:     	b	0x10017b298 <bacnet_encoding::npdu::decode_npdu+0x62c>
10017af60:     	add	x8, x1, #0x5
10017af64:     	cmp	x8, x20
10017af68:     	b.ls	0x10017afd0 <bacnet_encoding::npdu::decode_npdu+0x364>
10017af6c:     	mov	w25, #0x0               ; =0
10017af70:     	mov	w8, #0x1                ; =1
10017af74:     	stp	x8, xzr, [sp, #0x80]
10017af78:     	adrp	x8, 0x10020f000 <_anon.53300889abeb74b6fbdfa7dcf1d6cfd9.86+0x508>
10017af7c:     	add	x8, x8, #0x3b0
10017af80:     	str	x8, [sp, #0x90]
10017af84:     	adrp	x8, 0x10020f000 <_anon.53300889abeb74b6fbdfa7dcf1d6cfd9.86+0x508>
10017af88:     	ldr	q0, [x8, #0x380]
10017af8c:     	stur	q0, [sp, #0x98]
10017af90:     	stp	x1, xzr, [sp, #0xa8]
10017af94:     	strb	w25, [sp, #0xb8]
10017af98:     	ldp	x27, x24, [sp, #0x80]
10017af9c:     	ldp	x9, x23, [sp, #0x90]
10017afa0:     	ldr	x28, [sp, #0xa0]
10017afa4:     	ldur	q0, [sp, #0xa8]
10017afa8:     	str	q0, [sp, #0x70]
10017afac:     	ldur	w8, [sp, #0xb9]
10017afb0:     	str	w8, [sp, #0x68]
10017afb4:     	ldr	w8, [sp, #0xbc]
10017afb8:     	stur	w8, [sp, #0x6b]
10017afbc:     	cmp	w25, #0xff
10017afc0:     	b.ne	0x10017b26c <bacnet_encoding::npdu::decode_npdu+0x600>
10017afc4:     	tbnz	w26, #0x3, 0x10017b20c <bacnet_encoding::npdu::decode_npdu+0x5a0>
10017afc8:     	mov	x26, #0x0               ; =0
10017afcc:     	b	0x10017b22c <bacnet_encoding::npdu::decode_npdu+0x5c0>
10017afd0:     	str	w9, [sp, #0x3c]
10017afd4:     	mov	x24, #0x0               ; =0
10017afd8:     	add	x8, sp, #0x80
10017afdc:     	add	x0, x21, #0x5
10017afe0:     	bl	0x10017a710 <<smallvec::SmallVec<[u8; 6]>>::from_slice>
10017afe4:     	ldp	x8, x9, [sp, #0x80]
10017afe8:     	str	x8, [sp, #0x48]
10017afec:     	ldr	x8, [sp, #0x90]
10017aff0:     	cmp	x8, #0x6
10017aff4:     	str	x8, [sp, #0x58]
10017aff8:     	str	x9, [sp, #0x28]
10017affc:     	csel	x8, x9, x8, hi
10017b000:     	add	x28, x8, #0x5
10017b004:     	mov	w24, #0x1               ; =1
10017b008:     	tbz	w26, #0x3, 0x10017adb8 <bacnet_encoding::npdu::decode_npdu+0x14c>
10017b00c:     	add	x27, x28, #0x3
10017b010:     	cmp	x27, x20
10017b014:     	b.ls	0x10017b02c <bacnet_encoding::npdu::decode_npdu+0x3c0>
10017b018:     	stp	xzr, xzr, [x22, #0x30]
10017b01c:     	mov	w8, #0x1                ; =1
10017b020:     	adrp	x9, 0x10020f000 <_anon.53300889abeb74b6fbdfa7dcf1d6cfd9.86+0x508>
10017b024:     	add	x9, x9, #0x528
10017b028:     	b	0x10017b078 <bacnet_encoding::npdu::decode_npdu+0x40c>
10017b02c:     	cmp	x28, x20
10017b030:     	b.hs	0x10017b3a0 <bacnet_encoding::npdu::decode_npdu+0x734>
10017b034:     	add	x8, x28, #0x1
10017b038:     	cmp	x8, x20
10017b03c:     	b.hs	0x10017b3b4 <bacnet_encoding::npdu::decode_npdu+0x748>
10017b040:     	ldrb	w9, [x21, x28]
10017b044:     	ldrb	w8, [x21, x8]
10017b048:     	orr	w23, w9, w8, lsl #8
10017b04c:     	rev	w8, w23
10017b050:     	mov	w9, #0xffff             ; =65535
10017b054:     	cmp	w9, w8, lsr #16
10017b058:     	b.eq	0x10017b0b0 <bacnet_encoding::npdu::decode_npdu+0x444>
10017b05c:     	and	w8, w23, #0xffff
10017b060:     	rev16	w8, w8
10017b064:     	cbnz	w8, 0x10017b0cc <bacnet_encoding::npdu::decode_npdu+0x460>
10017b068:     	stp	xzr, xzr, [x22, #0x30]
10017b06c:     	mov	w8, #0x1                ; =1
10017b070:     	adrp	x9, 0x10020f000 <_anon.53300889abeb74b6fbdfa7dcf1d6cfd9.86+0x508>
10017b074:     	add	x9, x9, #0x45b
10017b078:     	stp	xzr, x9, [x22, #0x10]
10017b07c:     	mov	w9, #0x20               ; =32
10017b080:     	stp	x9, x28, [x22, #0x20]
10017b084:     	strb	wzr, [x22, #0x40]
10017b088:     	mov	w9, #0x2                ; =2
10017b08c:     	stp	x9, x8, [x22]
10017b090:     	ldr	x1, [sp, #0x58]
10017b094:     	tbz	w26, #0x5, 0x10017b298 <bacnet_encoding::npdu::decode_npdu+0x62c>
10017b098:     	cmp	x1, #0x7
10017b09c:     	b.lo	0x10017b298 <bacnet_encoding::npdu::decode_npdu+0x62c>
10017b0a0:     	ldr	x0, [sp, #0x48]
10017b0a4:     	mov	w2, #0x1                ; =1
10017b0a8:     	bl	0x1000305cc <__rustc::__rust_dealloc>
10017b0ac:     	b	0x10017b298 <bacnet_encoding::npdu::decode_npdu+0x62c>
10017b0b0:     	stp	xzr, xzr, [x22, #0x30]
10017b0b4:     	mov	w8, #0x1                ; =1
10017b0b8:     	adrp	x9, 0x10020f000 <_anon.53300889abeb74b6fbdfa7dcf1d6cfd9.86+0x508>
10017b0bc:     	add	x9, x9, #0x47b
10017b0c0:     	stp	xzr, x9, [x22, #0x10]
10017b0c4:     	mov	w9, #0x25               ; =37
10017b0c8:     	b	0x10017b080 <bacnet_encoding::npdu::decode_npdu+0x414>
10017b0cc:     	add	x28, x28, #0x2
10017b0d0:     	cmp	x28, x20
10017b0d4:     	b.hs	0x10017b3cc <bacnet_encoding::npdu::decode_npdu+0x760>
10017b0d8:     	ldrb	w4, [x21, x28]
10017b0dc:     	cbz	w4, 0x10017b2d0 <bacnet_encoding::npdu::decode_npdu+0x664>
10017b0e0:     	add	x0, sp, #0x80
10017b0e4:     	mov	x1, x21
10017b0e8:     	mov	x2, x20
10017b0ec:     	mov	x3, x27
10017b0f0:     	mov	x5, x24
10017b0f4:     	ldr	w6, [sp, #0x3c]
10017b0f8:     	bl	0x10017a9c0 <bacnet_encoding::npdu::decode_address::<core::option::Option<u16>>>
10017b0fc:     	ldrb	w8, [sp, #0xb8]
10017b100:     	ldp	x11, x13, [sp, #0x80]
10017b104:     	ldr	x10, [sp, #0x90]
10017b108:     	cmp	w8, #0xfe
10017b10c:     	b.ne	0x10017b30c <bacnet_encoding::npdu::decode_npdu+0x6a0>
10017b110:     	str	x11, [sp, #0x30]
10017b114:     	rev16	w14, w23
10017b118:     	cmp	x10, #0x6
10017b11c:     	str	x10, [sp, #0x40]
10017b120:     	csel	x8, x13, x10, hi
10017b124:     	add	x28, x8, x27
10017b128:     	mov	w12, #0x1               ; =1
10017b12c:     	tbz	w26, #0x5, 0x10017adc0 <bacnet_encoding::npdu::decode_npdu+0x154>
10017b130:     	cmp	x28, x20
10017b134:     	b.hs	0x10017b1b4 <bacnet_encoding::npdu::decode_npdu+0x548>
10017b138:     	ldrb	w15, [x21, x28]
10017b13c:     	add	x28, x28, #0x1
10017b140:     	tbz	w26, #0x7, 0x10017adc8 <bacnet_encoding::npdu::decode_npdu+0x15c>
10017b144:     	cmp	x28, x20
10017b148:     	b.hs	0x10017b1c4 <bacnet_encoding::npdu::decode_npdu+0x558>
10017b14c:     	ldrsb	w10, [x21, x28]
10017b150:     	add	x8, x28, #0x1
10017b154:     	tbnz	w10, #0x1f, 0x10017b2ec <bacnet_encoding::npdu::decode_npdu+0x680>
10017b158:     	str	w10, [sp, #0x24]
10017b15c:     	str	x24, [sp, #0x50]
10017b160:     	mov	w17, #0x0               ; =0
10017b164:     	mov	w16, #0x1               ; =1
10017b168:     	mov	x28, x8
10017b16c:     	str	x8, [sp, #0x70]
10017b170:     	stur	x20, [x29, #-0x80]
10017b174:     	subs	x25, x20, x8
10017b178:     	b.hs	0x10017ade4 <bacnet_encoding::npdu::decode_npdu+0x178>
10017b17c:     	str	x12, [sp, #0x18]
10017b180:     	adrp	x8, 0x10017e000 <bacnet_encoding::tags::extract_context_value+0x1ac>
10017b184:     	add	x8, x8, #0x574
10017b188:     	add	x9, sp, #0x70
10017b18c:     	stp	x9, x8, [sp, #0x80]
10017b190:     	sub	x9, x29, #0x80
10017b194:     	stp	x9, x8, [sp, #0x90]
10017b198:     	adrp	x0, 0x1001f7000 <_anon.e81895284fd2357c11254d988870f03b.3+0x3b>
10017b19c:     	add	x0, x0, #0x3ca
10017b1a0:     	adrp	x2, 0x100273000 <_anon.53300889abeb74b6fbdfa7dcf1d6cfd9.42+0x30>
10017b1a4:     	add	x2, x2, #0xe08
10017b1a8:     	add	x1, sp, #0x80
10017b1ac:     	bl	0x1001dc790 <core::panicking::panic_fmt>
10017b1b0:     	b	0x10017b3e8 <bacnet_encoding::npdu::decode_npdu+0x77c>
10017b1b4:     	adrp	x9, 0x10020f000 <_anon.53300889abeb74b6fbdfa7dcf1d6cfd9.86+0x508>
10017b1b8:     	add	x9, x9, #0x50c
10017b1bc:     	mov	w10, #0x1c              ; =28
10017b1c0:     	b	0x10017b1d0 <bacnet_encoding::npdu::decode_npdu+0x564>
10017b1c4:     	adrp	x9, 0x10020f000 <_anon.53300889abeb74b6fbdfa7dcf1d6cfd9.86+0x508>
10017b1c8:     	add	x9, x9, #0x4e5
10017b1cc:     	mov	w10, #0x27              ; =39
10017b1d0:     	stp	xzr, xzr, [x22, #0x30]
10017b1d4:     	mov	w8, #0x1                ; =1
10017b1d8:     	stp	xzr, x9, [x22, #0x10]
10017b1dc:     	stp	x10, x28, [x22, #0x20]
10017b1e0:     	strb	wzr, [x22, #0x40]
10017b1e4:     	mov	w9, #0x2                ; =2
10017b1e8:     	stp	x9, x8, [x22]
10017b1ec:     	ldr	x1, [sp, #0x40]
10017b1f0:     	tbz	w26, #0x3, 0x10017b090 <bacnet_encoding::npdu::decode_npdu+0x424>
10017b1f4:     	cmp	x1, #0x7
10017b1f8:     	b.lo	0x10017b090 <bacnet_encoding::npdu::decode_npdu+0x424>
10017b1fc:     	ldr	x0, [sp, #0x30]
10017b200:     	mov	w2, #0x1                ; =1
10017b204:     	bl	0x1000305cc <__rustc::__rust_dealloc>
10017b208:     	b	0x10017b090 <bacnet_encoding::npdu::decode_npdu+0x424>
10017b20c:     	sub	x0, x29, #0x80
10017b210:     	add	x3, x1, #0x5
10017b214:     	mov	x1, x21
10017b218:     	mov	x2, x20
10017b21c:     	bl	0x10017b7bc <bacnet_encoding::npdu::source_behind>
10017b220:     	ldp	x26, x8, [x29, #-0x80]
10017b224:     	ldp	x9, x10, [x29, #-0x70]
10017b228:     	ldur	x28, [x29, #-0x60]
10017b22c:     	cbz	x27, 0x10017b260 <bacnet_encoding::npdu::decode_npdu+0x5f4>
10017b230:     	cmp	x23, #0x7
10017b234:     	b.lo	0x10017b260 <bacnet_encoding::npdu::decode_npdu+0x5f4>
10017b238:     	mov	x0, x24
10017b23c:     	mov	x1, x23
10017b240:     	mov	w2, #0x1                ; =1
10017b244:     	mov	x23, x9
10017b248:     	mov	x24, x8
10017b24c:     	mov	x27, x10
10017b250:     	bl	0x1000305cc <__rustc::__rust_dealloc>
10017b254:     	mov	x9, x23
10017b258:     	mov	x23, x27
10017b25c:     	b	0x10017b268 <bacnet_encoding::npdu::decode_npdu+0x5fc>
10017b260:     	mov	x23, x10
10017b264:     	mov	x24, x8
10017b268:     	mov	x27, x26
10017b26c:     	stp	x24, x9, [x22, #0x10]
10017b270:     	stp	x23, x28, [x22, #0x20]
10017b274:     	ldr	q0, [sp, #0x70]
10017b278:     	str	q0, [x22, #0x30]
10017b27c:     	strb	w25, [x22, #0x40]
10017b280:     	ldr	w8, [sp, #0x68]
10017b284:     	stur	w8, [x22, #0x41]
10017b288:     	ldur	w8, [sp, #0x6b]
10017b28c:     	str	w8, [x22, #0x44]
10017b290:     	mov	w8, #0x2                ; =2
10017b294:     	stp	x8, x27, [x22]
10017b298:     	ldr	x0, [x19, #0x18]
10017b29c:     	ldr	x8, [x19]
10017b2a0:     	ldr	x8, [x8, #0x20]
10017b2a4:     	mov	x1, x21
10017b2a8:     	mov	x2, x20
10017b2ac:     	blr	x8
10017b2b0:     	ldp	x29, x30, [sp, #0x140]
10017b2b4:     	ldp	x20, x19, [sp, #0x130]
10017b2b8:     	ldp	x22, x21, [sp, #0x120]
10017b2bc:     	ldp	x24, x23, [sp, #0x110]
10017b2c0:     	ldp	x26, x25, [sp, #0x100]
10017b2c4:     	ldp	x28, x27, [sp, #0xf0]
10017b2c8:     	add	sp, sp, #0x150
10017b2cc:     	ret
10017b2d0:     	stp	xzr, xzr, [x22, #0x30]
10017b2d4:     	mov	w8, #0x1                ; =1
10017b2d8:     	adrp	x9, 0x10020f000 <_anon.53300889abeb74b6fbdfa7dcf1d6cfd9.86+0x508>
10017b2dc:     	add	x9, x9, #0x4a0
10017b2e0:     	stp	xzr, x9, [x22, #0x10]
10017b2e4:     	mov	w9, #0x1d               ; =29
10017b2e8:     	b	0x10017b080 <bacnet_encoding::npdu::decode_npdu+0x414>
10017b2ec:     	add	x9, x28, #0x3
10017b2f0:     	cmp	x9, x20
10017b2f4:     	b.ls	0x10017b33c <bacnet_encoding::npdu::decode_npdu+0x6d0>
10017b2f8:     	adrp	x9, 0x10020f000 <_anon.53300889abeb74b6fbdfa7dcf1d6cfd9.86+0x508>
10017b2fc:     	add	x9, x9, #0x4bd
10017b300:     	mov	w10, #0x28              ; =40
10017b304:     	mov	x28, x8
10017b308:     	b	0x10017b1d0 <bacnet_encoding::npdu::decode_npdu+0x564>
10017b30c:     	ldur	q0, [sp, #0x98]
10017b310:     	ldur	q1, [sp, #0xa8]
10017b314:     	stp	q0, q1, [x22, #0x20]
10017b318:     	ldur	w9, [sp, #0xb9]
10017b31c:     	stur	w9, [x22, #0x41]
10017b320:     	ldr	w9, [sp, #0xbc]
10017b324:     	str	w9, [x22, #0x44]
10017b328:     	stp	x13, x10, [x22, #0x10]
10017b32c:     	strb	w8, [x22, #0x40]
10017b330:     	mov	w8, #0x2                ; =2
10017b334:     	stp	x8, x11, [x22]
10017b338:     	b	0x10017b090 <bacnet_encoding::npdu::decode_npdu+0x424>
10017b33c:     	cmp	x8, x20
10017b340:     	str	x24, [sp, #0x50]
10017b344:     	b.hs	0x10017b3ec <bacnet_encoding::npdu::decode_npdu+0x780>
10017b348:     	str	w10, [sp, #0x24]
10017b34c:     	add	x28, x28, #0x2
10017b350:     	cmp	x28, x20
10017b354:     	b.hs	0x10017b400 <bacnet_encoding::npdu::decode_npdu+0x794>
10017b358:     	ldrb	w8, [x21, x8]
10017b35c:     	ldrb	w10, [x21, x28]
10017b360:     	orr	w8, w8, w10, lsl #8
10017b364:     	rev16	w0, w8
10017b368:     	mov	w16, #0x1               ; =1
10017b36c:     	mov	x28, x9
10017b370:     	mov	w17, #0x1               ; =1
10017b374:     	str	x9, [sp, #0x70]
10017b378:     	stur	x20, [x29, #-0x80]
10017b37c:     	subs	x25, x20, x9
10017b380:     	b.hs	0x10017ade4 <bacnet_encoding::npdu::decode_npdu+0x178>
10017b384:     	b	0x10017b17c <bacnet_encoding::npdu::decode_npdu+0x510>
10017b388:     	adrp	x0, 0x100278000 <dyld_stub_binder+0x100278000>
10017b38c:     	add	x0, x0, #0x550
10017b390:     	bl	0x1001d6f40 <<tracing_core::callsite::DefaultCallsite>::register>
10017b394:     	and	w1, w0, #0xff
10017b398:     	cbnz	w1, 0x10017ad0c <bacnet_encoding::npdu::decode_npdu+0xa0>
10017b39c:     	b	0x10017ada8 <bacnet_encoding::npdu::decode_npdu+0x13c>
10017b3a0:     	str	x24, [sp, #0x50]
10017b3a4:     	str	xzr, [sp, #0x18]
10017b3a8:     	adrp	x2, 0x100273000 <_anon.53300889abeb74b6fbdfa7dcf1d6cfd9.42+0x30>
10017b3ac:     	add	x2, x2, #0xc28
10017b3b0:     	b	0x10017b3dc <bacnet_encoding::npdu::decode_npdu+0x770>
10017b3b4:     	str	x24, [sp, #0x50]
10017b3b8:     	str	xzr, [sp, #0x18]
10017b3bc:     	adrp	x2, 0x100273000 <_anon.53300889abeb74b6fbdfa7dcf1d6cfd9.42+0x30>
10017b3c0:     	add	x2, x2, #0xc40
10017b3c4:     	mov	x28, x8
10017b3c8:     	b	0x10017b3dc <bacnet_encoding::npdu::decode_npdu+0x770>
10017b3cc:     	str	x24, [sp, #0x50]
10017b3d0:     	str	xzr, [sp, #0x18]
10017b3d4:     	adrp	x2, 0x100273000 <_anon.53300889abeb74b6fbdfa7dcf1d6cfd9.42+0x30>
10017b3d8:     	add	x2, x2, #0xc58
10017b3dc:     	mov	x0, x28
10017b3e0:     	mov	x1, x20
10017b3e4:     	bl	0x1001dc5f8 <core::panicking::panic_bounds_check>
10017b3e8:     	brk	#0x1
10017b3ec:     	str	x12, [sp, #0x18]
10017b3f0:     	adrp	x2, 0x100273000 <_anon.53300889abeb74b6fbdfa7dcf1d6cfd9.42+0x30>
10017b3f4:     	add	x2, x2, #0xc70
10017b3f8:     	mov	x28, x8
10017b3fc:     	b	0x10017b3dc <bacnet_encoding::npdu::decode_npdu+0x770>
10017b400:     	str	x12, [sp, #0x18]
10017b404:     	adrp	x2, 0x100273000 <_anon.53300889abeb74b6fbdfa7dcf1d6cfd9.42+0x30>
10017b408:     	add	x2, x2, #0xc88
10017b40c:     	b	0x10017b3dc <bacnet_encoding::npdu::decode_npdu+0x770>
10017b410:     	mov	x20, x0
10017b414:     	add	x0, sp, #0x80
10017b418:     	bl	0x10017ac28 <core::ptr::drop_glue::<bacnet_encoding::npdu::decode_error::NpduDecodeError>>
10017b41c:     	b	0x10017b480 <bacnet_encoding::npdu::decode_npdu+0x814>
10017b420:     	str	x24, [sp, #0x50]
10017b424:     	mov	x20, x0
10017b428:     	b	0x10017b45c <bacnet_encoding::npdu::decode_npdu+0x7f0>
10017b42c:     	mov	x20, x0
10017b430:     	b	0x10017b480 <bacnet_encoding::npdu::decode_npdu+0x814>
10017b434:     	mov	x20, x0
10017b438:     	ldr	x8, [sp, #0x18]
10017b43c:     	cbz	x8, 0x10017b45c <bacnet_encoding::npdu::decode_npdu+0x7f0>
10017b440:     	ldr	x8, [sp, #0x40]
10017b444:     	cmp	x8, #0x7
10017b448:     	b.lo	0x10017b45c <bacnet_encoding::npdu::decode_npdu+0x7f0>
10017b44c:     	ldr	x0, [sp, #0x30]
10017b450:     	ldr	x1, [sp, #0x40]
10017b454:     	mov	w2, #0x1                ; =1
10017b458:     	bl	0x1000305cc <__rustc::__rust_dealloc>
10017b45c:     	ldr	x8, [sp, #0x50]
10017b460:     	cbz	x8, 0x10017b480 <bacnet_encoding::npdu::decode_npdu+0x814>
10017b464:     	ldr	x8, [sp, #0x58]
10017b468:     	cmp	x8, #0x7
10017b46c:     	b.lo	0x10017b480 <bacnet_encoding::npdu::decode_npdu+0x814>
10017b470:     	ldr	x0, [sp, #0x48]
10017b474:     	ldr	x1, [sp, #0x58]
10017b478:     	mov	w2, #0x1                ; =1
10017b47c:     	bl	0x1000305cc <__rustc::__rust_dealloc>
10017b480:     	ldp	x8, x1, [x19]
10017b484:     	ldr	x8, [x8, #0x20]
10017b488:     	ldp	x2, x0, [x19, #0x10]
10017b48c:     	blr	x8
10017b490:     	mov	x0, x20
10017b494:     	bl	0x1001dca58 <dyld_stub_binder+0x1001dca58>
10017b498:     	bl	0x1001dc5e0 <core::panicking::panic_in_cleanup>

