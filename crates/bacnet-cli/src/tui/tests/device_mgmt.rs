//! `:dcc` and `:reinit` against the lab device on the in-memory site
//! (#1799): the requests on the wire with their parameters and password,
//! the replies and errors shown, the audit records, the dangerous opt-in
//! enforced by the worker too, the masked prompt and the T3 confirmation at
//! 80x24, and a distinctive password found in none of the files the
//! session writes. All on paused time.

use std::path::Path;

use bacnet_services::device_mgmt::{DeviceCommunicationControlRequest, ReinitializeDeviceRequest};
use bacnet_types::enums::{ConfirmedServiceChoice, EnableDisable, ReinitializedState};
use insta::assert_snapshot;
use ratatui::crossterm::event::KeyCode;

use super::browser::open_device;
use super::site::{ClientLimits, Server, Site};
use super::writes::{LAB, LAB_MAC, arm, audit, finished, modal, options, outcomes};
use super::*;
use crate::tui::app::safety::{CONFIRM_GUARD, Modal};
use crate::tui::profile::{Safety, Writes};
use crate::tui::safety::audit::Secret;
use crate::tui::safety::manage::{Action, NEEDS_DANGEROUS, Request};
use crate::tui::safety::{Outcome, SafetyCommand, SafetyEvent};

/// The device's password, distinctive enough to grep for.
pub(super) const PASSWORD: &str = "Zq7-hunter2-Xv9";

pub(super) const DANGEROUS: Safety = Safety {
    writes: Writes::Session,
    dangerous: true,
};

/// The lab device, accepting `PASSWORD`, with a profile workspace in
/// `dir` and the device open in the browser.
pub(super) async fn site(dir: &Path, policy: Safety) -> Site {
    let server = Server {
        password: Some(PASSWORD),
        ..Server::lab(100, LAB)
    };
    let options = options(dir, policy, false);
    let mut site = Site::start_with_options(vec![server], ClientLimits::default(), options).await;
    site.pump_until(|app| app.safety.policy.is_some()).await;
    open_device(&mut site).await;
    site
}

/// `:LINE`, then `password` at the prompt and Enter.
async fn start(site: &mut Site, line: &str, password: &str) {
    site.press(KeyCode::Char(':')).await;
    type_text(&mut site.app, line);
    site.press(KeyCode::Enter).await;
    assert!(
        matches!(modal(&site.app), Some(Modal::Password { .. })),
        "{:?}",
        site.app.flash
    );
    type_text(&mut site.app, password);
    site.press(KeyCode::Enter).await;
    advance(&mut site.app, CONFIRM_GUARD);
}

/// Type the device instance, send, and wait for the result.
async fn confirm(site: &mut Site) {
    type_text(&mut site.app, &LAB.to_string());
    site.press(KeyCode::Enter).await;
    finished(site).await;
}

pub(super) fn bodies(site: &Site, service: ConfirmedServiceChoice) -> Vec<Vec<u8>> {
    site.hub.bodies_to(&LAB_MAC, service)
}

#[tokio::test(start_paused = true)]
async fn dcc_sends_its_state_duration_and_password_and_shows_the_ack() {
    let dir = tempfile::tempdir().unwrap();
    let mut site = site(dir.path(), DANGEROUS).await;
    arm(&mut site).await;
    start(&mut site, "dcc disable-initiation 30", PASSWORD).await;
    confirm(&mut site).await;
    assert_eq!(outcomes(&site.app), [Outcome::Ack]);
    let [body] = bodies(&site, ConfirmedServiceChoice::DEVICE_COMMUNICATION_CONTROL)
        .try_into()
        .unwrap();
    let sent = DeviceCommunicationControlRequest::decode(&body).unwrap();
    assert_eq!(sent.enable_disable, EnableDisable::DISABLE_INITIATION);
    assert_eq!(sent.time_duration, Some(30));
    assert_eq!(sent.password.as_deref(), Some(PASSWORD));
    let records = audit(dir.path());
    let dcc = records.iter().find(|r| r["event"] == "dcc").unwrap();
    assert_eq!(dcc["service"], "DeviceCommunicationControl");
    assert_eq!(dcc["target"]["device"], LAB);
    assert_eq!(dcc["params"]["state"], "disable-initiation");
    assert_eq!(dcc["params"]["minutes"], 30);
    assert_eq!(dcc["params"]["password"], "[redacted]");
    assert_eq!(dcc["tier"], "T3");
    assert_eq!(dcc["level"], "write-session+dangerous");
    assert_eq!(dcc["outcome"]["result"], "ack");
    assert!(site.app.safety.ledger.is_empty(), "not kept for revert");
    site.stop().await;
}

