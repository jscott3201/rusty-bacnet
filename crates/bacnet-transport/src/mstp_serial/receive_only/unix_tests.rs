use super::*;
#[cfg(target_os = "linux")]
use crate::mstp::monitor::{MonitorConfig, MstpMonitor};
use crate::sys::serial_receive_probe::{assert_readonly, denied_write, speed};
#[cfg(target_os = "linux")]
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_serial::{SerialPort, SerialStream};

fn pair() -> (SerialStream, SerialConfig) {
    let (master, slave) = SerialStream::pair().unwrap();
    let config = SerialConfig {
        port_name: slave.name().unwrap(),
        baud_rate: 38_400,
    };
    drop(slave);
    (master, config)
}
#[tokio::test]
async fn production_open_is_readonly_raw_exclusive_and_awaited_close_reopens() {
    let (mut master, config) = pair();
    let mut input = ReceiveOnlyPort::open(&config, None).unwrap();
    let fd = input.input.as_ref().unwrap().get_ref();
    assert_readonly(fd);
    denied_write(fd);
    assert_eq!(speed(fd), config.baud_rate);
    assert!(ReceiveOnlyPort::open(&config, None).is_err());
    let bytes = [0, b'\r', b'\n', 0x11, 0x13, 0x7f, 0x80, 0xff];
    master.write_all(&bytes).await.unwrap();
    let mut received = [0; 8];
    input.read_exact(&mut received).await.unwrap();
    assert_eq!(received, bytes);
    assert_eq!(master.bytes_to_read().unwrap(), 0);
    input.close().await.unwrap();
    input.close().await.unwrap();
    ReceiveOnlyPort::open(&config, None)
        .unwrap()
        .close()
        .await
        .unwrap();
}
#[tokio::test]
async fn input_is_purged_before_open_returns() {
    let (mut master, config) = pair();
    // Keep a setup slave open to make preloading deterministic across PTYs.
    let setup = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_NOCTTY)
        .open(&config.port_name)
        .unwrap();
    master.write_all(b"old\n").await.unwrap();
    // Readiness acknowledges actual queued input; no sleep or flush assumption.
    crate::sys::serial_receive_probe::readable(&setup)
        .await
        .unwrap();
    assert_eq!(crate::sys::serial_receive_probe::queued(&setup), 4);
    drop(setup);
    let mut input = ReceiveOnlyPort::open(&config, None).unwrap();
    master.write_all(b"new").await.unwrap();
    let mut bytes = [0; 3];
    input.read_exact(&mut bytes).await.unwrap();
    assert_eq!(&bytes, b"new");
    input.close().await.unwrap();
}
#[tokio::test]
async fn unsupported_or_custom_rate_is_truthful() {
    let (_master, mut config) = pair();
    config.baud_rate = 0;
    assert!(
        ReceiveOnlyPort::open(&config, None)
            .unwrap_err()
            .release_confirmed()
    );
    config.baud_rate = 76_800;
    #[cfg(target_os = "linux")]
    {
        let mut input = ReceiveOnlyPort::open(&config, None).unwrap();
        assert_eq!(speed(input.input.as_ref().unwrap().get_ref()), 76_800);
        input.close().await.unwrap();
    }
    #[cfg(target_vendor = "apple")]
    {
        let error = ReceiveOnlyPort::open(&config, None).unwrap_err();
        assert!(
            error.release_confirmed(),
            "failed configuration released the acquired descriptor"
        );
        config.baud_rate = 38_400;
        ReceiveOnlyPort::open(&config, None)
            .unwrap()
            .close()
            .await
            .unwrap();
    }
}
#[cfg(target_os = "linux")]
#[tokio::test]
async fn production_open_trace_phases() {
    use std::io::Write;
    let (mut master, config) = pair();
    let path = &config.port_name;
    let mut control = std::fs::OpenOptions::new().write(true).open(path).unwrap();
    trace(format!(
        "RB_TRACE phase=control path={path} fd={}",
        control.as_raw_fd()
    ));
    control.write_all(b"allowed").unwrap();
    drop(control);
    let mut probe = ReceiveOnlyPort::open(&config, None).unwrap();
    let fd = probe.input.as_ref().unwrap().get_ref();
    trace(format!(
        "RB_TRACE phase=probe path={path} fd={}",
        fd.as_raw_fd()
    ));
    denied_write(fd);
    probe.close().await.unwrap();
    trace(format!("RB_TRACE phase=measure_begin path={path} fd=-1"));
    let input = ReceiveOnlyPort::open(&config, None).unwrap();
    let fd = input.input.as_ref().unwrap().get_ref().as_raw_fd();
    trace(format!("RB_TRACE phase=measure_open path={path} fd={fd}"));
    let mut monitor = MstpMonitor::start(input, MonitorConfig::new(config.baud_rate))
        .await
        .unwrap();
    let mut stats = monitor.statistics();
    let mut wire = [0; 8];
    crate::mstp::codec::Frame::new(crate::mstp::codec::FrameType::TOKEN, 2, 1, &[])
        .encode_into(&mut wire)
        .unwrap();
    AsyncWriteExt::write_all(&mut master, &wire).await.unwrap();
    stats
        .wait_for(|stats| stats.token_passes == 1)
        .await
        .unwrap();
    monitor.stop().await.unwrap();
    trace(format!("RB_TRACE phase=measure_end path={path} fd={fd}"));
    let mut reopened = ReceiveOnlyPort::open(&config, None).unwrap();
    trace(format!(
        "RB_TRACE phase=reopen path={path} fd={}",
        reopened.input.as_ref().unwrap().get_ref().as_raw_fd()
    ));
    reopened.close().await.unwrap();
}

#[cfg(target_os = "linux")]
fn trace(marker: String) {
    use std::io::Write;
    let bytes = format!("{marker}\n");
    std::io::stderr()
        .lock()
        .write_all(bytes.as_bytes())
        .unwrap();
}

async fn inherited_hupcl(set: bool) {
    use crate::sys::serial_receive_probe::hupcl;
    let (mut master, config) = pair();
    let setup = std::fs::File::open(&config.port_name).unwrap();
    assert_eq!(hupcl(&setup, Some(set)), set);
    // Keep the setup descriptor alive: macOS resets PTY termios when its
    // final slave descriptor closes. This controls inherited state at open.
    assert_eq!(hupcl(&setup, None), set);
    let mut input = ReceiveOnlyPort::open(&config, None).unwrap();
    assert_eq!(hupcl(input.input.as_ref().unwrap().get_ref(), None), set);
    master.write_all(b"raw").await.unwrap();
    let mut bytes = [0; 3];
    input.read_exact(&mut bytes).await.unwrap();
    assert_eq!(&bytes, b"raw");
    input.close().await.unwrap();
    let mut reopened = ReceiveOnlyPort::open(&config, None).unwrap();
    assert_eq!(hupcl(reopened.input.as_ref().unwrap().get_ref(), None), set);
    reopened.close().await.unwrap();
}
#[tokio::test]
async fn inherited_hupcl_set_survives_open_read_close_and_reopen() {
    inherited_hupcl(true).await;
}
#[tokio::test]
async fn inherited_hupcl_unset_survives_open_read_close_and_reopen() {
    inherited_hupcl(false).await;
}
