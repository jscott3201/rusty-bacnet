//! Device management in the write worker (#1799): DeviceCommunicationControl
//! and ReinitializeDevice, checked here again whatever the app decided,
//! sent once, and recorded with their outcome. The audit record names the
//! service, the device, the state and duration, and whether a password went
//! with it, never the password itself; the request, password and all, is
//! dropped once it has been sent.

use bacnet_client::client::BACnetClient;
use bacnet_ops::class::Class;
use bacnet_ops::limits::RequestLimits;
use bacnet_ops::safety::desk::{Caller, ENDED};
use bacnet_ops::safety::find;
use bacnet_transport::port::TransportPort;

use super::{PASSIVE, Runner, tell};
use crate::tui::safety::manage::{Action, NEEDS_DANGEROUS, Request};
use crate::tui::safety::{Outcome, SafetyEvent};
use crate::tui::worker::EventSink;

impl Runner<'_> {
    /// Why a device management request may not be sent now, if it may not.
    fn manage_refusal(&self, sink: &EventSink) -> Option<String> {
        if self.options.passive {
            Some(PASSIVE.into())
        } else if let Some(why) = self.desk.missing() {
            Some(why.to_string())
        } else if !self.options.policy.dangerous {
            Some(NEEDS_DANGEROUS.into())
        } else if !self.caller(sink).level().permits(Class::DeviceManagement) {
            Some("read-only: press W to arm a write session".into())
        } else {
            None
        }
    }

    /// Send a confirmed request, record it, and report its outcome.
    pub(super) async fn manage<T: TransportPort + 'static>(
        &mut self,
        client: &BACnetClient<T>,
        sink: &EventSink,
        rate: &RequestLimits,
        req: u64,
        request: Request,
    ) {
        let event = request.action.command();
        if let Some(why) = self.manage_refusal(sink) {
            let fields = request.audit_json();
            return self.refuse(sink, Some(req), event, why, fields).await;
        }
        let outcome = if sink.is_cancelled() {
            Outcome::NotSent(ENDED.into())
        } else {
            send(client, rate, &request).await
        };
        let label = request.label();
        let mut fields = request.audit_json();
        drop(request);
        fields["outcome"] = outcome.json();
        self.desk.record(&self.caller(sink), event, fields).await;
        let outcomes = vec![(label, outcome)];
        tell(sink, SafetyEvent::Done { req, outcomes }).await;
    }
}

/// One request on the wire, in the device's turn: its password goes out
/// as the service's CharacterString, the one place it is ever written.
async fn send<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    rate: &RequestLimits,
    request: &Request,
) -> Outcome {
    let device = request.device;
    find(client, device).await;
    let turn = rate.turn(device).await;
    let password = request.password.as_ref().map(|p| p.expose().to_owned());
    let result = match request.action {
        Action::Dcc { state, time } => {
            client
                .device_communication_control_from_device(device, state, time.minutes(), password)
                .await
        }
        Action::Reinit { state } => {
            client
                .reinitialize_device_from_device(device, state, password)
                .await
        }
    };
    drop(turn);
    Outcome::of(&result)
}
