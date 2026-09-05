use super::*;
use bacnet_transport::sc_frame::{decode_sc_message, encode_sc_message, ScFunction};
use bytes::BytesMut;
use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::{rustls::pki_types::ServerName, TlsAcceptor, TlsConnector, TlsStream};
use tokio_tungstenite::{tungstenite::Message, WebSocketStream};

pub struct Peer {
    pub ws: WebSocketStream<TlsStream<TcpStream>>,
    pub masked: bool,
}

impl Peer {
    pub async fn hub(url: &str, certs: &CertMaterial, id: u8) -> Self {
        let address = url.strip_prefix("wss://").unwrap();
        let tcp = TcpStream::connect(address).await.unwrap();
        let tls = TlsConnector::from(make_client_tls_config_mtls(certs))
            .connect(ServerName::try_from("localhost").unwrap(), tcp)
            .await
            .unwrap();
        let request =
            tokio_tungstenite::tungstenite::client::ClientRequestBuilder::new(url.parse().unwrap())
                .with_sub_protocol("hub.bsc.bacnet.org");
        let (ws, _) = tokio_tungstenite::client_async(request, TlsStream::Client(tls))
            .await
            .unwrap();
        let mut peer = Self { ws, masked: true };
        let mut conn = bacnet_transport::sc::ScConnection::new([0x22, 0, 0, 0, 0, id], [id; 16]);
        conn.max_bvlc_length = 65535;
        conn.max_apdu_length = 61327;
        let mut wire = BytesMut::new();
        encode_sc_message(&mut wire, &conn.build_connect_request());
        peer.ws.send(Message::Binary(wire.freeze())).await.unwrap();
        let accept = decode_sc_message(&peer.binary().await).unwrap();
        assert_eq!(accept.function, ScFunction::ConnectAccept);
        assert!(conn.handle_connect_accept(&accept));
        peer.warmup().await;
        peer
    }

    pub async fn node(listener: &TcpListener, certs: &CertMaterial) -> Self {
        let (tcp, _) = listener.accept().await.unwrap();
        let tls = TlsAcceptor::from(make_server_tls_config_mtls(certs))
            .accept(tcp)
            .await
            .unwrap();
        #[allow(clippy::result_large_err)]
        let ws = tokio_tungstenite::accept_hdr_async(
            TlsStream::Server(tls),
            |_: &_, mut response: tokio_tungstenite::tungstenite::handshake::server::Response| {
                response.headers_mut().insert(
                    "Sec-WebSocket-Protocol",
                    "hub.bsc.bacnet.org".parse().unwrap(),
                );
                Ok(response)
            },
        )
        .await
        .unwrap();
        let mut peer = Self { ws, masked: false };
        let mut request = peer.binary().await;
        assert_eq!(request.len(), 30);
        assert_eq!(request[0], 6);
        request[0] = 7;
        request[4..26].fill(0x10);
        request[26..30].copy_from_slice(&[0x16, 0x49, 0x05, 0xd9]);
        peer.ws.send(Message::Binary(request.into())).await.unwrap();
        peer.warmup().await;
        peer
    }

