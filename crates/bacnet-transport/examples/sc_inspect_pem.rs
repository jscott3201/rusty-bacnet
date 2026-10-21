//! Inspect existing local credential files without a running hub.
//!
//! cargo run -p bacnet-transport --features sc-tls --example sc_inspect_pem -- node.pem site-ca.pem node.key
#![allow(clippy::print_stdout, clippy::print_stderr)] // This example displays the requested report.

#[cfg(feature = "sc-tls")]
#[path = "support/sc_inspection_display.rs"]
mod display;

#[cfg(feature = "sc-tls")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::path::Path;
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.is_empty() || args.len() > 3 {
        return Err("usage: sc_inspect_pem IDENTITY.pem [PEER-TRUST.pem [KEY.pem]]".into());
    }
    display::inspect_and_write(
        &mut std::io::stdout().lock(),
        Path::new(&args[0]),
        args.get(1).map(Path::new),
        args.get(2).map(Path::new),
        None,
    )?;
    Ok(())
}

#[cfg(not(feature = "sc-tls"))]
fn main() {
    eprintln!("enable --features sc-tls to run this example");
}
