//! A device named by its instance is reached only at an address whose
//! device answered as that instance to a question of this client's own
//! (#2824), as a `bacnet_ops` handle is placed. The discovery table is
//! filled by I-Ams nobody asked for, which anyone can send, so a claim in
//! it is asked which device answers there first: one read of its Device
//! object by the wildcard instance, or, refused, of the instance's own
//! (`bacnet_ops::device::identify`). The refusal names each address asked
//! and what answered there.

use bacnet_client::client::BACnetClient;
use bacnet_ops::DeviceAddress;
use bacnet_ops::device::{Answer, identify, placed};
use bacnet_ops::limits::RequestLimits;
use bacnet_transport::port::TransportPort;

use super::Destination;

/// The one of `claims`, the addresses claiming device `instance` (named
/// `name` as the target wrote it), whose device answers as it, while no
/// other may be it ([`placed`]). Each is asked in turn, but a row the
/// application registered by hand, never announced, which is its own word
/// as an address it names is.
pub(super) async fn confirmed<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    name: &str,
    instance: u32,
    claims: Vec<Destination>,
) -> Result<Destination, String> {
    // A one-shot command has the device to itself: no rate cap beyond the
    // client's own.
    let limits = RequestLimits::new(0);
    let mut asked = Vec::with_capacity(claims.len());
    for claim in claims {
        let at = claim.address;
        let row = client.get_device_at(at.network(), at.mac()).await;
        let registered = row.is_some_and(|row| {
            row.last_announced.is_none() && row.object_identifier.instance_number() == instance
        });
        let answer = match registered {
            true => Answer::Is(instance),
            false => identify(client, &limits, &at, instance).await,
        };
        asked.push((at, answer));
    }
    match placed(instance, &asked) {
        Ok(address) => Ok(Destination {
            address,
            instance: Some(instance),
        }),
        Err(_) => Err(refusal(name, instance, &asked)),
    }
}

/// Why device `name` is reached nowhere: each address asked, and what
/// answered there.
fn refusal(name: &str, instance: u32, asked: &[(DeviceAddress, Answer)]) -> String {
    match asked {
        [(at, answer)] => format!(
            "device {name} is claimed at {at}, but asked which device answers there, {}: \
             nothing was sent to it. Name the device by its address to reach whatever \
             answers there",
            said(*answer, instance)
        ),
        _ => {
            let each: Vec<String> = asked
                .iter()
                .map(|(at, answer)| format!("{at} ({})", said(*answer, instance)))
                .collect();
            format!(
                "device {name} is claimed at {}, and not one of them alone answered as \
                 device {instance}; name one by its address",
                each.join(" and ")
            )
        }
    }
}

/// What `answer` says, asked for device `instance`.
fn said(answer: Answer, instance: u32) -> String {
    match answer {
        Answer::Is(found) => format!("it answered as device {found}"),
        Answer::Has(found) => format!("it has device {found}'s Device object"),
        Answer::Not => format!("it has no Device object {instance}"),
        Answer::Unclear => "it turned the question away".into(),
        Answer::Failed => "the question couldn't be sent".into(),
        Answer::Silent => "it didn't answer".into(),
        _ => "its answer named no device".into(),
    }
}