#[tokio::test(start_paused = true)]
async fn a_wrong_reinit_password_is_shown_as_the_devices_error() {
    let dir = tempfile::tempdir().unwrap();
    let mut site = site(dir.path(), DANGEROUS).await;
    arm(&mut site).await;
    start(&mut site, "reinit warmstart", "not-it").await;
    confirm(&mut site).await;
    let [Outcome::Refused(_)] = outcomes(&site.app)[..] else {
        panic!("{:?}", outcomes(&site.app));
    };
    let text = screen(&mut site.app, 80, 24);
    assert!(text.contains("PASSWORD_FAILURE"), "{text}");
    let [body] = bodies(&site, ConfirmedServiceChoice::REINITIALIZE_DEVICE)
        .try_into()
        .unwrap();
    let sent = ReinitializeDeviceRequest::decode(&body).unwrap();
    assert_eq!(sent.reinitialized_state, ReinitializedState::WARMSTART);
    assert_eq!(sent.password.as_deref(), Some("not-it"));
    // The right one is acknowledged; a blank one sends none.
    site.press(KeyCode::Esc).await;
    start(&mut site, "reinit coldstart", PASSWORD).await;
    confirm(&mut site).await;
    assert_eq!(outcomes(&site.app), [Outcome::Ack]);
    site.press(KeyCode::Esc).await;
    start(&mut site, "reinit warmstart", "").await;
    confirm(&mut site).await;
    let sent: Vec<_> = bodies(&site, ConfirmedServiceChoice::REINITIALIZE_DEVICE)
        .iter()
        .map(|body| ReinitializeDeviceRequest::decode(body).unwrap())
        .collect();
    assert_eq!(sent.len(), 3);
    assert_eq!(sent[1].reinitialized_state, ReinitializedState::COLDSTART);
    assert_eq!(sent[2].password, None);
    let records = audit(dir.path());
    let reinit: Vec<_> = records.iter().filter(|r| r["event"] == "reinit").collect();
    assert_eq!(reinit.len(), 3);
    assert_eq!(reinit[0]["outcome"]["code"], "PASSWORD_FAILURE");
    assert_eq!(reinit[1]["outcome"]["result"], "ack");
    assert!(reinit[2]["params"].get("password").is_none(), "none sent");
    site.stop().await;
}

/// Without `dangerous = true` the bar refuses both with the reason, and
/// the worker refuses a request sent straight to it, and records that.
#[tokio::test(start_paused = true)]
async fn without_the_dangerous_opt_in_both_are_refused_and_nothing_is_sent() {
    let dir = tempfile::tempdir().unwrap();
    let mut site = site(dir.path(), Safety::default()).await;
    arm(&mut site).await;
    for line in ["dcc disable 10", "reinit coldstart"] {
        site.app.flash = None;
        site.press(KeyCode::Char(':')).await;
        type_text(&mut site.app, line);
        site.press(KeyCode::Enter).await;
        assert!(modal(&site.app).is_none(), "{line}");
        let flash = &site.app.flash.as_ref().unwrap().text;
        assert!(flash.contains(NEEDS_DANGEROUS), "{line}: {flash}");
    }
    let request = Request {
        device: LAB,
        action: Action::Reinit {
            state: ReinitializedState::COLDSTART,
        },
        password: Some(Secret::new(PASSWORD)),
    };
    let manage = SafetyCommand::Manage {
        req: 5,
        request: Box::new(request),
    };
    site.send(vec![Command::Safety(manage)]).await;
    site.pump_events_until(|event| {
        matches!(event, WorkerEvent::Safety(SafetyEvent::Refused { req: Some(5), reason })
            if reason == NEEDS_DANGEROUS)
    })
    .await;
    for service in [
        ConfirmedServiceChoice::DEVICE_COMMUNICATION_CONTROL,
        ConfirmedServiceChoice::REINITIALIZE_DEVICE,
    ] {
        assert!(bodies(&site, service).is_empty(), "{service:?}");
    }
    let records = audit(dir.path());
    let refused = records.iter().find(|r| r["event"] == "reinit").unwrap();
    assert_eq!(refused["outcome"]["result"], "refused");
    assert_eq!(refused["params"]["password"], "[redacted]");
    site.stop().await;
}

/// Every file under `dir`, recursively.
fn files(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            out.extend(files(&path));
        } else {
            out.push(path);
        }
    }
    out
}

