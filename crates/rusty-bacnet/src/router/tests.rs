//! Native causes and copied routes through the binding's actual conversions.
use super::{
    errors,
    lifecycle::{CloseError, Lifecycle, Session, StartError},
    session::NativeSession,
};
use bacnet_network::router::{BACnetRouter, RouterOptions, RouterPort, RouterStopError};
use bacnet_network::router_table::{Duration, PortId, Reachability};
use bacnet_transport::port::{ReceivedNpdu, TransportPort};
use bacnet_types::enums::{AbortReason, ErrorClass, ErrorCode, RejectReason};
use bacnet_types::error::{Error, RemoteError};
use pyo3::prelude::*;
use tokio::sync::mpsc;

struct FailingPort {
    error: Option<Error>,
    incoming: Option<mpsc::Sender<ReceivedNpdu>>,
}
impl TransportPort for FailingPort {
    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        let (tx, rx) = mpsc::channel(4);
        self.incoming = Some(tx);
        Ok(rx)
    }
    async fn stop(&mut self) -> Result<(), Error> {
        self.incoming.take();
        self.error.take().map_or(Ok(()), Err)
    }
    async fn send_unicast(&self, _: &[u8], _: &[u8]) -> Result<(), Error> {
        Ok(())
    }
    async fn send_broadcast(&self, _: &[u8]) -> Result<(), Error> {
        Ok(())
    }
    fn local_mac(&self) -> &[u8] {
        &[127, 0, 0, 1, 0, 9]
    }
    fn local_receive_apdu_capacity(&self) -> u16 {
        1476
    }
}
struct Running(BACnetRouter);
impl Session for Running {
    type StartError = Error;
    type StopError = RouterStopError;
    async fn start(&mut self) -> Result<(), Error> {
        Ok(())
    }
    async fn stop(&mut self) -> Result<(), RouterStopError> {
        self.0.stop().await
    }
}
async fn native(errors: Vec<Option<Error>>) -> BACnetRouter {
    BACnetRouter::start(
        errors
            .into_iter()
            .enumerate()
            .map(|(i, error)| RouterPort {
                network_number: 1 + i as u16 * 4,
                transport: FailingPort {
                    error,
                    incoming: None,
                },
            })
            .collect(),
        RouterOptions::new(),
    )
    .await
    .unwrap()
    .router
}

#[tokio::test]
async fn native_cleanup_causes_survive_repeated_python_conversion_and_context_body_error() {
    Python::initialize();
    let router = native(vec![
        Some(Error::Transport(std::io::Error::from_raw_os_error(13))),
        Some(Error::protocol(ErrorClass::SERVICES, ErrorCode::OTHER)),
        None,
    ])
    .await;
    let owner = Lifecycle::new();
    owner
        .start(false, move || Running(router))
        .await
        .unwrap()
        .publish(|_| {});
    // The context helper awaits this same retained failure. Returning None
    // with a body exception leaves Python's original exception primary.
    errors::context_outcome(owner.close().await, true, &[1, 5, 9]).unwrap();
    for _ in 0..3 {
        let outcome = owner.close().await;
        let error = errors::context_outcome(outcome, false, &[1, 5, 9]).unwrap_err();
        Python::attach(|py| {
            assert!(error.is_instance_of::<errors::BacnetRouterCloseError>(py));
            let failures = error.value(py).getattr("failures").unwrap();
            assert_eq!(failures.len().unwrap(), 2);
            let io = failures.get_item(0).unwrap();
            assert_eq!(
                io.getattr("network_number")
                    .unwrap()
                    .extract::<u16>()
                    .unwrap(),
                1
            );
            assert_eq!(
                io.getattr("port_index")
                    .unwrap()
                    .extract::<usize>()
                    .unwrap(),
                0
            );
            assert_eq!(io.getattr("errno").unwrap().extract::<i32>().unwrap(), 13);
            assert!(io.getattr("error_class").unwrap().is_none());
            let protocol = failures.get_item(1).unwrap();
            assert_eq!(
                protocol
                    .getattr("network_number")
                    .unwrap()
                    .extract::<u16>()
                    .unwrap(),
                5
            );
            assert_eq!(
                protocol
                    .getattr("port_index")
                    .unwrap()
                    .extract::<usize>()
                    .unwrap(),
                1
            );
            assert_eq!(
                protocol
                    .getattr("error_class")
                    .unwrap()
                    .extract::<u16>()
                    .unwrap(),
                ErrorClass::SERVICES.to_raw()
            );
            assert_eq!(
                protocol
                    .getattr("error_code")
                    .unwrap()
                    .extract::<u16>()
                    .unwrap(),
                ErrorCode::OTHER.to_raw()
            );
            assert!(protocol.getattr("errno").unwrap().is_none());
            assert!(io.setattr("errno", 0).is_err());
        });
    }
    let error = owner
        .start(false, || panic!("failed owner must not build"))
        .await
        .unwrap_err();
    assert!(matches!(error, StartError::Cleanup(CloseError::Stop(_))));
    let error = errors::start_error(error, &[1, 5, 9]);
    Python::attach(|py| assert!(error.is_instance_of::<errors::BacnetRouterCloseError>(py)));
}