    async fn binary(&mut self) -> Vec<u8> {
        match tokio::time::timeout(Duration::from_secs(5), self.ws.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap()
        {
            Message::Binary(data) => data.to_vec(),
            _ => panic!("expected BVLC binary message"),
        }
    }

    async fn warmup(&mut self) {
        self.ws
            .send(Message::Binary(vec![0x0a, 0, 0x12, 0x34].into()))
            .await
            .unwrap();
        assert_eq!(self.binary().await, [0x0b, 0, 0x12, 0x34]);
    }

    pub async fn attack(mut self, workload: &str) -> (Self, Value) {
        let mut wire_written = 0usize;
        let mut body_written = 0usize;
        let mut failure = None;
        let started = Instant::now();
        // Only one reusable 4KiB attacker chunk; never allocate the offered 8MiB.
        let chunk: Vec<u8> = (0..4096)
            .map(|i| {
                if self.masked {
                    0x55 ^ [1, 2, 3, 4][i % 4]
                } else {
                    0x55
                }
            })
            .collect();
        if workload == "header" {
            let header = header(2, 8 * 1024 * 1024, self.masked);
            failure = write_counted(self.ws.get_mut(), &header, &mut wire_written).await;
            let mut remaining = 8 * 1024 * 1024 - 1;
            while failure.is_none() && remaining > 0 {
                let count = remaining.min(chunk.len());
                let mut written = 0;
                failure = write_counted(self.ws.get_mut(), &chunk[..count], &mut written).await;
                wire_written += written;
                body_written += written;
                remaining -= written;
            }
        } else {
            for frame in 0..2048 {
                // Individually valid frames, no FIN: reject cumulative growth.
                let header = header(if frame == 0 { 2 } else { 0 }, 4096, self.masked);
                failure = write_counted(self.ws.get_mut(), &header, &mut wire_written).await;
                if failure.is_some() {
                    break;
                }
                let mut written = 0;
                failure = write_counted(self.ws.get_mut(), &chunk, &mut written).await;
                wire_written += written;
                body_written += written;
                if failure.is_some() {
                    break;
                }
            }
        }
        if failure.is_none() {
            failure = match tokio::time::timeout(Duration::from_secs(5), self.ws.get_mut().flush())
                .await
            {
                Ok(Ok(())) => None,
                Ok(Err(error)) => Some(write_error(error)),
                Err(_) => Some(json!({"status":"timeout","operation":"flush"})),
            };
        }
        let incomplete = failure
            .as_ref()
            .is_some_and(|f| f["status"] != "terminal_io_error");
        (
            self,
            json!({"wire_bytes_written":wire_written,"body_bytes_written":body_written,"write_failure":failure,"write_incomplete":incomplete,"write_duration_ms":started.elapsed().as_millis()}),
        )
    }

    pub async fn rejection_evidence(&mut self) -> Value {
        let mut byte = [0; 1];
        match tokio::time::timeout(
            Duration::from_millis(100),
            self.ws.get_mut().read(&mut byte),
        )
        .await
        {
            Ok(Ok(0)) => json!({"status":"eof","rejected":true}),
            Ok(Err(error)) => {
                json!({"status":"read_error","kind":format!("{:?}", error.kind()),"rejected":true})
            }
            Ok(Ok(_)) => json!({"status":"unexpected_output","rejected":false}),
            Err(_) => json!({"status":"timeout","rejected":false}),
        }
    }
}

fn header(opcode: u8, size: usize, masked: bool) -> Vec<u8> {
    let mask = if masked { 0x80 } else { 0 };
    let mut out = vec![opcode]; // FIN remains unset for both workloads.
    if size <= 65535 {
        out.push(mask | 126);
        out.extend_from_slice(&(size as u16).to_be_bytes());
    } else {
        out.push(mask | 127);
        out.extend_from_slice(&(size as u64).to_be_bytes());
    }
    if masked {
        out.extend_from_slice(&[1, 2, 3, 4]);
    }
    out
}

fn write_error(error: std::io::Error) -> Value {
    use std::io::ErrorKind;
    let terminal = matches!(
        error.kind(),
        ErrorKind::BrokenPipe
            | ErrorKind::ConnectionReset
            | ErrorKind::ConnectionAborted
            | ErrorKind::NotConnected
            | ErrorKind::UnexpectedEof
    );
    json!({"status":if terminal {"terminal_io_error"} else {"other_io_error"},"kind":format!("{:?}", error.kind())})
}

async fn write_counted(
    stream: &mut TlsStream<TcpStream>,
    bytes: &[u8],
    written: &mut usize,
) -> Option<Value> {
    let mut offset = 0;
    while offset < bytes.len() {
        match tokio::time::timeout(Duration::from_secs(5), stream.write(&bytes[offset..])).await {
            Ok(Ok(count)) if count > 0 => {
                offset += count;
                *written += count;
            }
            Ok(Ok(_)) => return Some(json!({"status":"zero_write"})),
            Ok(Err(error)) => return Some(write_error(error)),
            Err(_) => return Some(json!({"status":"timeout","operation":"write"})),
        }
    }
    None
}
