//! The credential lint in the TUI (#968): the preview holds a connection
//! until its warnings are accepted, the coordinator refuses a hard failure
//! or an unaccepted warning before any dial, and the SC screen's
//! credentials view, with its frame at 120x40.

use super::sc_screen::{identity, sc_app};
use super::*;
use crate::tui::app::connection::Connection;
use crate::tui::app::sc::View;
use crate::tui::app::watch::Screen;
use crate::tui::app::{Link, Overlay};
use crate::tui::profile::checks::Checks;
use crate::tui::profile::sc::{DeviceUuid, Vmac};
use crate::tui::profile::service::{Reply, Request};
use crate::tui::profile::{Overrides, Profile};
use crate::tui::sc::credentials::{Cert, Credentials, Level};
use crate::tui::sc::{Fault, ScFailure, ScReport};
use insta::assert_snapshot;

fn sc(app: &mut App, report: ScReport) {
    worker(app, WorkerEvent::Sc(report));
}

/// A preview of an authorized SC profile with these checks.
fn preview(checks: Checks) -> App {
    let mut p = Profile::sc_default();
    let s = p.sc.as_mut().unwrap();
    s.hub_url = "wss://hub.site.example:443/bacnet".into();
    s.ca = "/etc/bacnet/site/ca.pem".into();
    s.cert = "/etc/bacnet/site/node.pem".into();
    s.key = "/etc/bacnet/site/node.key".into();
    s.vmac = Some(Vmac([0x22, 0x10, 0x20, 0x30, 0x40, 0x50]));
    s.device_uuid = Some(DeviceUuid([0x6f; 16]));
    p.authorized_at_unix = Some(1_794_000_000);
    p.authorized_safety = Some(p.safety.clone());
    let mut app = new_app(None);
    app.link = Link::Down;
    app.overlay = Overlay::Connection;
    app.connection = Some(Connection::new(Overrides::default()));
    worker(
        &mut app,
        WorkerEvent::Profile(Ok(Reply::Loaded {
            name: "hub-west".into(),
            profile: p,
            checks,
        })),
    );
    app
}

const THIRTY: &str = "Operational certificate [0]: It expires within thirty days. Plan renewal.";

#[test]
fn warnings_hold_the_connection_until_y_accepts_them() {
    let warnings = vec![THIRTY.to_owned()];
    let mut app = preview(Checks {
        errors: Vec::new(),
        warnings: warnings.clone(),
    });
    let text = screen(&mut app, 120, 40);
    assert!(text.contains("y accept these warnings"), "{text}");
    for key in ['c', 'a'] {
        assert!(press(&mut app, KeyCode::Char(key)).is_empty());
        let connection = app.connection.as_ref().unwrap();
        assert!(connection
            .error
            .as_ref()
            .unwrap()
            .contains("y accepts them"));
    }
    assert!(press(&mut app, KeyCode::Enter).is_empty());
    assert_eq!(app.link, Link::Down, "nothing was asked of the coordinator");
    assert!(press(&mut app, KeyCode::Char('y')).is_empty());
    assert!(screen(&mut app, 120, 40).contains("Warnings accepted for this connection."));
    let commands = press(&mut app, KeyCode::Char('c'));
    let [Command::Profile(Request::Connect { accepted, .. })] = commands.as_slice() else {
        panic!("expected a connect, got {commands:?}");
    };
    assert_eq!(accepted, &warnings);
}

#[test]
fn y_with_no_warnings_says_so_and_errors_still_go_to_the_coordinator() {
    let mut app = preview(Checks::default());
    press(&mut app, KeyCode::Char('y'));
    let connection = app.connection.as_ref().unwrap();
    assert_eq!(connection.error.as_deref(), Some("No warnings to accept."));
    // An error is refused (freshly) by the coordinator, which names it.
    let mut app = preview(Checks {
        errors: vec!["Key file: The private key does not match the identity leaf.".into()],
        warnings: vec![THIRTY.into()],
    });
    let commands = press(&mut app, KeyCode::Char('c'));
    assert!(matches!(
        commands.as_slice(),
        [Command::Profile(Request::Connect { .. })]
    ));
}

#[test]
fn sc_preview_with_lint_findings_at_120x40() {
    let mut app = preview(Checks {
        errors: Vec::new(),
        warnings: vec![
            THIRTY.into(),
            "Operational certificate [0]: No CA in the --sc-ca bundle signed the identity leaf \
             directly. Peers using the Annex AB.7.4 default check with this bundle can refuse \
             it; have a bundle CA issue it, or add its issuing CA to every peer's bundle."
                .into(),
        ],
    });
    assert_snapshot!("sc_profile_lint_warnings_120x40", screen(&mut app, 120, 40));
}

