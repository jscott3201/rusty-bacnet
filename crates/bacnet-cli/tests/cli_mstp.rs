//! The MS/TP one-shots as a user runs them (#969): flag errors, the port
//! list, and on Linux a read across a pseudo-terminal pair from a stack
//! server on the other end.
#[allow(dead_code)]
mod support;

/// A refused command line exits non-zero, writes nothing to stdout, and
/// says what it would have taken.
#[tokio::test]
async fn invalid_mstp_flags_are_refused_with_the_allowed_values() {
    for (args, diagnostic) in [
        (
            vec![
                "--mstp",
                "/dev/x",
                "--mstp-mac",
                "3",
                "--mstp-baud",
                "12345",
                "discover",
            ],
            "must be one of 9600, 19200, 38400, 57600, 76800 or 115200",
        ),
        (
            vec!["--mstp", "/dev/x", "--mstp-mac", "128", "discover"],
            "a master station's MAC must be 0 to 127",
        ),
        (
            vec![
                "--mstp",
                "/dev/x",
                "--mstp-mac",
                "1",
                "--mstp-max-master",
                "200",
                "discover",
            ],
            "Max_Master must be 0 to 127",
        ),
        (
            vec![
                "--mstp",
                "/dev/x",
                "--mstp-mac",
                "1",
                "--mstp-max-info-frames",
                "0",
                "discover",
            ],
            "must be 1 to 255",
        ),
        (
            vec!["--mstp", "/dev/x", "discover"],
            "--mstp needs --mstp-mac (0 to 127)",
        ),
        (
            vec![
                "--mstp",
                "/dev/x",
                "--mstp-mac",
                "9",
                "--mstp-max-master",
                "5",
                "discover",
            ],
            "above --mstp-max-master 5",
        ),
        (
            vec!["discover", "--mstp-mac", "9"],
            "--mstp-mac requires --mstp PORT",
        ),
        (
            vec![
                "--json",
                "--mstp",
                "/dev/x",
                "--mstp-mac",
                "1",
                "--sc",
                "discover",
            ],
            "cannot be used with",
        ),
        // Split across the subcommand, either way round.
        (
            vec!["--sc", "discover", "--mstp", "/dev/x", "--mstp-mac", "1"],
            "--sc cannot be used with --mstp",
        ),
        (
            vec!["--mstp", "/dev/x", "--mstp-mac", "1", "discover", "--ipv6"],
            "--ipv6 cannot be used with --mstp",
        ),
        (
            vec![
                "--mstp",
                "/dev/x",
                "--mstp-mac",
                "1",
                "discover",
                "--foreign-bbmd",
                "192.0.2.10",
            ],
            "--foreign-bbmd cannot be used with --mstp",
        ),
        (
            vec!["--mstp", "", "--mstp-mac", "1", "discover"],
            "--mstp needs a serial port name",
        ),
        // The monitor (#970) proposes the MAC: it takes no --mstp-mac,
        // before or after the subcommand.
        (
            vec!["--mstp-mac", "3", "tui", "--mstp-monitor", "/dev/x"],
            "--mstp-mac cannot be used with --mstp-monitor",
        ),
    ] {
        let output = support::run(&args).await;
        support::failure(&output, diagnostic);
        assert_eq!(output.status.code(), Some(2), "{args:?}");
    }
}

/// `mstp-ports --json` is an array on every OS, possibly empty; without
/// the feature it says how to get it.
#[tokio::test]
async fn mstp_ports_prints_a_json_array() {
    let output = support::run(["mstp-ports", "--json"]).await;
    if cfg!(feature = "serial") {
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let ports: Vec<String> = serde_json::from_slice(&output.stdout).unwrap();
        assert!(ports.iter().all(|port| !port.is_empty()), "{ports:?}");
    } else {
        support::failure(&output, "requires the 'serial' feature");
    }
}

// Linux only: on macOS the serial backend sets the baud rate with an
// ioctl a pseudo-terminal refuses (ENOTTY), so the CLI can't open one by
// name there; the transport's own pseudo-terminal tests use the pair's
// streams directly and run on macOS too.
#[cfg(all(target_os = "linux", feature = "serial"))]
mod pty {
    use std::time::Duration;

