//! How a refused write reads (#1814): the result view and the audit log name
//! a refusal's class and code, or its reject or abort reason, and give the
//! number for a value without a name; in the audit log each is a string.

use std::time::Duration;

use bacnet_types::enums::{AbortReason, ErrorClass, ErrorCode, RejectReason};
use bacnet_types::error::{Error, ErrorDetail, RemoteError};
use insta::assert_snapshot;
use serde_json::json;

use super::*;
use crate::tui::app::Overlay;
use crate::tui::app::safety::Modal;
use crate::tui::safety::Outcome;

fn refusals() -> Vec<(String, Outcome)> {
    let label = "device 100 analog-output 1 present-value @8";
    [
        RemoteError::Error {
            class: ErrorClass::PROPERTY,
            code: ErrorCode::WRITE_ACCESS_DENIED,
        },
        RemoteError::Error {
            class: ErrorClass::from_raw(64),
            code: ErrorCode::from_raw(300),
        },
        RemoteError::Reject {
            reason: RejectReason::INVALID_PARAMETER_DATA_TYPE,
        },
        RemoteError::Abort {
            reason: AbortReason::from_raw(200),
        },
    ]
    .into_iter()
    .map(|remote| (label.to_owned(), Outcome::Refused(remote)))
    .collect()
}

#[test]
fn the_result_view_names_refusals_and_numbers_the_unnamed() {
    let mut app = connected_app();
    app.overlay = Overlay::Safety(Box::new(Modal::Result(refusals())));
    assert_snapshot!("write_refusals_100x30", screen(&mut app, 100, 30));
}

#[test]
fn the_audit_log_names_refusals_and_numbers_the_unnamed() {
    let records: Vec<_> = refusals().iter().map(|(_, o)| o.json()).collect();
    assert_eq!(
        records,
        [
            json!({ "result": "error", "class": "PROPERTY", "code": "WRITE_ACCESS_DENIED" }),
            json!({ "result": "error", "class": "64", "code": "300" }),
            json!({ "result": "reject", "reason": "INVALID_PARAMETER_DATA_TYPE" }),
            json!({ "result": "abort", "reason": "200" }),
        ]
    );
    assert!(refusals().iter().all(|(_, o)| o.applied() == Some(false)));
}

#[test]
fn a_client_result_keeps_its_typed_refusal() {
    let (class, code) = (ErrorClass::PROPERTY, ErrorCode::VALUE_OUT_OF_RANGE);
    let error = RemoteError::Error { class, code };
    assert_eq!(
        Outcome::of(&Err(Error::Remote(error))),
        Outcome::Refused(error)
    );
    // A structured error keeps its class and code.
    let detail = Some(ErrorDetail::FirstFailedElementNumber(2));
    assert_eq!(
        Outcome::of(&Err(Error::with_detail(class, code, detail))),
        Outcome::Refused(error)
    );
    let reject = RemoteError::Reject {
        reason: RejectReason::from_raw(99),
    };
    assert_eq!(
        Outcome::of(&Err(Error::Remote(reject))),
        Outcome::Refused(reject)
    );
    assert_eq!(
        Outcome::of(&Err(Error::protocol(
            ErrorClass::SERVICES,
            ErrorCode::SERVICE_REQUEST_DENIED
        ))),
        Outcome::Denied
    );
    for timeout in [
        Error::abort(AbortReason::TSM_TIMEOUT),
        Error::Timeout(Duration::from_secs(3)),
    ] {
        assert_eq!(Outcome::of(&Err(timeout)), Outcome::Timeout);
    }
    assert_eq!(Outcome::of(&Ok(())), Outcome::Ack);
    assert!(matches!(
        Outcome::of(&Err(Error::buffer_too_short(2, 1))),
        Outcome::Failed(_)
    ));
}
