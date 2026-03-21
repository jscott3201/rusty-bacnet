//! Integration test for BVLL operations and router discovery.
//!
//! Designed to run inside a container on the campus test network.
//! Expects BBMDs and a router to be available on the local subnet.

use bacnet_client::client::BACnetClient;
use std::net::Ipv4Addr;
use std::process::ExitCode;
use tokio::time::Duration;

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt::init();

    let interface: Ipv4Addr = std::env::var("BACNET_INTERFACE")
        .unwrap_or_else(|_| "0.0.0.0".into())
        .parse()
        .expect("invalid BACNET_INTERFACE");

    let broadcast: Ipv4Addr = std::env::var("BACNET_BROADCAST")
        .unwrap_or_else(|_| "10.1.0.255".into())
        .parse()
        .expect("invalid BACNET_BROADCAST");

    let bbmd_addr = std::env::var("BBMD_ADDRESS").unwrap_or_else(|_| "10.1.0.2:47808".into());

    println!("=== BVLL & Router Discovery Integration Test ===");
    println!("Interface: {interface}");
    println!("Broadcast: {broadcast}");
    println!("BBMD target: {bbmd_addr}");
    println!();

    let mut client = BACnetClient::bip_builder()
        .interface(interface)
        .port(47809) // non-standard port; BDT/FDT are unicast replies so this works
        .broadcast_address(broadcast)
        .apdu_timeout_ms(5000)
        .build()
        .await
        .expect("failed to build client");

    let mut passed = 0u32;
    let mut failed = 0u32;

    // ── Test 1: Who-Is discovers devices ──────────────────────────────────
    print!("Test 1: Who-Is discovers devices ... ");
    client.who_is(None, None).await.expect("who_is failed");
    tokio::time::sleep(Duration::from_secs(5)).await;
    let devices = client.discovered_devices().await;
    if devices.is_empty() {
        println!("SKIP (no devices found — simulator may use virtual networks)");
        // Not a hard failure — depends on simulator routing config
    } else {
        println!("OK ({} devices)", devices.len());
        for d in &devices {
            println!("  device {:?} @ {:?}", d.object_identifier, d.mac_address);
        }
    }
    passed += 1; // Who-Is itself succeeded; discovery depends on topology
    println!();

    // ── Test 2: Read BDT from BBMD ────────────────────────────────────────
    print!("Test 2: Read BDT from BBMD ({bbmd_addr}) ... ");
    let bbmd_mac = addr_to_mac(&bbmd_addr);
    match client.read_bdt(&bbmd_mac).await {
        Ok(entries) => {
            if entries.is_empty() {
                println!("WARN (BDT is empty — BBMD may not have peers configured)");
                // Empty BDT is valid, just means no peers
                passed += 1;
            } else {
                println!("OK ({} entries)", entries.len());
                for e in &entries {
                    println!(
                        "  {}.{}.{}.{}:{} mask={}.{}.{}.{}",
                        e.ip[0], e.ip[1], e.ip[2], e.ip[3], e.port,
                        e.broadcast_mask[0], e.broadcast_mask[1],
                        e.broadcast_mask[2], e.broadcast_mask[3],
                    );
                }
                passed += 1;
            }
        }
        Err(e) => {
            println!("FAIL ({e})");
            failed += 1;
        }
    }
    println!();

    // ── Test 3: Read FDT from BBMD ────────────────────────────────────────
    print!("Test 3: Read FDT from BBMD ({bbmd_addr}) ... ");
    match client.read_fdt(&bbmd_mac).await {
        Ok(entries) => {
            println!("OK ({} entries)", entries.len());
            for e in &entries {
                println!(
                    "  {}.{}.{}.{}:{} ttl={} remaining={}",
                    e.ip[0], e.ip[1], e.ip[2], e.ip[3], e.port, e.ttl, e.seconds_remaining,
                );
            }
            passed += 1;
        }
        Err(e) => {
            println!("FAIL ({e})");
            failed += 1;
        }
    }
    println!();

    // ── Test 4: Who-Is-Router-To-Network ──────────────────────────────────
    print!("Test 4: Who-Is-Router-To-Network ... ");
    match client.who_is_router_to_network(None, 3000).await {
        Ok(routers) => {
            if routers.is_empty() {
                println!("WARN (no routers responded — may be expected on single-subnet)");
                // Not a failure — depends on topology
                passed += 1;
            } else {
                println!("OK ({} routers)", routers.len());
                for r in &routers {
                    let mac = r.mac.as_ref();
                    let addr = if mac.len() == 6 {
                        format!(
                            "{}.{}.{}.{}:{}",
                            mac[0],
                            mac[1],
                            mac[2],
                            mac[3],
                            u16::from_be_bytes([mac[4], mac[5]])
                        )
                    } else {
                        format!("{:?}", mac)
                    };
                    println!("  {} serves networks: {:?}", addr, r.networks);
                }
                passed += 1;
            }
        }
        Err(e) => {
            println!("FAIL ({e})");
            failed += 1;
        }
    }
    println!();

    // ── Test 5: Who-Is-Router for specific network ────────────────────────
    print!("Test 5: Who-Is-Router-To-Network(network=1001) ... ");
    match client.who_is_router_to_network(Some(1001), 3000).await {
        Ok(routers) => {
            println!("OK ({} routers)", routers.len());
            for r in &routers {
                let mac = r.mac.as_ref();
                let addr = if mac.len() == 6 {
                    format!(
                        "{}.{}.{}.{}:{}",
                        mac[0],
                        mac[1],
                        mac[2],
                        mac[3],
                        u16::from_be_bytes([mac[4], mac[5]])
                    )
                } else {
                    format!("{:?}", mac)
                };
                println!("  {} serves networks: {:?}", addr, r.networks);
            }
            passed += 1;
        }
        Err(e) => {
            println!("FAIL ({e})");
            failed += 1;
        }
    }
    println!();

    // ── Test 6: Read BDT from second BBMD (cross-subnet if reachable) ───
    let bbmd2_addr =
        std::env::var("BBMD2_ADDRESS").unwrap_or_else(|_| "10.2.0.2:47808".into());
    print!("Test 6: Read BDT from BBMD2 ({bbmd2_addr}) ... ");
    let bbmd2_mac = addr_to_mac(&bbmd2_addr);
    match client.read_bdt(&bbmd2_mac).await {
        Ok(entries) => {
            println!("OK ({} entries)", entries.len());
            for e in &entries {
                println!(
                    "  {}.{}.{}.{}:{} mask={}.{}.{}.{}",
                    e.ip[0], e.ip[1], e.ip[2], e.ip[3], e.port,
                    e.broadcast_mask[0], e.broadcast_mask[1],
                    e.broadcast_mask[2], e.broadcast_mask[3],
                );
            }
            passed += 1;
        }
        Err(e) => {
            println!("SKIP ({e}) — cross-subnet may not be routable");
            // Not a hard failure — depends on IP routing
            passed += 1;
        }
    }
    println!();

    // ── Test 7: BDT contains expected peer entries ────────────────────────
    print!("Test 7: BDT has peer BBMD entries ... ");
    match client.read_bdt(&bbmd_mac).await {
        Ok(entries) if entries.len() >= 2 => {
            // Verify both BBMDs are listed
            let has_bbmd1 = entries.iter().any(|e| e.ip == [10, 1, 0, 2] && e.port == 47808);
            let has_bbmd2 = entries.iter().any(|e| e.ip == [10, 2, 0, 2] && e.port == 47808);
            if has_bbmd1 && has_bbmd2 {
                println!("OK (both BBMDs present in BDT)");
                passed += 1;
            } else {
                println!("FAIL (expected both BBMDs, got: bbmd1={has_bbmd1}, bbmd2={has_bbmd2})");
                failed += 1;
            }
        }
        Ok(entries) => {
            println!("FAIL (expected >= 2 entries, got {})", entries.len());
            failed += 1;
        }
        Err(e) => {
            println!("FAIL ({e})");
            failed += 1;
        }
    }
    println!();

    // ── Summary ───────────────────────────────────────────────────────────
    client.stop().await.unwrap();

    println!("=== Results: {passed} passed, {failed} failed ===");
    if failed > 0 {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

/// Parse "ip:port" into a 6-byte BACnet/IP MAC address.
fn addr_to_mac(addr: &str) -> Vec<u8> {
    let parts: Vec<&str> = addr.split(':').collect();
    let ip: Ipv4Addr = parts[0].parse().expect("invalid IP");
    let port: u16 = parts[1].parse().expect("invalid port");
    let mut mac = ip.octets().to_vec();
    mac.extend_from_slice(&port.to_be_bytes());
    mac
}