/// The error fields are IntEnum members of the right class, pseudo-members
/// for unnamed values, that still equal their numbers (#2025).
#[tokio::test]
async fn cleanup_failure_errors_are_int_enum_members() {
    Python::initialize();
    let router = native(vec![
        Some(Error::protocol(
            ErrorClass::SERVICES,
            ErrorCode::from_raw(600),
        )),
        Some(Error::Remote(RemoteError::Reject {
            reason: RejectReason::from_raw(200),
        })),
        Some(Error::Remote(RemoteError::Abort {
            reason: AbortReason::SEGMENTATION_NOT_SUPPORTED,
        })),
    ])
    .await;
    let owner = Lifecycle::new();
    owner
        .start(false, move || Running(router))
        .await
        .unwrap()
        .publish(|_| {});
    let error = errors::context_outcome(owner.close().await, false, &[1, 5, 9]).unwrap_err();
    Python::attach(|py| {
        let failures = error.value(py).getattr("failures").unwrap();
        let class_of = |value: &Bound<'_, PyAny>| {
            value
                .get_type()
                .name()
                .unwrap()
                .extract::<String>()
                .unwrap()
        };
        let protocol = failures.get_item(0).unwrap();
        let class = protocol.getattr("error_class").unwrap();
        let code = protocol.getattr("error_code").unwrap();
        assert_eq!(class_of(&class), "ErrorClass");
        assert_eq!(class.extract::<u16>().unwrap(), 5);
        assert_eq!(class_of(&code), "ErrorCode");
        assert_eq!(code.extract::<u16>().unwrap(), 600);
        assert!(code.getattr("name").unwrap().is_none());
        assert!(protocol.getattr("reason").unwrap().is_none());
        let reject = failures.get_item(1).unwrap().getattr("reason").unwrap();
        assert_eq!(class_of(&reject), "RejectReason");
        assert_eq!(reject.extract::<u8>().unwrap(), 200);
        let abort = failures.get_item(2).unwrap();
        assert!(abort.getattr("error_class").unwrap().is_none());
        let reason = abort.getattr("reason").unwrap();
        assert_eq!(class_of(&reason), "AbortReason");
        assert_eq!(
            reason.extract::<u8>().unwrap(),
            AbortReason::SEGMENTATION_NOT_SUPPORTED.to_raw()
        );
    });
}

#[tokio::test]
async fn worker_failure_has_a_typed_python_record() {
    Python::initialize();
    let error = errors::close_error(&CloseError::WorkerEnded, &[1]);
    Python::attach(|py| {
        let failure = error
            .value(py)
            .getattr("failures")
            .unwrap()
            .get_item(0)
            .unwrap();
        assert_eq!(
            failure
                .getattr("kind")
                .unwrap()
                .extract::<String>()
                .unwrap(),
            "worker"
        );
        assert_eq!(
            failure
                .getattr("worker")
                .unwrap()
                .extract::<String>()
                .unwrap(),
            "router lifecycle"
        );
        assert!(failure.getattr("network_number").unwrap().is_none());
        assert!(failure.getattr("errno").unwrap().is_none());
    });
}

#[tokio::test]
async fn snapshot_copies_learned_routes_and_effective_expired_busy_state() {
    let router = native(vec![None]).await;
    let now = router.now();
    {
        let mut table = router.table();
        assert!(table.add_learned(
            7,
            PortId(0),
            [127, 0, 0, 1, 0, 77].as_slice().try_into().unwrap(),
            now
        ));
        table.mark_busy(7, now + Duration::from_secs(60));
    }
    let mut session = NativeSession::from_started(router);
    let before = session.routes();
    assert_eq!(before[1].reachability, "busy");
    assert!(!before[1].direct);
    assert_eq!(
        before[1].next_hop.as_deref(),
        Some([127, 0, 0, 1, 0, 77].as_slice())
    );
    {
        let router = session.router.as_ref().unwrap();
        router.table().mark_available(7);
        router.table().mark_busy(7, router.now());
        assert_eq!(
            router.table().lookup(7).unwrap().reachability(),
            Reachability::Busy
        );
    }
    let after = session.routes();
    assert_eq!(after[1].reachability, "reachable");
    session.stop().await.unwrap();
    assert_eq!(before[1].reachability, "busy");
    assert_eq!(after[1].network_number, 7);
}

#[test]
fn real_python_context_preserves_sentinel_while_failed_cleanup_blocks_restart() {
    Python::initialize();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let router = runtime.block_on(native(vec![Some(Error::Transport(
        std::io::Error::from_raw_os_error(13),
    ))]));
    let lifecycle = Lifecycle::with_running_for_test(NativeSession::from_started(router));
    Python::attach(|py| {
        let scope = pyo3::types::PyDict::new(py);
        scope
            .set_item(
                "router",
                Py::new(
                    py,
                    super::PyBACnetRouter {
                        configs: vec![super::config::PortConfig {
                            network_number: 1,
                            interface: std::net::Ipv4Addr::LOCALHOST,
                            port: 0,
                            broadcast: std::net::Ipv4Addr::LOCALHOST,
                            mode: bacnet_transport::bip::BipMode::Normal,
                        }],
                        application: super::application::Application::None,
                        lifecycle,
                        event_loop: std::sync::Mutex::new(None),
                    },
                )
                .unwrap(),
            )
            .unwrap();
        py.run(c"import asyncio\nasync def main():\n    sentinel = ValueError('body sentinel')\n    try:\n        async with router as entered:\n            assert entered is router\n            raise sentinel\n    except ValueError as actual:\n        assert actual is sentinel\n    else:\n        raise AssertionError('body error lost')\n    for method in (router.close, router.close, router.start):\n        try:\n            await method()\n        except Exception as error:\n            assert type(error).__name__ == 'BacnetRouterCloseError', repr(error)\n            assert len(error.failures) == 1\n            assert error.failures[0].network_number == 1\n            assert error.failures[0].port_index == 0\n            assert error.failures[0].errno == 13\n        else:\n            raise AssertionError('cleanup failure lost or restart admitted')\nasyncio.run(main())",Some(&scope),None).unwrap();
    });
}