/// The password goes on the wire and nowhere else: not the audit log, the
/// ledger, the workspace's other files, the log (every level, to a file in
/// the workspace), any frame drawn, nor this test's snapshots. The masked
/// prompt and the T3 confirmation are snapshot at 80x24. The capture, which
/// this hub's ports can't feed, is checked over real B/IP in
/// `device_mgmt_capture`.
#[tokio::test(start_paused = true)]
async fn the_password_reaches_no_file_frame_or_log() {
    let dir = tempfile::tempdir().unwrap();
    let log = std::fs::File::create(dir.path().join("session.log")).unwrap();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .with_writer(std::sync::Mutex::new(log))
        .finish();
    let _logging = tracing::subscriber::set_default(subscriber);
    let mut site = site(dir.path(), DANGEROUS).await;
    arm(&mut site).await;
    let mut frames = Vec::new();
    site.press(KeyCode::Char(':')).await;
    type_text(&mut site.app, "dcc disable-initiation 30");
    site.press(KeyCode::Enter).await;
    type_text(&mut site.app, PASSWORD);
    let prompt = screen(&mut site.app, 80, 24);
    assert_snapshot!("dcc_password_prompt_80x24", prompt);
    frames.push(prompt);
    site.press(KeyCode::Enter).await;
    advance(&mut site.app, CONFIRM_GUARD);
    let t3 = screen(&mut site.app, 80, 24);
    assert_snapshot!("dcc_confirm_t3_80x24", t3);
    frames.push(t3);
    confirm(&mut site).await;
    frames.push(screen(&mut site.app, 120, 40));
    site.press(KeyCode::Esc).await;
    start(&mut site, "reinit warmstart", PASSWORD).await;
    frames.push(screen(&mut site.app, 80, 24));
    site.press(KeyCode::Esc).await;
    start(&mut site, "reinit coldstart", PASSWORD).await;
    confirm(&mut site).await;
    frames.push(screen(&mut site.app, 80, 24));
    // On the wire, as the protocol carries it.
    let dcc = bodies(&site, ConfirmedServiceChoice::DEVICE_COMMUNICATION_CONTROL);
    let sent = DeviceCommunicationControlRequest::decode(&dcc[0]).unwrap();
    assert_eq!(sent.password.as_deref(), Some(PASSWORD));
    site.stop().await;
    drop(_logging);
    let records = audit(dir.path());
    let events: Vec<&str> = records.iter().filter_map(|r| r["event"].as_str()).collect();
    assert_eq!(events, ["arm", "dcc", "declined", "reinit"]);
    let written = files(dir.path());
    assert!(written.len() >= 3, "{written:?}");
    for path in written {
        let bytes = std::fs::read(&path).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        assert!(
            !text.contains(PASSWORD),
            "{} holds the password",
            path.display()
        );
    }
    let log = std::fs::read_to_string(dir.path().join("session.log")).unwrap();
    assert!(!log.is_empty(), "the session logged");
    for frame in &frames {
        assert!(!frame.contains(PASSWORD), "{frame}");
    }
    let snapshots = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/tui/tests/snapshots");
    for name in ["dcc_password_prompt_80x24", "dcc_confirm_t3_80x24"] {
        let path = snapshots.join(format!("bacnet__tui__tests__device_mgmt__{name}.snap"));
        let text =
            std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        assert!(!text.contains(PASSWORD), "{}", path.display());
    }
}

/// #2056: a DCC or reinitialize the device's policy refuses
/// (SERVICE_REQUEST_DENIED) shows as denied by the device's policy, with no
/// word of writes. The lab device here has no password, so its DCC policy
/// denies every DCC, and its mutation policy refuses the reinitialize.
#[tokio::test(start_paused = true)]
async fn a_denied_dcc_or_reinitialize_names_the_devices_policy() {
    let dir = tempfile::tempdir().unwrap();
    let server = Server {
        deny_writes: true,
        ..Server::lab(100, LAB)
    };
    let options = options(dir.path(), DANGEROUS, false);
    let mut site = Site::start_with_options(vec![server], ClientLimits::default(), options).await;
    site.pump_until(|app| app.safety.policy.is_some()).await;
    open_device(&mut site).await;
    arm(&mut site).await;
    for line in ["dcc disable-initiation 30", "reinit warmstart"] {
        start(&mut site, line, "").await;
        confirm(&mut site).await;
        assert_eq!(outcomes(&site.app), [Outcome::Denied], "{line}");
        let text = screen(&mut site.app, 80, 24);
        assert!(text.contains("DENIED by the device's policy"), "{text}");
        assert!(!text.contains("write"), "{text}");
        site.press(KeyCode::Esc).await;
    }
    site.stop().await;
}
