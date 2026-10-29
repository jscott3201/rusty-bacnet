00000001000305dc <<bacnet_npdu::view::Npdu>::decode>:
1000305dc:     	sub	sp, sp, #0x50
1000305e0:     	stp	x22, x21, [sp, #0x20]
1000305e4:     	stp	x20, x19, [sp, #0x30]
1000305e8:     	stp	x29, x30, [sp, #0x40]
1000305ec:     	add	x29, sp, #0x40
1000305f0:     	mov	x2, x1
1000305f4:     	cmp	x1, #0x1
1000305f8:     	b.ls	0x1000307e8 <<bacnet_npdu::view::Npdu>::decode+0x20c>
1000305fc:     	mov	x1, x0
100030600:     	ldrb	w9, [x0]
100030604:     	cmp	w9, #0x1
100030608:     	b.ne	0x10003063c <<bacnet_npdu::view::Npdu>::decode+0x60>
10003060c:     	ldrb	w9, [x1, #0x1]
100030610:     	tbnz	w9, #0x5, 0x100030648 <<bacnet_npdu::view::Npdu>::decode+0x6c>
100030614:     	mov	x10, #0x0               ; =0
100030618:     	mov	w0, #0x2                ; =2
10003061c:     	tbnz	w9, #0x3, 0x100030688 <<bacnet_npdu::view::Npdu>::decode+0xac>
100030620:     	mov	x13, #0x0               ; =0
100030624:     	cbz	x10, 0x10003075c <<bacnet_npdu::view::Npdu>::decode+0x180>
100030628:     	cmp	x0, x2
10003062c:     	b.hs	0x1000307e8 <<bacnet_npdu::view::Npdu>::decode+0x20c>
100030630:     	ldrb	w14, [x1, x0]
100030634:     	add	x0, x0, #0x1
100030638:     	b	0x100030760 <<bacnet_npdu::view::Npdu>::decode+0x184>
10003063c:     	mov	w9, #0x1                ; =1
100030640:     	strb	w9, [x8]
100030644:     	b	0x1000307ec <<bacnet_npdu::view::Npdu>::decode+0x210>
100030648:     	cmp	x2, #0x5
10003064c:     	b.lo	0x1000307e8 <<bacnet_npdu::view::Npdu>::decode+0x20c>
100030650:     	ldrh	w10, [x1, #0x2]
100030654:     	cbz	w10, 0x1000306ec <<bacnet_npdu::view::Npdu>::decode+0x110>
100030658:     	rev16	w19, w10
10003065c:     	ldrb	w20, [x1, #0x4]
100030660:     	cmp	w20, #0x12
100030664:     	b.ls	0x100030674 <<bacnet_npdu::view::Npdu>::decode+0x98>
100030668:     	tbnz	w9, #0x3, 0x1000306f8 <<bacnet_npdu::view::Npdu>::decode+0x11c>
10003066c:     	str	xzr, [sp, #0x8]
100030670:     	b	0x10003070c <<bacnet_npdu::view::Npdu>::decode+0x130>
100030674:     	add	x0, x20, #0x5
100030678:     	cmp	x0, x2
10003067c:     	b.hi	0x1000307e8 <<bacnet_npdu::view::Npdu>::decode+0x20c>
100030680:     	add	x10, x1, #0x5
100030684:     	tbz	w9, #0x3, 0x100030620 <<bacnet_npdu::view::Npdu>::decode+0x44>
100030688:     	add	x13, x0, #0x3
10003068c:     	cmp	x13, x2
100030690:     	b.hi	0x1000307e8 <<bacnet_npdu::view::Npdu>::decode+0x20c>
100030694:     	add	x11, x1, x0
100030698:     	ldrh	w12, [x11]
10003069c:     	rev	w14, w12
1000306a0:     	lsr	w14, w14, #16
1000306a4:     	sub	w14, w14, #0x1
1000306a8:     	mov	w15, #0xfffe            ; =65534
1000306ac:     	cmp	w15, w14, uxth
1000306b0:     	b.ls	0x1000306ec <<bacnet_npdu::view::Npdu>::decode+0x110>
1000306b4:     	ldrb	w11, [x11, #0x2]
1000306b8:     	cbz	w11, 0x100030738 <<bacnet_npdu::view::Npdu>::decode+0x15c>
1000306bc:     	cmp	w11, #0x12
1000306c0:     	b.ls	0x100030744 <<bacnet_npdu::view::Npdu>::decode+0x168>
1000306c4:     	cmp	x10, #0x0
1000306c8:     	str	xzr, [x8]
1000306cc:     	cset	w9, ne
1000306d0:     	strh	w9, [x8, #0x18]
1000306d4:     	strh	w19, [x8, #0x1a]
1000306d8:     	mov	w9, #0x1                ; =1
1000306dc:     	strb	w9, [x8, #0x1c]
1000306e0:     	strb	w11, [x8, #0x1d]
1000306e4:     	mov	w9, #0x2                ; =2
1000306e8:     	b	0x1000307f4 <<bacnet_npdu::view::Npdu>::decode+0x218>
1000306ec:     	mov	w9, #0x2                ; =2
1000306f0:     	strb	w9, [x8]
1000306f4:     	b	0x1000307f0 <<bacnet_npdu::view::Npdu>::decode+0x214>
1000306f8:     	add	x0, sp, #0x8
1000306fc:     	add	x3, x20, #0x5
100030700:     	mov	x21, x8
100030704:     	bl	0x100030830 <bacnet_npdu::view::source_behind>
100030708:     	mov	x8, x21
10003070c:     	ldur	q0, [sp, #0x8]
100030710:     	str	q0, [x8]
100030714:     	ldr	x9, [sp, #0x18]
100030718:     	str	x9, [x8, #0x10]
10003071c:     	mov	w9, #0x1                ; =1
100030720:     	strh	w9, [x8, #0x18]
100030724:     	strh	w19, [x8, #0x1a]
100030728:     	strb	wzr, [x8, #0x1c]
10003072c:     	strb	w20, [x8, #0x1d]
100030730:     	mov	w9, #0x2                ; =2
100030734:     	b	0x1000307f4 <<bacnet_npdu::view::Npdu>::decode+0x218>
100030738:     	mov	w9, #0x3                ; =3
10003073c:     	strb	w9, [x8]
100030740:     	b	0x1000307ec <<bacnet_npdu::view::Npdu>::decode+0x210>
100030744:     	add	x0, x13, x11
100030748:     	cmp	x0, x2
10003074c:     	b.hi	0x1000307e8 <<bacnet_npdu::view::Npdu>::decode+0x20c>
100030750:     	rev16	w12, w12
100030754:     	add	x13, x1, x13
100030758:     	cbnz	x10, 0x100030628 <<bacnet_npdu::view::Npdu>::decode+0x4c>
10003075c:     	mov	w14, #0xff              ; =255
100030760:     	tbnz	w9, #0x7, 0x1000307b8 <<bacnet_npdu::view::Npdu>::decode+0x1dc>
100030764:     	mov	w16, #0x0               ; =0
100030768:     	mov	w3, #0x0                ; =0
10003076c:     	subs	x4, x2, x0
100030770:     	b.lo	0x100030820 <<bacnet_npdu::view::Npdu>::decode+0x244>
100030774:     	and	w2, w9, #0x3
100030778:     	add	x1, x1, x0
10003077c:     	ubfx	w9, w9, #2, #1
100030780:     	stp	x1, x4, [x8]
100030784:     	stp	x10, x20, [x8, #0x10]
100030788:     	strh	w19, [x8, #0x20]
10003078c:     	stp	x13, x11, [x8, #0x28]
100030790:     	strh	w12, [x8, #0x38]
100030794:     	str	x0, [x8, #0x40]
100030798:     	strb	w3, [x8, #0x48]
10003079c:     	strb	w15, [x8, #0x49]
1000307a0:     	strh	w16, [x8, #0x4a]
1000307a4:     	strh	w17, [x8, #0x4c]
1000307a8:     	strb	w14, [x8, #0x4e]
1000307ac:     	strb	w2, [x8, #0x4f]
1000307b0:     	strb	w9, [x8, #0x50]
1000307b4:     	b	0x1000307f8 <<bacnet_npdu::view::Npdu>::decode+0x21c>
1000307b8:     	cmp	x0, x2
1000307bc:     	b.hs	0x1000307e8 <<bacnet_npdu::view::Npdu>::decode+0x20c>
1000307c0:     	ldrsb	w15, [x1, x0]
1000307c4:     	add	x17, x0, #0x1
1000307c8:     	tbnz	w15, #0x1f, 0x1000307dc <<bacnet_npdu::view::Npdu>::decode+0x200>
1000307cc:     	mov	w16, #0x0               ; =0
1000307d0:     	mov	w3, #0x1                ; =1
1000307d4:     	mov	x0, x17
1000307d8:     	b	0x10003076c <<bacnet_npdu::view::Npdu>::decode+0x190>
1000307dc:     	add	x0, x0, #0x3
1000307e0:     	cmp	x0, x2
1000307e4:     	b.ls	0x10003080c <<bacnet_npdu::view::Npdu>::decode+0x230>
1000307e8:     	strb	wzr, [x8]
1000307ec:     	mov	w9, #0x2                ; =2
1000307f0:     	strh	w9, [x8, #0x18]
1000307f4:     	strh	w9, [x8, #0x4a]
1000307f8:     	ldp	x29, x30, [sp, #0x40]
1000307fc:     	ldp	x20, x19, [sp, #0x30]
100030800:     	ldp	x22, x21, [sp, #0x20]
100030804:     	add	sp, sp, #0x50
100030808:     	ret
10003080c:     	ldrh	w16, [x1, x17]
100030810:     	rev16	w17, w16
100030814:     	mov	w16, #0x1               ; =1
100030818:     	mov	w3, #0x1                ; =1
10003081c:     	b	0x10003076c <<bacnet_npdu::view::Npdu>::decode+0x190>
100030820:     	adrp	x3, 0x10026a000 <_anon.be9598f3412941a7e0e20546922aa682.10+0x190>
100030824:     	add	x3, x3, #0xc0
100030828:     	mov	x1, x2
10003082c:     	bl	0x1001dc7f8 <core::slice::index::slice_index_fail>

