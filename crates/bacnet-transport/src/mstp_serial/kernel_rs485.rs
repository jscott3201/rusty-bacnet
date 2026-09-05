use bacnet_types::error::Error;

const SER_RS485_ENABLED: u32 = 1;
const SER_RS485_RTS_ON_SEND: u32 = 1 << 1;
const SER_RS485_RTS_AFTER_SEND: u32 = 1 << 2;

// Linux UAPI serial_rs485; addressing features are unused and left zeroed.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct SerialRs485 {
    flags: u32,
    delay_rts_before_send: u32,
    delay_rts_after_send: u32,
    padding: [u32; 5],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Request {
    Set,
    Get,
}

pub(super) fn configure(
    invert_rts: bool,
    delay_before_send_us: u32,
    delay_after_send_us: u32,
    mut ioctl: impl FnMut(Request, &mut SerialRs485) -> std::io::Result<()>,
) -> Result<(), Error> {
    if !delay_before_send_us.is_multiple_of(1_000) || !delay_after_send_us.is_multiple_of(1_000) {
        return Err(Error::Encoding(format!(
            "Kernel RS-485 delays must be multiples of 1000 microseconds: \
             delay_before_send_us={delay_before_send_us}, delay_after_send_us={delay_after_send_us}"
        )));
    }

    let requested = SerialRs485 {
        flags: SER_RS485_ENABLED
            | if invert_rts {
                SER_RS485_RTS_AFTER_SEND
            } else {
                SER_RS485_RTS_ON_SEND
            },
        delay_rts_before_send: delay_before_send_us / 1_000,
        delay_rts_after_send: delay_after_send_us / 1_000,
        ..SerialRs485::default()
    };
    // The set ioctl may replace the input with sanitized settings. Keep the
    // original request separately so that sanitization cannot hide a mismatch.
    let mut config = requested;
    ioctl(Request::Set, &mut config)
        .map_err(|e| Error::Encoding(format!("TIOCSRS485 ioctl failed: {e}")))?;
    let mut effective = SerialRs485::default();
    ioctl(Request::Get, &mut effective).map_err(|e| {
        Error::Encoding(format!(
            "TIOCGRS485 ioctl failed after setting RS-485: {e}; \
             hardware configuration may have changed; no rollback was attempted"
        ))
    })?;
    let required_flags = SER_RS485_ENABLED | SER_RS485_RTS_ON_SEND | SER_RS485_RTS_AFTER_SEND;
    if effective.flags & required_flags != requested.flags
        || effective.delay_rts_before_send != requested.delay_rts_before_send
        || effective.delay_rts_after_send != requested.delay_rts_after_send
    {
        return Err(Error::Encoding(format!(
            "RS-485 readback mismatch: effective flags={:#010x}, \
             delay_before_send_ms={}, delay_after_send_ms={}; \
             hardware configuration may have changed; no rollback was attempted",
            effective.flags, effective.delay_rts_before_send, effective.delay_rts_after_send
        )));
    }
    tracing::info!(
        flags = effective.flags,
        delay_before_send_ms = effective.delay_rts_before_send,
        delay_after_send_ms = effective.delay_rts_after_send,
        "Kernel RS-485 mode enabled"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_fractional_milliseconds_before_any_ioctl() {
        for (before_us, after_us) in [
            (1, 0),
            (999, 0),
            (1_001, 0),
            (0, 1),
            (0, 999),
            (0, 1_001),
            (u32::MAX, 0),
            (0, u32::MAX),
        ] {
            let error = configure(false, before_us, after_us, |_, _| {
                panic!("invalid delays must not reach the ioctl boundary")
            })
            .unwrap_err();
            assert!(error.to_string().contains("multiples of 1000 microseconds"));
        }
    }

    #[test]
    fn requests_kernel_delays_in_milliseconds() {
        let mut accepted = SerialRs485::default();
        configure(false, 1_000, 2_000, |request, config| {
            match request {
                Request::Set => {
                    assert_eq!(config.flags, 0b011);
                    assert_eq!(config.delay_rts_before_send, 1);
                    assert_eq!(config.delay_rts_after_send, 2);
                    assert_eq!(config.padding, [0; 5]);
                    accepted = *config;
                }
                Request::Get => *config = accepted,
            }
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn reads_effective_settings_after_setting_them() {
        let mut requests = Vec::new();
        let mut accepted = SerialRs485::default();
        configure(true, 0, 0, |request, config| {
            requests.push(request);
            match request {
                Request::Set => {
                    assert_eq!(config.flags, 0b101);
                    assert_eq!(config.delay_rts_before_send, 0);
                    assert_eq!(config.delay_rts_after_send, 0);
                    accepted = *config;
                }
                Request::Get => {
                    assert_eq!(*config, SerialRs485::default());
                    *config = accepted;
                }
            }
            Ok(())
        })
        .unwrap();
        assert_eq!(requests, [Request::Set, Request::Get]);
    }

    #[test]
    fn rejects_sanitized_flags_or_delays_without_retry_or_rollback() {
        for (invert_rts, flags, before_ms, after_ms) in [
            (false, 0b010, 1, 2), // RS-485 disabled
            (false, 0b001, 1, 2), // RTS on send cleared
            (false, 0b111, 1, 2), // RTS after send set
            (true, 0b100, 1, 2),  // RS-485 disabled
            (true, 0b111, 1, 2),  // RTS on send set
            (true, 0b001, 1, 2),  // RTS after send cleared
            (false, 0b011, 0, 2), // before-send delay clamped
            (false, 0b011, 1, 0), // after-send delay clamped
        ] {
            let mut requests = Vec::new();
            let error = configure(invert_rts, 1_000, 2_000, |request, config| {
                requests.push(request);
                // TIOCSRS485 can overwrite its input with sanitized settings.
                *config = SerialRs485 {
                    flags,
                    delay_rts_before_send: before_ms,
                    delay_rts_after_send: after_ms,
                    ..SerialRs485::default()
                };
                Ok(())
            })
            .unwrap_err();
            let message = error.to_string();
            assert!(message.contains("RS-485 readback mismatch"));
            assert!(message.contains(&format!("flags={flags:#010x}")));
            assert!(message.contains(&format!("delay_before_send_ms={before_ms}")));
            assert!(message.contains(&format!("delay_after_send_ms={after_ms}")));
            assert!(message.contains("hardware configuration may have changed"));
            assert!(message.contains("no rollback was attempted"));
            assert_eq!(requests, [Request::Set, Request::Get]);
        }
    }

    #[test]
    fn propagates_set_failure_without_readback_or_retry() {
        let mut requests = Vec::new();
        let error = configure(false, 0, 0, |request, _| {
            requests.push(request);
            Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "set rejected",
            ))
        })
        .unwrap_err();
        assert!(matches!(&error, Error::Encoding(_)));
        assert!(error.to_string().contains("TIOCSRS485 ioctl failed"));
        assert!(error.to_string().contains("set rejected"));
        assert_eq!(requests, [Request::Set]);
    }

    #[test]
    fn propagates_readback_failure_without_retry_or_rollback() {
        let mut requests = Vec::new();
        let error = configure(false, 0, 0, |request, _| {
            requests.push(request);
            match request {
                Request::Set => Ok(()),
                Request::Get => Err(std::io::Error::new(
                    std::io::ErrorKind::Unsupported,
                    "readback unavailable",
                )),
            }
        })
        .unwrap_err();
        let message = error.to_string();
        assert!(message.contains("TIOCGRS485 ioctl failed"));
        assert!(message.contains("readback unavailable"));
        assert!(message.contains("hardware configuration may have changed"));
        assert!(message.contains("no rollback was attempted"));
        assert_eq!(requests, [Request::Set, Request::Get]);
    }

    #[test]
    fn accepts_exact_boundary_delays_and_preserves_both_rts_polarities() {
        for (before_us, after_us, before_ms, after_ms) in [
            (0, 0, 0, 0),
            (0, 4_294_967_000, 0, 4_294_967),
            (4_294_967_000, 0, 4_294_967, 0),
        ] {
            for (invert_rts, flags) in [(false, 0b011), (true, 0b101)] {
                let expected = SerialRs485 {
                    flags,
                    delay_rts_before_send: before_ms,
                    delay_rts_after_send: after_ms,
                    padding: [0; 5],
                };
                configure(invert_rts, before_us, after_us, |request, config| {
                    match request {
                        Request::Set => assert_eq!(*config, expected),
                        Request::Get => *config = expected,
                    }
                    Ok(())
                })
                .unwrap();
            }
        }
    }

    #[test]
    fn allows_unrelated_readback_flags() {
        let mut accepted = SerialRs485::default();
        configure(false, 0, 0, |request, config| {
            match request {
                Request::Set => accepted = *config,
                Request::Get => {
                    *config = accepted;
                    config.flags |= 1 << 5; // Driver-reported bus termination.
                }
            }
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn linux_serial_rs485_layout_matches_uapi() {
        use std::mem::{align_of, offset_of, size_of};

        assert_eq!(size_of::<SerialRs485>(), 32);
        assert_eq!(align_of::<SerialRs485>(), 4);
        assert_eq!(offset_of!(SerialRs485, flags), 0);
        assert_eq!(offset_of!(SerialRs485, delay_rts_before_send), 4);
        assert_eq!(offset_of!(SerialRs485, delay_rts_after_send), 8);
        assert_eq!(offset_of!(SerialRs485, padding), 12);
        assert_eq!(size_of::<[u32; 5]>(), 20);
    }
}
