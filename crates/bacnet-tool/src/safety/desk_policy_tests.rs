//! The tool's writes beside the library's write policy (#2560): the desk
//! sends through the low-level client under the tool's own tiers, so a
//! write the person confirmed at a priority the library guards still goes,
//! while a `bacnet-ops` handle with the default policy refuses the same
//! write and sends nothing.

use std::sync::Arc;

use bacnet_ops::write::WritePolicy;
use bacnet_ops::{Client, OpsError, Priority};

use super::*;

/// The priority of each WriteProperty sent.
fn write_priorities(sent: &SendLog) -> Vec<Option<u8>> {
    sent.unicasts()
        .iter()
        .filter_map(|frame| match frame.apdu() {
            Apdu::ConfirmedRequest(request)
                if request.service_choice == ConfirmedServiceChoice::WRITE_PROPERTY =>
            {
                let write = WritePropertyRequest::decode(&request.service_request).unwrap();
                Some(write.priority)
            }
            _ => None,
        })
        .collect()
}

/// Priority 1 is T3 at the desk; confirmed, it is sent at 1. The library's
/// default policy refuses the same write, sending nothing, until allowed.
#[tokio::test(start_paused = true)]
async fn a_confirmed_tool_write_at_a_guarded_priority_goes_and_the_library_refuses_it() {
    let lab = lab().await;
    lab.client.add_device(DEVICE, &A).await.unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut desk = Desk::open(Some(dir.path()), context(), Origin::default());
    let life_safety = WriteSpec {
        priority: Some(1),
        ..spec(at(&A))
    };
    let preview = desk
        .prepare(&lab.client, &lab.rate, &Armed, &[life_safety], false, false)
        .await
        .unwrap();
    assert_eq!(preview.gate, Gate::Allowed(Tier::T3));
    let done = desk.commit(&lab.client, &lab.rate, &Armed, &preview).await;
    assert_eq!(done[0].outcome, Outcome::Ack);
    assert_eq!(write_priorities(&lab.sent), [Some(1)]);

    let Lab {
        client,
        sent,
        inbox: _inbox,
        ..
    } = lab;
    let ops = Client::new(Arc::new(client), Arc::new(RequestLimits::new(0)));
    let target = (ao1(), PropertyIdentifier::PRESENT_VALUE);
    let one = Priority::MANUAL_LIFE_SAFETY;
    let device = ops.device(at(&A)).await.unwrap();
    let refused = device.write(target, "5", Some(one)).await.unwrap_err();
    assert!(matches!(refused, OpsError::Denied(_)), "{refused:?}");
    assert_eq!(write_priorities(&sent), [Some(1)], "nothing more sent");
    let allowed = ops.with_write_policy(WritePolicy::default().allow_priorities([one]));
    let device = allowed.device(at(&A)).await.unwrap();
    device.write(target, "5", Some(one)).await.unwrap();
    assert_eq!(write_priorities(&sent), [Some(1), Some(1)]);
}