    use bacnet_objects::analog::AnalogInputObject;
    use bacnet_objects::database::ObjectDatabase;
    use bacnet_objects::device::{DeviceConfig, DeviceObject};
    use bacnet_objects::traits::BACnetObject;
    use bacnet_objects::DeviceIdentity;
    use bacnet_server::server::BACnetServer;
    use bacnet_transport::mstp::{MstpConfig, MstpTransport, SerialPort};
    use bacnet_types::enums::EngineeringUnits;
    use bacnet_types::error::Error;
    use serde_json::{json, Value};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::sync::Mutex;
    use tokio_serial::{SerialPort as _, SerialStream};

    use super::support;

    /// Generous: a loaded CI host can be slow to start a process and to
    /// pass octets through a pseudo-terminal (#1811).
    const DEADLINE: Duration = Duration::from_secs(60);

    /// The server's end of the pair. The transport reads and writes from
    /// one task, never both at once, so one lock is enough.
    struct Line(Mutex<SerialStream>);

    impl SerialPort for Line {
        async fn write(&self, data: &[u8]) -> Result<(), Error> {
            self.0
                .lock()
                .await
                .write_all(data)
                .await
                .map_err(Error::Transport)
        }

        async fn read(&self, buf: &mut [u8]) -> Result<usize, Error> {
            self.0
                .lock()
                .await
                .read(buf)
                .await
                .map_err(Error::Transport)
        }
    }

    fn database() -> (DeviceIdentity, ObjectDatabase) {
        let identity = DeviceIdentity::new(4001, 260)
            .and_then(|identity| identity.with_max_apdu(480))
            .unwrap();
        let mut device = DeviceObject::new(DeviceConfig {
            identity: identity.clone(),
            ..Default::default()
        })
        .unwrap();
        let mut ai2 =
            AnalogInputObject::new(2, "Outside Air", EngineeringUnits::DEGREES_FAHRENHEIT).unwrap();
        ai2.set_present_value(51.0);
        device.set_object_list(vec![device.object_identifier(), ai2.object_identifier()]);
        let mut db = ObjectDatabase::new();
        db.add(Box::new(device)).unwrap();
        db.add(Box::new(ai2)).unwrap();
        (identity, db)
    }

    /// `bacnet --mstp <pty> read mstp:1 ai:2 pv` joins the trunk as station
    /// 3 and prints the documented JSON from the server at station 1.
    #[tokio::test]
    async fn a_read_over_a_pseudo_terminal_prints_the_documented_json() {
        let (server_end, cli_end) = SerialStream::pair().unwrap();
        let path = cli_end.name().expect("the pseudo-terminal's path");
        let (identity, database) = database();
        let config = MstpConfig {
            this_station: 1,
            max_master: 3,
            baud_rate: 38_400,
            ..MstpConfig::default()
        };
        let mut server = BACnetServer::generic_builder()
            .device(identity)
            .database(database)
            .transport(MstpTransport::new(Line(Mutex::new(server_end)), config))
            .build()
            .await
            .unwrap();
        let mut command = support::command();
        command.args([
            "--mstp",
            &path,
            "--mstp-mac",
            "3",
            "--mstp-max-master",
            "3",
            "--json",
            "read",
            "mstp:1",
            "ai:2",
            "pv",
        ]);
        // This test keeps `cli_end` open while the child runs: on Linux the
        // server's end fails with EIO whenever no process holds the other
        // end, between the pair's creation and the child's open included.
        let child = support::Process::spawn(&mut command);
        let output = tokio::time::timeout(DEADLINE, child.output_unbounded())
            .await
            .expect("the CLI finished");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "{stderr}");
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(
            value,
            json!({ "object": "ANALOG_INPUT:2", "property": "PRESENT_VALUE", "value": "51" })
        );
        server.stop().await.unwrap();
        drop(cli_end);
    }
}
