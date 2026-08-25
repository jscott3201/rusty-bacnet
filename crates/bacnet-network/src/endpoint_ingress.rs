use bacnet_encoding::apdu::{decode_apdu, Apdu};
use bacnet_transport::port::TransportPort;
use bacnet_types::error::Error;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

use crate::layer::{NetworkLayer, ReceivedApdu};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IngressRoute {
    InboundRequest,
    TerminalOrSegment,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PolicyReason {
    MalformedApdu,
    UnsupportedPduType(u8),
    RouteFull(IngressRoute),
    RouteClosed(IngressRoute),
}

#[derive(Debug)]
pub(crate) struct PolicyOutcome {
    pub(crate) reason: PolicyReason,
    pub(crate) received: ReceivedApdu,
}

pub(crate) struct IngressReceivers {
    pub(crate) inbound_requests: mpsc::Receiver<ReceivedApdu>,
    pub(crate) terminal_or_segment: mpsc::Receiver<ReceivedApdu>,
    pub(crate) policy_outcomes: mpsc::Receiver<PolicyOutcome>,
}

#[derive(Debug)]
pub(crate) enum ClassifierExit {
    Cancelled,
    InputClosed,
    PolicyRouteFull(PolicyOutcome),
    PolicyRouteClosed(PolicyOutcome),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Lifecycle {
    Ready,
    Running,
    Stopped,
}

pub(crate) struct EndpointIngress<T: TransportPort> {
    network: NetworkLayer<T>,
    queue_capacity: usize,
    lifecycle: Lifecycle,
    cancel_tx: Option<oneshot::Sender<()>>,
    classifier_task: Option<JoinHandle<ClassifierExit>>,
}

impl<T: TransportPort + 'static> EndpointIngress<T> {
    pub(crate) fn new(transport: T, queue_capacity: usize) -> Self {
        Self {
            network: NetworkLayer::new(transport),
            queue_capacity,
            lifecycle: Lifecycle::Ready,
            cancel_tx: None,
            classifier_task: None,
        }
    }

    pub(crate) async fn start(&mut self) -> Result<IngressReceivers, Error> {
        if self.lifecycle != Lifecycle::Ready {
            return Err(Error::Encoding(
                "endpoint ingress cannot be started more than once".into(),
            ));
        }
        if self.queue_capacity == 0 {
            return Err(Error::Encoding(
                "endpoint ingress queue capacity must be greater than zero".into(),
            ));
        }

        let apdu_rx = self.network.start().await?;
        let (inbound_tx, inbound_requests) = mpsc::channel(self.queue_capacity);
        let (terminal_tx, terminal_or_segment) = mpsc::channel(self.queue_capacity);
        let (policy_tx, policy_outcomes) = mpsc::channel(self.queue_capacity);
        let (cancel_tx, cancel_rx) = oneshot::channel();

        self.classifier_task = Some(tokio::spawn(classifier_task(
            apdu_rx,
            inbound_tx,
            terminal_tx,
            policy_tx,
            cancel_rx,
        )));
        self.cancel_tx = Some(cancel_tx);
        self.lifecycle = Lifecycle::Running;

        Ok(IngressReceivers {
            inbound_requests,
            terminal_or_segment,
            policy_outcomes,
        })
    }

    pub(crate) async fn stop(&mut self) -> Result<ClassifierExit, Error> {
        if self.lifecycle != Lifecycle::Running {
            return Err(Error::Encoding("endpoint ingress is not running".into()));
        }

        self.lifecycle = Lifecycle::Stopped;
        if let Some(cancel_tx) = self.cancel_tx.take() {
            let _ = cancel_tx.send(());
        }

        let classifier_result = match self.classifier_task.take() {
            Some(task) => task.await.map_err(|error| {
                Error::Encoding(format!("endpoint ingress classifier failed: {error}"))
            }),
            None => Err(Error::Encoding(
                "endpoint ingress classifier task is missing".into(),
            )),
        };
        let stop_result = self.network.stop().await;

        stop_result?;
        classifier_result
    }
}

impl<T: TransportPort> Drop for EndpointIngress<T> {
    fn drop(&mut self) {
        if let Some(cancel_tx) = self.cancel_tx.take() {
            let _ = cancel_tx.send(());
        }
        if let Some(task) = self.classifier_task.take() {
            task.abort();
        }
    }
}

async fn classifier_task(
    mut apdu_rx: mpsc::Receiver<ReceivedApdu>,
    inbound_tx: mpsc::Sender<ReceivedApdu>,
    terminal_tx: mpsc::Sender<ReceivedApdu>,
    policy_tx: mpsc::Sender<PolicyOutcome>,
    mut cancel_rx: oneshot::Receiver<()>,
) -> ClassifierExit {
    loop {
        let received = tokio::select! {
            biased;
            _ = &mut cancel_rx => return ClassifierExit::Cancelled,
            received = apdu_rx.recv() => match received {
                Some(received) => received,
                None => return ClassifierExit::InputClosed,
            },
        };

        let route = match classify(&received) {
            Ok(route) => route,
            Err(reason) => {
                let outcome = PolicyOutcome { reason, received };
                if let Some(exit) = send_policy(&policy_tx, outcome) {
                    return exit;
                }
                continue;
            }
        };

        let send_result = match route {
            IngressRoute::InboundRequest => inbound_tx.try_send(received),
            IngressRoute::TerminalOrSegment => terminal_tx.try_send(received),
        };
        if let Err(error) = send_result {
            let (reason, received) = match error {
                mpsc::error::TrySendError::Full(received) => {
                    (PolicyReason::RouteFull(route), received)
                }
                mpsc::error::TrySendError::Closed(received) => {
                    (PolicyReason::RouteClosed(route), received)
                }
            };
            if let Some(exit) = send_policy(&policy_tx, PolicyOutcome { reason, received }) {
                return exit;
            }
        }
    }
}

fn classify(received: &ReceivedApdu) -> Result<IngressRoute, PolicyReason> {
    let Some(first) = received.apdu.first() else {
        return Err(PolicyReason::MalformedApdu);
    };
    let pdu_type = (first >> 4) & 0x0f;
    if pdu_type > 7 {
        return Err(PolicyReason::UnsupportedPduType(pdu_type));
    }

    match decode_apdu(received.apdu.clone()) {
        Ok(Apdu::ConfirmedRequest(_) | Apdu::UnconfirmedRequest(_)) => {
            Ok(IngressRoute::InboundRequest)
        }
        Ok(
            Apdu::SimpleAck(_)
            | Apdu::ComplexAck(_)
            | Apdu::Error(_)
            | Apdu::Reject(_)
            | Apdu::Abort(_)
            | Apdu::SegmentAck(_),
        ) => Ok(IngressRoute::TerminalOrSegment),
        Err(_) => Err(PolicyReason::MalformedApdu),
    }
}

fn send_policy(
    policy_tx: &mpsc::Sender<PolicyOutcome>,
    outcome: PolicyOutcome,
) -> Option<ClassifierExit> {
    match policy_tx.try_send(outcome) {
        Ok(()) => None,
        Err(mpsc::error::TrySendError::Full(outcome)) => {
            Some(ClassifierExit::PolicyRouteFull(outcome))
        }
        Err(mpsc::error::TrySendError::Closed(outcome)) => {
            Some(ClassifierExit::PolicyRouteClosed(outcome))
        }
    }
}

#[cfg(test)]
#[path = "endpoint_ingress_tests.rs"]
mod tests;
