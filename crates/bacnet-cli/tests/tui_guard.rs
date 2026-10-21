//! `bacnet tui` without a terminal: `bacnet tui > out.txt` must fail with a
//! hint on stderr and leave the output file empty. Plain `bacnet` there
//! prints the usage and exits 2 (#974). The support harness runs the binary
//! with stdin from /dev/null and stdout and stderr to files.
#[allow(dead_code)]
mod support;
use support::{command, failure, run, Process};

/// Plain `bacnet` in a pipeline or a script: the usage on stderr, nothing on
/// stdout and exit status 2, whatever opted out of the TUI.
#[tokio::test]
async fn plain_bacnet_without_a_terminal_prints_the_usage_and_exits_2() {
    for (args, no_tui_env) in [
        (&[][..], None),
        (&["--no-tui"][..], None),
        (&[][..], Some("1")),
        (&["-v", "--sc"][..], Some("0")),
    ] {
        let mut cmd = command();
        cmd.args(args).env_remove("BACNET_NO_TUI");
        if let Some(value) = no_tui_env {
            cmd.env("BACNET_NO_TUI", value);
        }
        let output = Process::spawn(&mut cmd).output().await;
        assert_eq!(output.status.code(), Some(2), "{args:?} {no_tui_env:?}");
        failure(&output, "Usage: bacnet [OPTIONS] [COMMAND]");
        failure(&output, "the terminal UI won't open");
    }
}

#[cfg(feature = "tui")]
#[tokio::test]
async fn tui_without_a_terminal_fails_with_a_hint_and_writes_nothing() {
    for args in [
        vec!["tui"],
        vec!["--json", "tui", "--fps", "5"],
        vec!["--sc", "tui"],
        vec!["tui", "--profile", "site"],
        vec!["tui", "--no-profile"],
    ] {
        let output = run(&args).await;
        failure(&output, "needs an interactive terminal");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("bacnet discover --json"), "{stderr}");
    }
}

#[cfg(not(feature = "tui"))]
#[tokio::test]
async fn tui_without_the_feature_preserves_its_rebuild_advice() {
    let output = run(["tui"]).await;
    failure(&output, "cargo install bacnet-cli --features tui");
}

#[tokio::test]
async fn tui_help_lists_its_flags() {
    let output = run(["tui", "--help"]).await;
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(
        text.contains("--fps <FPS>")
            && text.contains("--log-file <FILE>")
            && text.contains("--profile <NAME>")
            && text.contains("--no-profile"),
        "{text}"
    );
}
