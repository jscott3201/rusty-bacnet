//! Focused command projection and fallible-writer contracts.
//! Failure injection at the command's writer boundary and typed error projection.
use super::*;
use bacnet_transport::sc_tls::inspection::*;
use clap::Parser;

struct FailingWriter {
    flush_only: bool,
}
impl Write for FailingWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.flush_only {
            Ok(bytes.len())
        } else {
            Err(io::ErrorKind::BrokenPipe.into())
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        Err(io::ErrorKind::BrokenPipe.into())
    }
}

#[test]
fn write_and_flush_failures_override_completed_success_or_errors() {
    let directory = std::env::temp_dir().join(format!("sc-check-writer-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }
    let _cleanup = Cleanup(directory.clone());
    let path = directory.join("cert.pem");
    let pair = rcgen::KeyPair::generate().unwrap();
    let params = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
    let valid = params.self_signed(&pair).unwrap().pem();
    for (pem, expected) in [
        (valid.as_str(), 0),
        (
            "-----BEGIN CERTIFICATE-----\nYWJj\n-----END CERTIFICATE-----\n",
            1,
        ),
    ] {
        std::fs::write(&path, pem).unwrap();
        let path = path.clone();
        let cli = crate::parse_on_large_stack(move || {
            Cli::parse_from([
                std::ffi::OsString::from("bacnet"),
                "sc-check".into(),
                "--sc-cert".into(),
                path.into_os_string(),
            ])
        });
        for format in [OutputFormat::Json, OutputFormat::Table] {
            assert_eq!(
                execute(
                    &cli,
                    Some(1_704_067_200),
                    format,
                    &mut Vec::new(),
                    &mut Vec::new()
                ),
                expected
            );
            for flush_only in [false, true] {
                let mut stderr = Vec::new();
                assert_eq!(
                    execute(
                        &cli,
                        Some(1_704_067_200),
                        format,
                        &mut FailingWriter { flush_only },
                        &mut stderr
                    ),
                    3
                );
                assert!(
                    String::from_utf8(stderr)
                        .unwrap()
                        .contains("could not write or flush")
                );
            }
        }
    }
}

#[test]
fn execution_error_json_write_failure_is_safe_even_when_stderr_also_fails() {
    let error = projection::input_error(ScInspectionError::Input {
        role: ScInputRole::Identity,
        index: None,
        kind: ScInputErrorKind::ClockOutOfRange,
    });
    assert_eq!(
        execution_error(
            error,
            OutputFormat::Json,
            &mut FailingWriter { flush_only: true },
            &mut FailingWriter { flush_only: false }
        ),
        3
    );
}

#[test]
fn io_projection_discards_untrusted_source_text_and_keeps_native_error_code() {
    let unsafe_error = projection::input_error(ScInspectionError::Io {
        role: ScInputRole::PrivateKey,
        source: io::Error::other("private path\n\x1bsecret PEM"),
    });
    let mut output = Vec::new();
    assert_eq!(
        execution_error(
            unsafe_error,
            OutputFormat::Json,
            &mut output,
            &mut Vec::new()
        ),
        3
    );
    let text = String::from_utf8(output).unwrap();
    assert!(!text.contains("secret") && !text.contains("private path"));
    let native = projection::input_error(ScInspectionError::Io {
        role: ScInputRole::PrivateKey,
        source: io::Error::from_raw_os_error(2),
    });
    assert_eq!(native.os_code, Some(2));
    assert_eq!(native.role, Some("private_key"));
}

#[test]
fn signed_time_extremes_do_not_overflow_calendar_or_duration_formatting() {
    assert!(render::date(i64::MIN).contains("Unix -9223372036854775808"));
    assert!(render::date(i64::MAX).contains("Unix 9223372036854775807"));
    assert_eq!(render::date(0), "1970-01-01T00:00:00Z (Unix 0)");
    assert_eq!(
        render::distance(i64::MAX, i64::MIN),
        "213503982334601 days 07:00:15 remaining until expiry"
    );
    assert_eq!(
        render::distance(i64::MIN, i64::MAX),
        "213503982334601 days 07:00:15 elapsed since expiry"
    );
}
