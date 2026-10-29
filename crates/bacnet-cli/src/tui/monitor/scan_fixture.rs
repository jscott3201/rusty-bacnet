//! Reopenable scripted input. Baud-dependent bytes reach the real decoder.
//! This models requested rates and ownership, not native configuration/purge.
use super::lines::MonitorReader;
use bacnet_transport::mstp::codec::FrameType;
use bacnet_transport::mstp::monitor::script::{Damage, LineScript, ScriptedRing};
use bacnet_transport::mstp::monitor::{MonitorAdmissionError, MonitorRead};
use bacnet_types::error::Error;
use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::pin::Pin;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, DuplexStream, ReadBuf};
use tokio::sync::{Notify, oneshot};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Step {
    Open(usize, u32),
    Drained(usize),
    Closing(usize),
    Closed(usize),
    Failed(usize),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Fault {
    Admission,
    Read,
    Eof,
    Close,
    Hold,
}
#[derive(Default)]
struct State {
    steps: Vec<Step>,
    held: Option<usize>,
    faults: BTreeMap<usize, Fault>,
    peers: BTreeMap<usize, DuplexStream>,
    releases: BTreeMap<usize, oneshot::Sender<bool>>,
}
pub(crate) struct ScanLine {
    rates: BTreeSet<u32>,
    state: Arc<Mutex<State>>,
    pub(crate) changed: Arc<Notify>,
    pub(crate) participants: AtomicUsize,
}
impl ScanLine {
    pub(crate) fn new(rates: &[u32]) -> Arc<Self> {
        Arc::new(Self {
            rates: rates.iter().copied().collect(),
            state: Default::default(),
            changed: Arc::new(Notify::new()),
            participants: AtomicUsize::new(0),
        })
    }
    pub(crate) async fn feed_valid(&self) {
        use tokio::io::AsyncWriteExt;
        let (index, mut peer) = {
            let mut state = self.state.lock().unwrap();
            let index = state.held.unwrap();
            (index, state.peers.remove(&index).unwrap())
        };
        peer.write_all(&traffic(19200, true)).await.unwrap();
        self.return_peer(index, peer);
    }
    pub(crate) fn fault(&self, index: usize, fault: Fault) {
        self.state.lock().unwrap().faults.insert(index, fault);
    }
    pub(crate) fn steps(&self) -> Vec<Step> {
        self.state.lock().unwrap().steps.clone()
    }
    pub(crate) fn held(&self) -> Option<usize> {
        self.state.lock().unwrap().held
    }
    pub(crate) fn release(&self, index: usize, success: bool) {
        self.state
            .lock()
            .unwrap()
            .releases
            .remove(&index)
            .unwrap()
            .send(success)
            .unwrap();
    }
    pub(crate) fn return_peer(&self, index: usize, peer: DuplexStream) {
        self.state.lock().unwrap().peers.insert(index, peer);
    }
    pub(crate) fn take_peers(&self) -> Vec<(usize, DuplexStream)> {
        std::mem::take(&mut self.state.lock().unwrap().peers)
            .into_iter()
            .collect()
    }
    pub(crate) fn monitor(&self, baud: u32) -> Result<MonitorReader, MonitorAdmissionError> {
        let mut state = self.state.lock().unwrap();
        assert!(
            state.held.is_none(),
            "another input still owns the line: {:?}",
            state.steps
        );
        let index = state
            .steps
            .iter()
            .filter(|step| matches!(step, Step::Open(..)))
            .count();
        state.steps.push(Step::Open(index, baud));
        let fault = state.faults.get(&index).copied();
        self.changed.notify_waiters();
        if fault == Some(Fault::Admission) {
            return Err(MonitorAdmissionError::without_input(Error::Transport(
                io::Error::other("injected open/configuration/purge rejection before ownership"),
            )));
        }
        let (peer, stream) = tokio::io::duplex(4096);
        state.peers.insert(index, peer);
        state.held = Some(index);
        let release = if fault == Some(Fault::Hold) {
            let (tx, rx) = oneshot::channel();
            state.releases.insert(index, tx);
            Some(rx)
        } else {
            None
        };
        Ok(Box::new(Reader {
            stream: Some(stream),
            bytes: traffic(baud, self.rates.contains(&baud)),
            at: 0,
            drained: false,
            closing: false,
            result: None,
            state: self.state.clone(),
            changed: self.changed.clone(),
            index,
            fault,
            release,
        }))
    }
    pub(crate) fn participant(&self) -> Result<super::lines::CliSerial, Error> {
        self.participants.fetch_add(1, Ordering::SeqCst);
        Err(Error::Encoding(
            "scanner fixture rejects participant construction".into(),
        ))
    }
}
fn traffic(baud: u32, matched: bool) -> Vec<u8> {
    let mut ring = ScriptedRing::new(baud).master(1, 10).master(2, 10);
    if matched {
        ring.rotate(3, |line: &mut LineScript, source| {
            line.frame(
                FrameType::BACNET_DATA_NOT_EXPECTING_REPLY,
                255,
                source,
                &[1, 0, 0x10, 8],
            );
        });
        ring.line
            .chunks()
            .iter()
            .flat_map(|(_, bytes)| bytes.iter().copied())
            .collect()
    } else {
        let mut line = LineScript::new(baud);
        for _ in 0..3 {
            line.damaged(
                FrameType::BACNET_DATA_NOT_EXPECTING_REPLY,
                255,
                1,
                &[1, 0, 0x10, 8],
                Damage::HeaderCrc,
            );
        }
        let mut bytes = vec![0x31; 12];
        bytes.extend(
            line.chunks()
                .iter()
                .flat_map(|(_, bytes)| bytes.iter().copied()),
        );
        bytes
    }
}
struct Reader {
    stream: Option<DuplexStream>,
    bytes: Vec<u8>,
    at: usize,
    drained: bool,
    closing: bool,
    result: Option<bool>,
    state: Arc<Mutex<State>>,
    changed: Arc<Notify>,
    index: usize,
    fault: Option<Fault>,
    release: Option<oneshot::Receiver<bool>>,
}
impl AsyncRead for Reader {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if self.fault == Some(Fault::Read) {
            return Poll::Ready(Err(io::Error::other("injected scan read failure")));
        }
        if self.fault == Some(Fault::Eof) {
            return Poll::Ready(Ok(()));
        }
        if self.at < self.bytes.len() {
            let end = (self.at + buf.remaining()).min(self.bytes.len());
            buf.put_slice(&self.bytes[self.at..end]);
            self.at = end;
            return Poll::Ready(Ok(()));
        }
        if !self.drained {
            self.drained = true;
            self.state
                .lock()
                .unwrap()
                .steps
                .push(Step::Drained(self.index));
            self.changed.notify_waiters();
        }
        let before = buf.filled().len();
        let result = Pin::new(self.stream.as_mut().unwrap()).poll_read(cx, buf);
        if matches!(result, Poll::Ready(Ok(()))) && buf.filled().len() > before {
            // The next read is a causal receipt after these bytes were decoded.
            self.drained = false;
        }
        result
    }
}
impl MonitorRead for Reader {
    fn poll_close(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if !self.closing {
            self.closing = true;
            self.state
                .lock()
                .unwrap()
                .steps
                .push(Step::Closing(self.index));
            self.changed.notify_waiters();
        }
        if self.result.is_none() {
            let success = match self.release.as_mut() {
                Some(rx) => std::task::ready!(Pin::new(rx).poll(cx)).unwrap(),
                None => self.fault != Some(Fault::Close),
            };
            self.result = Some(success);
            let mut state = self.state.lock().unwrap();
            if success {
                self.stream = None;
                state.held = None;
                state.steps.push(Step::Closed(self.index));
            } else {
                state.steps.push(Step::Failed(self.index));
            }
            self.changed.notify_waiters();
        }
        if self.result == Some(true) {
            Poll::Ready(Ok(()))
        } else {
            Poll::Ready(Err(io::Error::other("injected scan release failure")))
        }
    }
}
