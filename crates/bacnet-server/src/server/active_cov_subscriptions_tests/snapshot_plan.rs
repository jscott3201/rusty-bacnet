//! A read samples the COV table only when its plan reads one of the
//! Device's COV lists, and then once (#1213). Every read is planned before a
//! value is read, so a selector or a Group member that expands to neither
//! list samples nothing, and neither does a read past its work limit.
use super::group_present_value::{abort, add_group, group, read_property, read_range};
use super::*;

const ALL: PropertyIdentifier = PropertyIdentifier::ALL;
const REQUIRED: PropertyIdentifier = PropertyIdentifier::REQUIRED;
const OPTIONAL: PropertyIdentifier = PropertyIdentifier::OPTIONAL;

/// How many times the server has sampled the live lists so far.
async fn samples(wire: &Wire) -> usize {
    wire.server.cov_table.read().await.live_samples()
}

/// Group 1's member names Active_COV_Subscriptions on the Device. Group 2's
/// member reads the Device's REQUIRED properties, and the Device's COV lists
/// are optional ones, so Group 2 reads neither list.
async fn add_groups(wire: &Wire) {
    add_group(wire, 1, &[(device(), &[ACTIVE])]).await;
    add_group(wire, 2, &[(device(), &[REQUIRED]), (av(1), &[ALL])]).await;
}

#[tokio::test]
async fn reads_whose_plan_names_no_device_cov_list_take_no_snapshot() {
    let mut wire = Wire::start(ServerConfig::default()).await;
    add_groups(&wire).await;
    let before = samples(&wire).await;
    // Present_Value is required on a Group, so OPTIONAL plans no member.
    for specs in [
        vec![(device(), vec![(REQUIRED, None)])],
        vec![(group(1), vec![(OPTIONAL, None)])],
        vec![(group(2), vec![(PV, None), (ALL, None)])],
    ] {
        wire.rpm(specs.clone()).await;
        assert_eq!(samples(&wire).await, before, "{specs:?}");
    }
    for request in [read_property(group(2)), read_range(group(2))] {
        assert!(matches!(
            wire.send(&direct(), request).await,
            Apdu::ComplexAck(_)
        ));
    }
    wire.server.read_local(&group(2), PV, None).await.unwrap();
    assert_eq!(samples(&wire).await, before);
    wire.server.stop().await.unwrap();
}

#[tokio::test]
async fn a_read_whose_plan_names_a_device_cov_list_takes_one_snapshot() {
    let mut wire = Wire::start(ServerConfig::default()).await;
    add_groups(&wire).await;
    let mut expected = samples(&wire).await;
    // Both lists, by name, through ALL and through the wildcard, and through
    // a Group's members: still one sample for the request.
    wire.rpm(vec![
        (device(), vec![(ACTIVE, None), (ALL, None)]),
        (wildcard(), vec![(MULTIPLE, None)]),
        (group(1), vec![(PV, None), (REQUIRED, None)]),
        (group(2), vec![(PV, None)]),
    ])
    .await;
    expected += 1;
    assert_eq!(samples(&wire).await, expected);
    wire.read(device(), ACTIVE, None).await.unwrap();
    for request in [read_property(group(1)), read_range(group(1))] {
        assert!(matches!(
            wire.send(&direct(), request).await,
            Apdu::ComplexAck(_)
        ));
    }
    wire.server.read_local(&group(1), PV, None).await.unwrap();
    expected += 4;
    assert_eq!(samples(&wire).await, expected);
    wire.server.stop().await.unwrap();
}

#[tokio::test]
async fn a_read_past_its_work_limit_aborts_before_sampling_the_cov_table() {
    let mut wire = Wire::start(ServerConfig {
        read_property_multiple_budget: ReadPropertyMultipleBudget {
            max_result_elements: 2,
            ..Default::default()
        },
        ..Default::default()
    })
    .await;
    // Group 3's Present_Value is three rows: its own and two members.
    add_group(&wire, 3, &[(device(), &[ACTIVE, MULTIPLE])]).await;
    let before = samples(&wire).await;
    for request in [
        rpm_request(vec![(
            device(),
            vec![(ACTIVE, None), (MULTIPLE, None), (ACTIVE, None)],
        )]),
        rpm_request(vec![(group(3), vec![(PV, None)])]),
        read_property(group(3)),
        read_range(group(3)),
    ] {
        assert_eq!(
            abort(wire.send(&direct(), request).await),
            (true, AbortReason::OUT_OF_RESOURCES)
        );
    }
    assert!(matches!(
        wire.server.read_local(&group(3), PV, None).await,
        Err(Error::Abort { reason }) if reason == AbortReason::OUT_OF_RESOURCES
    ));
    assert_eq!(samples(&wire).await, before);
    // Within the limit the same lists take their one sample.
    wire.rpm(vec![(device(), vec![(ACTIVE, None), (MULTIPLE, None)])])
        .await;
    assert_eq!(samples(&wire).await, before + 1);
    wire.server.stop().await.unwrap();
}
