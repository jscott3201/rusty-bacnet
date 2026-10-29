//! Run the router guide's program (#1989): a two-port B/IP router on
//! 127.0.0.1, a device behind it, and a client that finds the router and
//! reads the device through it.
#![forbid(unsafe_code)]

use std::process::Command;

#[test]
fn the_client_reads_the_device_through_the_router() {
    let output = Command::new(env!("CARGO_BIN_EXE_guide-bip-router"))
        .output()
        .expect("run the guide program");
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "Router started: networks 1 and 5\n\
         Router to network 5 found, reaching networks [5]\n\
         Device 1005 found on network 5\n\
         Read through the router: 8.5\n\
         Stopped\n"
    );
}
