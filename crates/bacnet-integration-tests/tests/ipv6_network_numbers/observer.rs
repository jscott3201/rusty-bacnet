//! Test-only client for the independently isolated raw multicast observer.
use super::{NODE, bounded, wire};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::TcpStream,
};

pub(super) struct Observer {
    pub index: u32,
    input: BufReader<TcpStream>,
}

impl Observer {
    pub async fn subscribe(port: u16) -> Self {
        let address: std::net::Ipv6Addr = std::env::var("RB_IPV6_OBSERVER_ADDRESS")
            .expect("normal wire qualification requires a separate isolated observer")
            .parse()
            .unwrap();
        assert!(address.is_unique_local());
        let mut tcp = bounded(TcpStream::connect((address, 47809))).await.unwrap();
        bounded(tcp.write_all(format!("{{\"port\":{port}}}\n").as_bytes()))
            .await
            .unwrap();
        let mut input = BufReader::new(tcp);
        let mut ready = String::new();
        bounded(input.read_line(&mut ready)).await.unwrap();
        let metadata: serde_json::Value = serde_json::from_str(&ready).unwrap();
        let index = metadata["index"].as_u64().unwrap().try_into().unwrap();
        Self { index, input }
    }

    pub async fn number(&mut self) -> wire::Frame {
        loop {
            let mut line = String::new();
            assert_ne!(self.input.read_line(&mut line).await.unwrap(), 0);
            let data: serde_json::Value = serde_json::from_str(&line).unwrap();
            let bytes: Vec<u8> = serde_json::from_value(data["bytes"].clone()).unwrap();
            if bytes.len() >= 7 && bytes[4..7] == NODE {
                return wire::Frame {
                    bytes,
                    source: data["source"].as_str().unwrap().parse().unwrap(),
                    destination: data["destination"].as_str().unwrap().parse().unwrap(),
                    index: data["index"].as_u64().unwrap().try_into().unwrap(),
                };
            }
        }
    }
}
