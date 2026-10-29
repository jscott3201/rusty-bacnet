//! Execute the canonical shared-endpoint guide, including an explicit registry binary.
#![forbid(unsafe_code)]

#[path = "support/guide_process.rs"]
mod guide;
#[path = "support/guide_interactive.rs"]
mod interactive;
#[path = "support/guide_retirement.rs"]
mod retirement;
use guide::{Process, WAIT};
use interactive::Interactive;
use std::ffi::OsStr;
use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};
use std::path::{Path, PathBuf};
use tokio::time::timeout;

fn select_binary(external: Option<&OsStr>) -> Result<PathBuf, String> {
    let requested =
        external.unwrap_or_else(|| OsStr::new(env!("CARGO_BIN_EXE_guide-shared-endpoint")));
    let path = Path::new(requested)
        .canonicalize()
        .map_err(|e| e.to_string())?;
    if !path.is_file() {
        return Err("selected guide binary is not a file".into());
    }
    Ok(path)
}

#[allow(clippy::print_stderr)] // Record the actual executable used for qualification.
fn binary() -> PathBuf {
    let external = std::env::var_os("BACNET_SHARED_ENDPOINT_BINARY");
    let selected = select_binary(external.as_deref()).expect("resolve explicit guide binary");
    eprintln!(
        "shared-endpoint executable: {} (external={})",
        selected.display(),
        external.is_some()
    );
    selected
}

async fn ready(child: &Interactive) -> SocketAddrV4 {
    let mut stdout = child.stdout();
    let observed = timeout(
        WAIT,
        stdout.wait_for(|bytes| bytes.ends_with(b"Press Enter to stop (or send EOF).\n")),
    )
    .await
    .expect("guide must report readiness")
    .expect("guide stdout must stay open")
    .clone();
    let text = std::str::from_utf8(&observed).unwrap();
    let lines: Vec<_> = text.lines().collect();
    assert_eq!(lines.len(), 3, "{text}");
    let address: SocketAddrV4 = lines[0]
        .strip_prefix("Started endpoint: ")
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(*address.ip(), Ipv4Addr::LOCALHOST);
    assert_ne!(
        address.port(),
        0,
        "published address must carry the actual ephemeral port"
    );
    assert_eq!(lines[1], "Roles: requester=true responder=true");
    assert!(
        UdpSocket::bind(address).is_err(),
        "started owner still holds its socket"
    );
    address
}

fn assert_rebind(address: SocketAddrV4) {
    let rebound = UdpSocket::bind(address).expect("retired endpoint's actual port is reusable");
    assert_eq!(rebound.local_addr().unwrap(), address.into());
}

fn assert_stopped(output: &std::process::Output, address: SocketAddrV4) {
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!(
            "Started endpoint: {address}\nRoles: requester=true responder=true\n\
             Press Enter to stop (or send EOF).\n\
             Stopped: address=None requester=false responder=false\n\
             Rebound endpoint: {address}; owner running=false\n"
        ),
        "post-stop owner state and rebind before owner destruction: {output:?}"
    );
    assert_rebind(address);
}

#[tokio::test]
async fn actual_guide_stops_before_drop_and_rebinds_while_owner_is_live() {
    let mut child = Interactive::start(&binary(), &[]).unwrap();
    let address = ready(&child).await;
    child.send_input(b"\n");
    let output = child.finish().await;
    assert_stopped(&output, address);
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
}

#[tokio::test]
async fn input_error_still_observably_stops_and_rebinds_before_returning_the_error() {
    let mut child = Interactive::start(&binary(), &[]).unwrap();
    let address = ready(&child).await;
    child.send_input(&[0xff, b'\n']);
    let output = child.finish().await;
    assert_stopped(&output, address);
    assert!(!output.status.success(), "{output:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("InvalidData") || stderr.contains("valid UTF-8"),
        "{stderr}"
    );
}

