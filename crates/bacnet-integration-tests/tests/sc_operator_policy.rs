//! Run the security guide's policy program (#1989). Only with `sc-tls`,
//! which the program's certificate accessors need.
#![cfg(feature = "sc-tls")]
#![forbid(unsafe_code)]

use std::process::{Command, Output};

fn run(args: &[&str], password: Option<&str>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_guide-sc-operator-policy"));
    command.args(args).env_remove("BACNET_DCC_PASSWORD");
    if let Some(password) = password {
        command.env("BACNET_DCC_PASSWORD", password);
    }
    command.output().expect("run the guide program")
}

#[test]
fn denies_a_request_without_a_certificate() {
    let output = run(&["operator.example.com"], Some("example-password"));
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "DCC without a certificate: denied\n"
    );
}

#[test]
fn refuses_to_start_without_a_name_or_a_password() {
    for (args, password) in [
        (&[][..], Some("example-password")),
        (&["operator.example.com"][..], None),
        (&["operator.example.com"][..], Some("")),
        (&["a", "b"][..], Some("example-password")),
    ] {
        let output = run(args, password);
        assert!(!output.status.success(), "{args:?} {password:?}");
        assert!(output.stdout.is_empty());
        assert_eq!(output.status.code(), Some(2), "{args:?} {password:?}");
        assert_eq!(
            String::from_utf8_lossy(&output.stderr),
            "usage: BACNET_DCC_PASSWORD=... sc-operator-policy <operator-dns-name>\n"
        );
    }
}
