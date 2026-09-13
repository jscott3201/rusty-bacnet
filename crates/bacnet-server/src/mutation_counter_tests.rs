use super::*;

const SERVICES: [ConfirmedServiceChoice; 10] = [
    ConfirmedServiceChoice::WRITE_PROPERTY,
    ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
    ConfirmedServiceChoice::CREATE_OBJECT,
    ConfirmedServiceChoice::DELETE_OBJECT,
    ConfirmedServiceChoice::ADD_LIST_ELEMENT,
    ConfirmedServiceChoice::REMOVE_LIST_ELEMENT,
    ConfirmedServiceChoice::ATOMIC_WRITE_FILE,
    ConfirmedServiceChoice::SUBSCRIBE_COV,
    ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY,
    ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY_MULTIPLE,
];

fn rows(c: MutationDecisionCounters) -> [MutationServiceCounters; 10] {
    [
        c.write_property,
        c.write_property_multiple,
        c.create_object,
        c.delete_object,
        c.add_list_element,
        c.remove_list_element,
        c.atomic_write_file,
        c.subscribe_cov,
        c.subscribe_cov_property,
        c.subscribe_cov_property_multiple,
    ]
}

#[test]
fn mutation_counters_fixed_shape_exactness_and_saturation() {
    let store = MutationDecisions::default();
    assert_eq!(
        rows(store.snapshot()),
        [MutationServiceCounters::default(); 10]
    );
    for (index, service) in SERVICES.into_iter().enumerate() {
        for _ in 0..=index {
            store.record(service, MutationDecision::Allow);
            store.record(service, MutationDecision::Deny);
            store.record(service, MutationDecision::PolicyDeny);
        }
    }
    for (index, row) in rows(store.snapshot()).into_iter().enumerate() {
        let n = index as u64 + 1;
        assert_eq!(
            row,
            MutationServiceCounters {
                allow_total: n,
                deny_total: 2 * n,
                policy_deny_total: n,
            }
        );
    }
    let before = store.snapshot();
    store.record(
        ConfirmedServiceChoice::READ_PROPERTY,
        MutationDecision::PolicyDeny,
    );
    assert_eq!(store.snapshot(), before);
    for row in &store.0 {
        for counter in row {
            counter.store(u64::MAX - 1, Ordering::Relaxed);
        }
    }
    for _ in 0..3 {
        for service in SERVICES {
            store.record(service, MutationDecision::Allow);
            store.record(service, MutationDecision::Deny);
            store.record(service, MutationDecision::PolicyDeny);
        }
    }
    assert_eq!(
        rows(store.snapshot()),
        [MutationServiceCounters {
            allow_total: u64::MAX,
            deny_total: u64::MAX,
            policy_deny_total: u64::MAX,
        }; 10]
    );
}

#[test]
fn mutation_counters_concurrent_quiescent_totals_are_exact() {
    let store = MutationDecisions::default();
    std::thread::scope(|scope| {
        for _ in 0..8 {
            scope.spawn(|| {
                for _ in 0..100 {
                    for service in SERVICES {
                        store.record(service, MutationDecision::Allow);
                        store.record(service, MutationDecision::Deny);
                        store.record(service, MutationDecision::PolicyDeny);
                    }
                }
            });
        }
    });
    assert_eq!(
        rows(store.snapshot()),
        [MutationServiceCounters {
            allow_total: 800,
            deny_total: 1600,
            policy_deny_total: 800,
        }; 10]
    );
}