#[tokio::test]
async fn simultaneous_owners_get_distinct_ports_and_eof_also_stops_them() {
    let first = Interactive::start(&binary(), &[]).unwrap();
    let first_address = ready(&first).await;
    let second = Interactive::start(&binary(), &[]).unwrap();
    let second_address = ready(&second).await;
    assert_ne!(first_address, second_address);
    // No explicit input: finish closes the owned input sender, supplying EOF.
    let (first_output, second_output) = tokio::join!(first.finish(), second.finish());
    for (output, address) in [
        (first_output, first_address),
        (second_output, second_address),
    ] {
        assert_stopped(&output, address);
        assert!(output.status.success(), "{output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
    }
}

#[tokio::test]
async fn unexpected_arguments_fail_before_reporting_or_starting_an_endpoint() {
    let output = Process::spawn(&binary(), &["--unexpected"])
        .unwrap()
        .finish()
        .await;
    assert!(!output.status.success());
    assert!(output.stdout.is_empty(), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("usage: shared-endpoint-guide"));
}

#[test]
fn executable_selection_is_explicit_without_missing_path_or_directory_fallback() {
    let expected = Path::new(env!("CARGO_BIN_EXE_guide-shared-endpoint"))
        .canonicalize()
        .unwrap();
    assert_eq!(select_binary(None).unwrap(), expected);
    assert_eq!(select_binary(Some(expected.as_os_str())).unwrap(), expected);
    assert!(select_binary(Some(expected.join("missing-child").as_os_str())).is_err());
    assert!(select_binary(Some(expected.parent().unwrap().as_os_str())).is_err());
}

#[tokio::test]
async fn an_existing_nonexecutable_override_fails_without_running_the_default() {
    let source = Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/shared_endpoint.rs"
    ));
    let selected = select_binary(Some(source.as_os_str())).unwrap();
    assert_eq!(selected, source.canonicalize().unwrap());
    assert!(Process::spawn(&selected, &[]).is_err());
}

#[tokio::test]
async fn dropped_cancelled_and_panicking_callers_reap_the_started_owner_and_release_its_port() {
    for mode in ["drop", "cancel", "panic"] {
        let child = Interactive::start(&binary(), &[]).unwrap();
        let address = ready(&child).await;
        let mut retired = child.retirement();
        match mode {
            "drop" => drop(child),
            "cancel" => {
                let (entered, ready) = tokio::sync::oneshot::channel();
                let task = tokio::spawn(async move {
                    let _child = child;
                    let _ = entered.send(());
                    std::future::pending::<()>().await;
                });
                ready.await.unwrap();
                task.abort();
                assert!(task.await.unwrap_err().is_cancelled());
            }
            _ => {
                let task = tokio::spawn(async move {
                    let _child = child;
                    panic!("controlled guide caller panic")
                });
                assert!(task.await.unwrap_err().is_panic());
            }
        }
        let completion = guide::retired(&mut retired).await;
        assert!(
            completion.cancelled,
            "owned retirement must request termination"
        );
        assert!(!completion.output.status.success(), "{completion:?}");
        assert!(completion.output.stderr.is_empty(), "{completion:?}");
        assert_rebind(address);
    }
}

#[test]
fn child_retirement_survives_shutdown_of_the_callers_runtime() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let (mut retired, address) = runtime.block_on(async {
        let child = Interactive::start(&binary(), &[]).unwrap();
        let address = ready(&child).await;
        let retired = child.retirement();
        let (entered, ready) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let _child = child;
            let _ = entered.send(());
            std::future::pending::<()>().await;
        });
        ready.await.unwrap();
        (retired, address)
    });
    drop(runtime);
    let observer = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let completion = observer.block_on(guide::retired(&mut retired));
    assert!(completion.cancelled);
    assert!(!completion.output.status.success());
    assert_rebind(address);
}

#[tokio::test]
async fn synchronous_test_failure_reaps_before_unwind_returns_control() {
    let child = Interactive::start(&binary(), &[]).unwrap();
    let address = ready(&child).await;
    let retired = child.retirement();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _child = child;
        panic!("controlled assertion failure")
    }));
    assert!(result.is_err());
    let completion = retired
        .borrow()
        .as_ref()
        .expect("Drop joins retirement")
        .clone()
        .unwrap();
    assert!(completion.cancelled);
    assert!(!completion.output.status.success());
    assert_rebind(address);
}
