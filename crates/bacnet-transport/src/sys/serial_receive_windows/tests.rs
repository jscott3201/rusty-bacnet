use super::*;

#[test]
fn native_readonly_access_denies_write_and_release_allows_immediate_reopen() {
    use std::io::Write;
    use windows_sys::Win32::Storage::FileSystem::WriteFile;
    let path = std::env::temp_dir().join(format!("bacnet-readonly-{}.bin", std::process::id()));
    let mut control = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&path)
        .unwrap();
    control.write_all(b"allowed control").unwrap();
    drop(control);
    let handle = open_readonly(path.to_str().unwrap()).unwrap();
    assert!(
        open_readonly(path.to_str().unwrap()).is_err(),
        "exclusive sharing"
    );
    let mut written = 0;
    // SAFETY: test-only denied-write probe with valid handle/buffer/output.
    let result = unsafe {
        WriteFile(
            handle.as_raw_handle(),
            b"probe".as_ptr(),
            5,
            &mut written,
            std::ptr::null_mut(),
        )
    };
    assert_eq!(result, 0);
    assert_eq!(io::Error::last_os_error().raw_os_error(), Some(5));
    assert_eq!(written, 0);
    let mut input = Input {
        handle: Some(handle),
    };
    input.close().unwrap();
    drop(open_readonly(path.to_str().unwrap()).unwrap());
    // A regular file is acquired by the production open, then rejected by
    // GetCommState. Its successful cleanup must remain safely retryable.
    let error = Input::open(path.to_str().unwrap(), 38_400, None).unwrap_err();
    assert!(error.release_confirmed());
    assert!(error.to_string().contains("GetCommState"), "{error}");
    drop(open_readonly(path.to_str().unwrap()).unwrap());
    std::fs::remove_file(path).unwrap();
}

#[derive(Default)]
struct Fake {
    state: DCB,
    calls: Vec<&'static str>,
    fail: Option<&'static str>,
    wrong_readback: bool,
}
impl Fake {
    fn record(&mut self, phase: &'static str) -> io::Result<()> {
        self.calls.push(phase);
        if self.fail == Some(phase) {
            Err(io::Error::other(phase))
        } else {
            Ok(())
        }
    }
}
impl Comm for Fake {
    fn state(&mut self) -> io::Result<DCB> {
        self.record("get")?;
        let mut state = self.state;
        if self.wrong_readback && self.calls.contains(&"set") {
            state.BaudRate += 1;
        }
        Ok(state)
    }
    fn set_state(&mut self, state: &DCB) -> io::Result<()> {
        self.record("set")?;
        self.state = *state;
        Ok(())
    }
    fn timeouts(&mut self, t: &COMMTIMEOUTS) -> io::Result<()> {
        assert_eq!(
            (
                t.ReadIntervalTimeout,
                t.ReadTotalTimeoutMultiplier,
                t.ReadTotalTimeoutConstant
            ),
            (u32::MAX, u32::MAX, 100)
        );
        self.record("timeouts")
    }
    fn rts(&mut self, level: bool) -> io::Result<()> {
        assert!(level);
        self.record("rts")
    }
    fn purge(&mut self) -> io::Result<()> {
        self.record("purge")
    }
}
#[test]
fn raw_dcb_clears_inherited_filtering_and_purges_after_rts() {
    let mut api = Fake {
        state: DCB {
            _bitfield: u32::MAX,
            ..Default::default()
        },
        ..Default::default()
    };
    configure(&mut api, 76_800, Some(true)).unwrap();
    assert_eq!(api.calls, ["get", "set", "get", "timeouts", "rts", "purge"]);
    assert_eq!(api.state._bitfield, 1 | (1 << 12));
    assert_eq!(api.state.DCBlength, std::mem::size_of::<DCB>() as u32);
    assert_eq!(
        (api.state.ByteSize, api.state.Parity, api.state.StopBits),
        (8, NOPARITY, ONESTOPBIT)
    );
    assert_eq!(api.state.BaudRate, 76_800);
    configure(&mut api, 38_400, None).unwrap();
    assert_eq!(api.state._bitfield, 1);
}
#[test]
fn every_configuration_failure_is_preserved() {
    for phase in ["get", "set", "timeouts", "rts", "purge"] {
        let mut api = Fake {
            fail: Some(phase),
            ..Default::default()
        };
        assert_eq!(
            configure(&mut api, 38_400, Some(true))
                .unwrap_err()
                .to_string(),
            phase
        );
        assert_eq!(api.calls.last(), Some(&phase));
    }
    assert!(
        Input::open("invalid\0path", 38_400, None)
            .unwrap_err()
            .release_confirmed()
    );
}

#[test]
fn a_driver_that_substitutes_a_rate_is_rejected_before_purge() {
    let mut api = Fake {
        wrong_readback: true,
        ..Default::default()
    };
    assert!(
        configure(&mut api, 38_400, None)
            .unwrap_err()
            .to_string()
            .contains("did not retain")
    );
    assert_eq!(api.calls, ["get", "set", "get"]);
}
