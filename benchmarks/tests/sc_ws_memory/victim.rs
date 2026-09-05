use super::*;
use bacnet_transport::sc::{ScConnection, WebSocketPort};
use bacnet_transport::sc_frame::{decode_sc_message, encode_sc_message};
use bacnet_transport::sc_tls::TlsWebSocket;
use bytes::BytesMut;
use std::io::{BufRead, Write};

pub fn run(role: &str) {
    // Emergency wall-clock bound also unblocks a controller waiting on stdout.
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(30));
        std::process::exit(124);
    });
    let stdin = std::io::stdin();
    let mut input = stdin.lock().lines();
    let setup: Value = serde_json::from_str(&input.next().unwrap().unwrap()).unwrap();
    let certs = CertMaterial {
        ca_cert_pem: setup["ca"].as_str().unwrap().into(),
        server_cert_pem: setup["server_cert"].as_str().unwrap().into(),
        server_key_pem: setup["server_key"].as_str().unwrap().into(),
        client_cert_pem: setup["client_cert"].as_str().unwrap().into(),
        client_key_pem: setup["client_key"].as_str().unwrap().into(),
    };
    let runtime = runtime();
    let mut hub = if role == "hub" {
        Some(runtime.block_on(start_sc_hub_mtls(&certs, [0x10; 6])))
    } else {
        None
    };
    report(json!({"ready":true,"url":hub.as_ref().map(|(_, url)| url)}));
    let mut tasks = Vec::new();
    let mut id = 0u8;
    for command in input {
        let command = command.unwrap();
        if command == "stop" {
            break;
        }
        if command == "dial" {
            id += 1;
            let url = setup["url"].as_str().unwrap().to_owned();
            let config = make_client_tls_config_mtls(&certs);
            tasks.push(runtime.spawn(async move {
                let ws = TlsWebSocket::connect(&url, config).await.unwrap();
                let mut connection = ScConnection::new([0x22, 0, 0, 0, 0, id], [id; 16]);
                let mut wire = BytesMut::new();
                encode_sc_message(&mut wire, &connection.build_connect_request());
                ws.send(&wire).await.unwrap();
                let accept = decode_sc_message(&ws.recv().await.unwrap()).unwrap();
                assert!(connection.handle_connect_accept(&accept));
                while let Ok(data) = ws.recv().await {
                    if data.len() == 4 && data[0] == 0x0a {
                        ws.send(&[0x0b, 0, data[2], data[3]]).await.unwrap();
                    }
                }
            }));
        }
    }
    for task in tasks {
        task.abort();
    }
    if let Some((ref mut hub, _)) = hub {
        runtime.block_on(hub.stop());
    }
}

fn report(value: Value) {
    println!("SC_MEMORY {value}");
    std::io::stdout().flush().unwrap();
}
