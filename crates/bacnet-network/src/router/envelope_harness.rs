//! Shared in-memory harness for the RB-03 router envelope test modules.
//!
//! No sockets, no timing. [`Harness::handle`] exercises the local-control
//! admission point; [`Harness::dispatch`] exercises the routing-first
//! dispatcher. Both test modules build ingress contexts here so discovery
//! and control-envelope coverage stays in lockstep.

use super::*;

pub(super) fn control_npdu(message_type: NetworkMessageType, payload: &[u8]) -> Npdu {
    Npdu {
        is_network_message: true,
        message_type: Some(message_type.to_raw()),
        payload: Bytes::copy_from_slice(payload),
        ..Npdu::default()
    }
}

pub(super) fn attributes() -> Vec<DataAttribute> {
    vec![DataAttribute {
        option_type: 31,
        must_understand: false,
        data: vec![0x12, 0x34],
    }]
}

pub(super) struct Harness {
    pub(super) table: Arc<Mutex<RouterTable>>,
    pub(super) txs: Vec<mpsc::Sender<SendRequest>>,
    pub(super) rxs: Vec<mpsc::Receiver<SendRequest>>,
}

impl Harness {
    pub(super) fn two_port() -> Self {
        let mut table = RouterTable::new();
        table.add_direct(1000, 0);
        table.add_direct(2000, 1);
        Self::with_table(table)
    }

    pub(super) fn with_table(table: RouterTable) -> Self {
        let (tx0, rx0) = mpsc::channel(16);
        let (tx1, rx1) = mpsc::channel(16);
        Self {
            table: Arc::new(Mutex::new(table)),
            txs: vec![tx0, tx1],
            rxs: vec![rx0, rx1],
        }
    }

    pub(super) fn ctx(&self, port: usize, source_mac: &[u8], npdu: Npdu) -> IngressContext {
        let _ = &self;
        IngressContext {
            port_idx: port,
            port_network: if port == 0 { 1000 } else { 2000 },
            source_mac: MacAddr::from_slice(source_mac),
            link_layer_group: true,
            data_attributes: Vec::new(),
            npdu,
        }
    }

    pub(super) async fn handle(&mut self, ctx: IngressContext) {
        handle_network_message(&self.table, &self.txs, &ctx).await;
    }

    pub(super) async fn dispatch(&mut self, ctx: IngressContext) {
        dispatch_network_message(&self.table, &self.txs, &ctx).await;
    }

    pub(super) fn drain(&mut self, port: usize) -> Vec<SendRequest> {
        let mut out = Vec::new();
        while let Ok(req) = self.rxs[port].try_recv() {
            out.push(req);
        }
        out
    }

    pub(super) fn assert_quiet(&mut self) {
        for port in 0..self.rxs.len() {
            assert!(
                self.rxs[port].try_recv().is_err(),
                "expected no output on port {port}"
            );
        }
    }
}

pub(super) fn broadcast_data(requests: Vec<SendRequest>) -> Vec<Bytes> {
    requests
        .into_iter()
        .map(|req| match req {
            SendRequest::Broadcast { npdu, .. } => npdu,
            SendRequest::Unicast { .. } => panic!("expected broadcast"),
        })
        .collect()
}