pub(super) fn credentials(findings: Vec<(Level, String)>) -> Credentials {
    let cert = |label: &str, subject: &str, window: &str, level| Cert {
        label: label.into(),
        subject: subject.into(),
        issuer: "CN=site issuing CA".into(),
        san: "DNS:node-17.site.example".into(),
        key_algorithm: "EC".into(),
        eku: "client-auth, server-auth".into(),
        not_after: "2024-01-26T00:00:00Z (Unix 1706227200)".into(),
        window: window.into(),
        level,
    };
    let mut ca = cert("CA 0", "CN=site root CA", "400 days left", Level::Ok);
    ca.issuer = "CN=site root CA".into();
    ca.san = "none".into();
    ca.eku = "none (any purpose)".into();
    ca.not_after = "2025-02-04T00:00:00Z (Unix 1738627200)".into();
    Credentials {
        inspected_at: "2024-01-01T00:00:00Z (Unix 1704067200)".into(),
        certificates: vec![
            cert(
                "certificate 0",
                "CN=node-17",
                "25 days left",
                Level::Warning,
            ),
            ca,
        ],
        key: (Level::Ok, "matches the operational certificate".into()),
        issuer: (
            Level::Warning,
            "no bundle CA issued it directly (Annex AB.7.4 default)".into(),
        ),
        findings,
    }
}

#[test]
fn sc_credentials_frame_at_120x40() {
    let mut app = sc_app();
    sc(&mut app, ScReport::Dialing(identity()));
    let findings = vec![
        (Level::Warning, THIRTY.to_owned()),
        (
            Level::Warning,
            "Operational certificate [0]: No CA in the --sc-ca bundle signed the identity leaf \
             directly."
                .to_owned(),
        ),
    ];
    sc(
        &mut app,
        ScReport::Credentials(Box::new(Ok(credentials(findings)))),
    );
    press(&mut app, KeyCode::Char('5'));
    press(&mut app, KeyCode::BackTab);
    assert_eq!(app.sc.view, View::Credentials);
    let text = screen(&mut app, 120, 40);
    assert!(text.contains("[My credentials]"), "{text}");
    assert!(text.contains("[warn] certificate 0"), "{text}");
    assert!(text.contains("25 days left"), "{text}");
    assert!(
        text.contains("stricter than the Annex AB.7.4 default"),
        "{text}"
    );
    assert_snapshot!("sc_credentials_120x40", text);
}

#[test]
fn a_refused_dial_opens_the_credentials_view() {
    let mut app = sc_app();
    sc(&mut app, ScReport::Dialing(identity()));
    let refusal = "Key file: The private key does not match the identity leaf. Select the \
                   corresponding certificate and key.";
    let mut view = credentials(vec![(Level::Failed, refusal.into())]);
    view.key = (
        Level::Failed,
        "does NOT match the operational certificate".into(),
    );
    sc(&mut app, ScReport::Credentials(Box::new(Ok(view))));
    let detail = format!("SC credential check refused the dial before connecting: {refusal}");
    sc(
        &mut app,
        ScReport::Failed(Box::new(ScFailure {
            fault: Fault::Other(detail.clone()),
            detail,
            presented: Vec::new(),
        })),
    );
    assert_eq!(app.sc.view, View::Credentials);
    app.screen = Screen::Sc;
    let text = screen(&mut app, 120, 40);
    assert!(text.contains("[My credentials (FAIL)]"), "{text}");
    assert!(text.contains("[FAIL] Key"), "{text}");
    assert!(text.contains("refused before connecting"), "{text}");
    // A lint that could not read the files is a refusal too.
    sc(
        &mut app,
        ScReport::Credentials(Box::new(Err(
            "SC credential check could not complete".into()
        ))),
    );
    assert!(app.sc.lint_refused());
}

/// The coordinator, against real files: a mismatched key and an unaccepted
/// warning are refused before any socket; an accepted warning dials.
#[cfg(feature = "sc-tls")]
mod coordinator {
    use super::super::connections::event;
    use super::*;
    use crate::tui::profile::sc_tests::sc_profile;
    use crate::tui::profile::service::Service;
    use crate::tui::profile::store::Store;
    use crate::tui::worker::{coordinator, EventSink};
    use std::path::Path;
    use tokio::sync::mpsc;

    struct Coordinator {
        commands: mpsc::Sender<Command>,
        events: mpsc::Receiver<WorkerEvent>,
        task: tokio::task::JoinHandle<()>,
    }

