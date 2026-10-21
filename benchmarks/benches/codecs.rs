//! Criterion suite: the portable `no_std` codecs and cores. B/IP BVLL
//! encode and decode, MS/TP frame encode and decode (standard and extended
//! frames, and the CRCs behind them), and the router core's NPDU parse and
//! rewrite next to `bacnet-encoding`'s owned NPDU decoder.
use std::hint::black_box;
use std::net::{Ipv4Addr, SocketAddrV4};

use bytes::Bytes;
use criterion::{criterion_group, criterion_main, BatchSize, Criterion};

use bacnet_bip_codec::Bvll;
use bacnet_mstp_codec::{
    crc, decode_frame, decode_frame_in_place, Frame, FrameType, MAX_EXTENDED_FRAME_LENGTH,
    MAX_EXTENDED_MPDU_DATA, MAX_STANDARD_FRAME_LENGTH, MAX_STANDARD_MPDU_DATA,
};
use bacnet_router_core::npdu::{Address, Npdu};

/// A ReadProperty request NPDU: the 2-octet NPCI and an 11-octet APDU.
const RP_NPDU: [u8; 13] = [
    0x01, 0x04, 0x00, 0x05, 0x01, 0x0C, 0x0C, 0x00, 0x00, 0x00, 0x01, 0x19, 0x55,
];

/// `len` octets of a non-zero pattern, so COBS has real work to do.
fn pattern(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i % 251) as u8 + 1).collect()
}

fn bench_bvll(c: &mut Criterion) {
    let mut group = c.benchmark_group("bvll");
    let small = Bvll::OriginalUnicastNpdu(&RP_NPDU);
    let large_npdu = pattern(MAX_EXTENDED_MPDU_DATA);
    let origin = SocketAddrV4::new(Ipv4Addr::new(192, 168, 1, 10), 0xBAC0);
    let forwarded = Bvll::ForwardedNpdu {
        origin,
        npdu: &large_npdu,
    };
    let mut out = [0u8; 1600];

    group.bench_function("encode_unicast_13b_npdu", |b| {
        b.iter(|| black_box(black_box(&small).encode_into(&mut out).unwrap()))
    });
    let length = small.encode_into(&mut out).unwrap();
    let small_wire = out[..length].to_vec();
    group.bench_function("decode_unicast_13b_npdu", |b| {
        b.iter(|| black_box(bacnet_bip_codec::decode(black_box(&small_wire)).unwrap()))
    });
    group.bench_function("encode_forwarded_1497b_npdu", |b| {
        b.iter(|| black_box(black_box(&forwarded).encode_into(&mut out).unwrap()))
    });
    let length = forwarded.encode_into(&mut out).unwrap();
    let forwarded_wire = out[..length].to_vec();
    group.bench_function("decode_forwarded_1497b_npdu", |b| {
        b.iter(|| black_box(bacnet_bip_codec::decode(black_box(&forwarded_wire)).unwrap()))
    });
    group.finish();
}

fn bench_mstp(c: &mut Criterion) {
    let mut group = c.benchmark_group("mstp");
    let token = Frame::new(FrameType::TOKEN, 1, 0, &[]);
    let standard_data = pattern(MAX_STANDARD_MPDU_DATA);
    let standard = Frame::new(FrameType::BACNET_DATA_EXPECTING_REPLY, 1, 0, &standard_data);
    let extended_data = pattern(MAX_EXTENDED_MPDU_DATA);
    let extended = Frame::new(
        FrameType::BACNET_EXTENDED_DATA_EXPECTING_REPLY,
        1,
        0,
        &extended_data,
    );
    let mut out = [0u8; MAX_EXTENDED_FRAME_LENGTH];

    group.bench_function("encode_token", |b| {
        b.iter(|| black_box(black_box(token).encode_into(&mut out).unwrap()))
    });
    group.bench_function("encode_standard_501b", |b| {
        b.iter(|| black_box(black_box(standard).encode_into(&mut out).unwrap()))
    });
    let length = standard.encode_into(&mut out).unwrap();
    let standard_wire = out[..length].to_vec();
    assert!(length <= MAX_STANDARD_FRAME_LENGTH);
    group.bench_function("decode_standard_501b", |b| {
        b.iter(|| black_box(decode_frame(black_box(&standard_wire)).unwrap()))
    });
    group.bench_function("encode_extended_1497b", |b| {
        b.iter(|| black_box(black_box(extended).encode_into(&mut out).unwrap()))
    });
    let length = extended.encode_into(&mut out).unwrap();
    let extended_wire = out[..length].to_vec();
    // Decoding an extended frame writes over its input, so each iteration
    // gets a fresh copy, made outside the timed part.
    group.bench_function("decode_extended_1497b", |b| {
        b.iter_batched_ref(
            || extended_wire.clone(),
            |wire| {
                black_box(decode_frame_in_place(wire).unwrap());
            },
            BatchSize::SmallInput,
        )
    });
    group.bench_function("crc16_501b", |b| {
        b.iter(|| black_box(crc::crc16(black_box(&standard_data))))
    });
    group.bench_function("crc32k_1497b", |b| {
        b.iter(|| black_box(crc::crc32k(black_box(&extended_data))))
    });
    group.finish();
}

fn bench_router_npdu(c: &mut Criterion) {
    let mut group = c.benchmark_group("router_npdu");
    // A routed ReadProperty: DNET 2 and a 6-octet DADR, hop count 255.
    let mut routed = vec![
        0x01, 0x24, 0x00, 0x02, 0x06, 192, 168, 2, 10, 0xBA, 0xC0, 0xFF,
    ];
    routed.extend_from_slice(&RP_NPDU[2..]);
    let sadr = [192, 168, 1, 20, 0xBA, 0xC0];
    let mut out = [0u8; 64];

    group.bench_function("parse_routed_request", |b| {
        b.iter(|| black_box(Npdu::decode(black_box(&routed)).unwrap()))
    });
    // What a router does to a request for a network it is on: drop DNET and
    // DADR, add SNET and SADR, then write the result.
    group.bench_function("parse_and_rewrite_for_final_hop", |b| {
        b.iter(|| {
            let npdu = Npdu::decode(black_box(&routed)).unwrap();
            let rewritten = Npdu {
                destination: None,
                source: Some(Address {
                    network: 1,
                    mac: &sadr,
                }),
                hop_count: 0,
                ..npdu
            };
            black_box(rewritten.encode_into(&mut out).unwrap())
        })
    });
    let routed_bytes = Bytes::from(routed.clone());
    group.bench_function("encoding_decode_npdu_routed_request", |b| {
        b.iter(|| black_box(bacnet_encoding::npdu::decode_npdu(routed_bytes.clone()).unwrap()))
    });
    group.finish();
}

criterion_group!(benches, bench_bvll, bench_mstp, bench_router_npdu);
criterion_main!(benches);
