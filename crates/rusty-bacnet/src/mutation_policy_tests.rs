//! The policy a Python server installs stays on Tokio's blocking pool
//! (#2049): a Python callable may block, so it is never asked inline.
use super::*;
use bacnet_server::ServerPolicy;

fn callback() -> Arc<PyMutationPolicy> {
    Python::initialize();
    Python::attach(|py| {
        let callable = py.eval(c"lambda context: True", None, None).unwrap();
        Arc::new(PyMutationPolicy {
            callable: callable.unbind(),
            keyword: "mutation_policy",
        })
    })
}

#[test]
fn a_python_servers_policy_is_never_asked_inline() {
    for audit_allow_all in [false, true] {
        let policy = server_policy(audit_allow_all, Some(callback())).unwrap();
        assert!(policy.decides_mutations());
        assert!(
            !policy.decides_inline(),
            "audit_allow_all: {audit_allow_all}"
        );
    }
    let audit_only = server_policy(true, None).unwrap();
    assert!(!audit_only.decides_mutations());
    assert!(!audit_only.decides_inline());
    assert!(server_policy(false, None).is_none());
}
