//! The application's route to a stored Program's Program_State (#2258).
//!
//! Program_State is read-only on the network (Clause 12.22.4), and a local
//! property write is refused the same way, so `set_program_state_local` is
//! how the application reports the state of the process it runs.
use super::cov_wire_test_support::*;
use super::*;
use bacnet_objects::program::ProgramObject;
use bacnet_types::enums::ObjectType;
use bacnet_types::error::RemoteError;

const PS: PropertyIdentifier = PropertyIdentifier::PROGRAM_STATE;

fn program() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::PROGRAM, 1).unwrap()
}

fn code(error: Error) -> (ErrorClass, ErrorCode) {
    match error {
        Error::Remote(RemoteError::Error { class, code }) => (class, code),
        other => panic!("expected a protocol error, got {other:?}"),
    }
}

async fn state(h: &Harness) -> PropertyValue {
    h.server
        .database()
        .read()
        .await
        .get(&program())
        .unwrap()
        .read_property(PS, None)
        .unwrap()
}

#[tokio::test(start_paused = true)]
async fn program_state_takes_the_application_route_and_refuses_a_local_write() {
    let mut h = Harness::start_with(ServerConfig::default(), |db| {
        db.add(Box::new(ProgramObject::new(1, "PRG-1").unwrap()))
            .unwrap();
    })
    .await;
    let error = h
        .server
        .write_local(
            &program(),
            PS,
            None,
            PropertyValue::Enumerated(2),
            None,
            crate::LocalCommandSource::ServerDevice,
        )
        .await
        .unwrap_err();
    assert_eq!(
        code(error),
        (ErrorClass::PROPERTY, ErrorCode::WRITE_ACCESS_DENIED)
    );
    assert_eq!(state(&h).await, PropertyValue::Enumerated(0));

    h.server
        .set_program_state_local(&program(), PropertyValue::Enumerated(2))
        .await
        .unwrap();
    assert_eq!(state(&h).await, PropertyValue::Enumerated(2));
    for (value, expected) in [
        (PropertyValue::Enumerated(6), ErrorCode::VALUE_OUT_OF_RANGE),
        (PropertyValue::Unsigned(1), ErrorCode::INVALID_DATA_TYPE),
    ] {
        let error = h
            .server
            .set_program_state_local(&program(), value)
            .await
            .unwrap_err();
        assert_eq!(code(error), (ErrorClass::PROPERTY, expected));
    }
    assert_eq!(state(&h).await, PropertyValue::Enumerated(2));
    let error = h
        .server
        .set_program_state_local(&av1(), PropertyValue::Enumerated(1))
        .await
        .unwrap_err();
    assert_eq!(
        code(error),
        (
            ErrorClass::OBJECT,
            ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED
        )
    );
    h.server.stop().await.unwrap();
}
