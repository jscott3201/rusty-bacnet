use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

use bacnet_benchmarks::helpers::{make_bip_client, make_bip_server};
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;

struct Counting;
static ALLOCS: AtomicU64 = AtomicU64::new(0);
static BYTES: AtomicU64 = AtomicU64::new(0);
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Relaxed);
        BYTES.fetch_add(l.size() as u64, Relaxed);
        System.alloc(l)
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        System.dealloc(p, l)
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        ALLOCS.fetch_add(1, Relaxed);
        BYTES.fetch_add(n as u64, Relaxed);
        System.realloc(p, l, n)
    }
}
#[global_allocator]
static A: Counting = Counting;

fn main() {
    let mut args = std::env::args().skip(1);
    let n: u64 = args.next().unwrap().parse().unwrap();
    let mode = args.next().unwrap_or_default();
    let mt = mode == "mt";
    let wp = args.next().as_deref() == Some("wp");
    let rt = if mt {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap()
    } else {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    };
    rt.block_on(async move {
        let mut server = make_bip_server().await.unwrap();
        let mut client = make_bip_client().await.unwrap();
        let mac = server.local_mac().to_vec();
        let ai = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
        let ao = ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 1).unwrap();
        let mut vb = bytes::BytesMut::new();
        bacnet_encoding::primitives::encode_app_real(&mut vb, 72.5);
        let vd = vb.to_vec();
        macro_rules! one {
            () => {
                if wp {
                    client
                        .write_property(&mac, ao, PropertyIdentifier::PRESENT_VALUE, None, vd.clone(), Some(16))
                        .await
                        .unwrap();
                } else {
                    client
                        .read_property(&mac, ai, PropertyIdentifier::PRESENT_VALUE, None)
                        .await
                        .unwrap();
                }
            };
        }
        let mut expected_ai = bytes::BytesMut::new();
        bacnet_encoding::primitives::encode_app_real(&mut expected_ai, 21.0);
        macro_rules! verify {
            () => {{
                let object = if wp { ao } else { ai };
                let ack = client.read_property(&mac, object, PropertyIdentifier::PRESENT_VALUE, None).await.unwrap();
                assert_eq!(ack.object_identifier, object);
                assert_eq!(ack.property_identifier, PropertyIdentifier::PRESENT_VALUE);
                assert_eq!(ack.property_array_index, None);
                let expected = if wp { vd.as_slice() } else { expected_ai.as_ref() };
                assert_eq!(ack.property_value.as_slice(), expected);
            }};
        }
        for _ in 0..200 {
            one!();
        }
        verify!();
        let a0 = ALLOCS.load(Relaxed);
        let b0 = BYTES.load(Relaxed);
        for _ in 0..n {
            one!();
        }
        let a = ALLOCS.load(Relaxed) - a0;
        let b = BYTES.load(Relaxed) - b0;
        eprintln!(
            "RESULT n={n} mt={mt} wp={wp} workers={} allocs={a} bytes={b} allocs/req={:.6} bytes/req={:.6}",
            if mt { 2 } else { 1 },
            a as f64 / n as f64,
            b as f64 / n as f64
        );
        verify!();
        let _ = client.stop().await;
        let _ = server.stop().await;
    });
}