    impl Coordinator {
        fn spawn(root: &Path) -> Self {
            let service = Service::new(Ok(Store::new(root.to_owned())), Overrides::default());
            let (sink, events) = EventSink::channel(16);
            let (commands, rx) = mpsc::channel(16);
            let task = coordinator::spawn(None, Some(service), sink, rx, Default::default());
            Self {
                commands,
                events,
                task,
            }
        }
        async fn connect(&mut self, profile: &Profile, accepted: Vec<String>) -> WorkerEvent {
            let request = Request::Connect {
                name: "site".into(),
                profile: profile.clone(),
                attest: true,
                accepted,
            };
            self.commands.send(Command::Profile(request)).await.unwrap();
            event(&mut self.events).await
        }
        async fn quit(self) {
            drop((self.commands, self.events));
            tokio::time::timeout(Duration::from_secs(30), self.task)
                .await
                .expect("coordinator quit")
                .unwrap();
        }
    }

    fn ca(name: &str) -> (rcgen::CertificateParams, rcgen::KeyPair) {
        let mut params = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
        params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        params.distinguished_name = rcgen::DistinguishedName::new();
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, name);
        (params, rcgen::KeyPair::generate().unwrap())
    }

    fn no_dial(hub: &std::net::TcpListener) {
        assert_eq!(
            hub.accept().map(|_| ()).unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock,
            "the hub was dialled"
        );
    }

    #[tokio::test]
    async fn a_mismatched_key_is_refused_and_nothing_dials_the_hub() {
        let hub = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        hub.set_nonblocking(true).unwrap();
        let url = format!("wss://127.0.0.1:{}", hub.local_addr().unwrap().port());
        let (root, files) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let p = sc_profile(files.path(), &url);
        let other = rcgen::KeyPair::generate().unwrap();
        std::fs::write(&p.sc.as_ref().unwrap().key, other.serialize_pem()).unwrap();
        Store::new(root.path().into())
            .save("site", &p, true)
            .unwrap();
        let mut coordinator = Coordinator::spawn(root.path());
        match coordinator.connect(&p, Vec::new()).await {
            WorkerEvent::Profile(Err(error)) => {
                assert!(
                    error.contains("Key file: The private key does not match"),
                    "{error}"
                );
            }
            other => panic!("expected a refusal, got {other:?}"),
        }
        assert!(coordinator.events.try_recv().is_err(), "no session started");
        coordinator.quit().await;
        // Every session is joined: a dial would have left a connection queued.
        no_dial(&hub);
    }

    #[tokio::test]
    async fn a_warning_is_refused_until_accepted_and_then_dials() {
        let hub = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        hub.set_nonblocking(true).unwrap();
        let url = format!("wss://127.0.0.1:{}", hub.local_addr().unwrap().port());
        let (root, files) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let p = sc_profile(files.path(), &url);
        // The leaf comes from an issuing CA the bundle lacks: a warning.
        let (root_ca, issuing) = (ca("site root"), ca("site issuing"));
        let sc = p.sc.as_ref().unwrap();
        let key = rcgen::KeyPair::generate().unwrap();
        let leaf = rcgen::CertificateParams::new(vec!["node".into()])
            .unwrap()
            .signed_by(&key, &rcgen::Issuer::from_params(&issuing.0, &issuing.1))
            .unwrap();
        std::fs::write(&sc.cert, leaf.pem()).unwrap();
        std::fs::write(&sc.key, key.serialize_pem()).unwrap();
        std::fs::write(&sc.ca, root_ca.0.self_signed(&root_ca.1).unwrap().pem()).unwrap();
        Store::new(root.path().into())
            .save("site", &p, true)
            .unwrap();
        let shown = crate::tui::profile::checks::inspect(&p).warnings;
        assert_eq!(shown.len(), 1, "{shown:?}");

        let mut coordinator = Coordinator::spawn(root.path());
        match coordinator.connect(&p, Vec::new()).await {
            WorkerEvent::Profile(Err(error)) => {
                assert!(error.contains("confirm the warnings first"), "{error}");
            }
            other => panic!("expected a refusal, got {other:?}"),
        }
        assert!(coordinator.events.try_recv().is_err(), "no session started");
        no_dial(&hub);

        let hub = tokio::net::TcpListener::from_std(hub).unwrap();
        assert!(matches!(
            coordinator.connect(&p, shown).await,
            WorkerEvent::Profile(Ok(_))
        ));
        let (socket, _) = tokio::time::timeout(Duration::from_secs(30), hub.accept())
            .await
            .expect("the accepted warning lets the profile dial")
            .unwrap();
        // End the TLS handshake so the attempt fails and can be joined.
        drop(socket);
        coordinator.quit().await;
    }
}
