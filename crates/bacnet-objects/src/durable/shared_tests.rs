//! The parts of the save machinery that need no writer thread, so these run
//! in the alloc-only build too (#2048).

use super::*;
use std::task::Wake;

#[cfg_attr(not(feature = "durable-writes"), allow(dead_code))]
struct Flag(std::sync::atomic::AtomicBool);

impl Wake for Flag {
    fn wake(self: Arc<Self>) {
        self.0.store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

#[cfg(feature = "durable-writes")]
/// A joined wait ends only once every wait in it has, in either order, and
/// wakes its task for the later one (#1526).
#[test]
fn a_joined_wait_ends_with_the_last_of_its_waits() {
    for first_set in [0, 2] {
        let events: Vec<Arc<Event>> = (0..3).map(|_| Arc::default()).collect();
        let waits = events.iter().map(|event| SaveWait::new(Arc::clone(event)));
        let mut joined = waits.reduce(SaveWait::and).unwrap();
        let flag = Arc::new(Flag(Default::default()));
        let waker = Waker::from(Arc::clone(&flag));
        let mut cx = Context::from_waker(&waker);
        let mut order: Vec<usize> = (0..3).collect();
        order.rotate_left(first_set);
        for (count, &index) in order.iter().enumerate() {
            assert!(!joined.is_ready());
            assert!(Pin::new(&mut joined).poll(&mut cx).is_pending());
            flag.0.store(false, std::sync::atomic::Ordering::SeqCst);
            events[index].set();
            if count + 1 < order.len() {
                continue;
            }
            assert!(flag.0.load(std::sync::atomic::Ordering::SeqCst));
        }
        assert!(joined.is_ready());
        assert!(Pin::new(&mut joined).poll(&mut cx).is_ready());
        joined.block();
    }
}

#[test]
fn crc32_matches_the_check_value_and_the_bitwise_form() {
    // The CRC-32 catalogue's check value, over the ASCII digits 1 to 9.
    assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
    assert_eq!(crc32(&[]), 0);
    let bitwise = |data: &[u8]| {
        let mut crc = !0u32;
        for byte in data {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                crc = (crc >> 1) ^ (0xedb8_8320 & 0u32.wrapping_sub(crc & 1));
            }
        }
        !crc
    };
    let data: Vec<u8> = (0..=255u8).chain((0..=255u8).rev()).collect();
    for end in 0..data.len() {
        assert_eq!(crc32(&data[..end]), bitwise(&data[..end]));
    }
}
