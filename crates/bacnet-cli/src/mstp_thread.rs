//! `--mstp-rt-priority` and `--mstp-cpus` (#501): how the thread that runs
//! this station's token handling is scheduled. They are settings of the
//! host, not of a trunk, so they come from the command line for every
//! station the process joins as (`--mstp`, a profile, or a join from the
//! monitor) and are not saved in profiles.
//!
//! Both are Linux only, and refused as they are parsed elsewhere. A setting
//! the kernel won't apply fails the join with the reason.

use bacnet_transport::mstp::{MAX_CPUS, MstpExecutionMode, MstpThreadConfig};

/// The flags' clap ids.
pub(crate) const FLAGS: [&str; 2] = ["mstp_rt_priority", "mstp_cpus"];

/// `--mstp-rt-priority`: a `SCHED_FIFO` priority, 1 to 99, on Linux.
pub(crate) fn parse_rt_priority(text: &str) -> Result<u8, String> {
    linux_only("--mstp-rt-priority")?;
    let priority = text.trim().parse::<u8>().ok();
    let config = priority.map(|p| MstpThreadConfig::new().with_realtime_priority(p));
    match (priority, config.map(|config| config.validate())) {
        (Some(priority), Some(Ok(()))) => Ok(priority),
        _ => Err("must be 1 to 99".into()),
    }
}

/// One CPU of `--mstp-cpus`, numbered from 0 as Linux numbers them.
pub(crate) fn parse_cpu(text: &str) -> Result<usize, String> {
    linux_only("--mstp-cpus")?;
    let cpu = text
        .trim()
        .parse::<usize>()
        .map_err(|_| "must be CPU numbers, such as 2 or 2,3".to_string())?;
    if cpu >= MAX_CPUS {
        return Err(format!(
            "CPU {cpu}: CPUs are numbered 0 to {}",
            MAX_CPUS - 1
        ));
    }
    Ok(cpu)
}

/// Refuse `flag` off Linux, naming it.
fn linux_only(flag: &str) -> Result<(), String> {
    if cfg!(target_os = "linux") {
        Ok(())
    } else {
        Err(format!(
            "{flag} is Linux only, and this is {}",
            std::env::consts::OS
        ))
    }
}

/// The flags' settings.
#[cfg_attr(not(any(feature = "serial", feature = "tui")), allow(dead_code))]
pub(crate) fn from_cli(cli: &crate::args::Cli) -> MstpThreadConfig {
    let mut config = MstpThreadConfig::new().with_cpu_affinity(cli.mstp_cpus.iter().copied());
    if let Some(priority) = cli.mstp_rt_priority {
        config = config.with_realtime_priority(priority);
    }
    config
}

/// Whether either flag was given.
#[cfg_attr(not(feature = "tui"), allow(dead_code))]
pub(crate) fn given(cli: &crate::args::Cli) -> bool {
    cli.mstp_rt_priority.is_some() || !cli.mstp_cpus.is_empty()
}

/// Log, once per process, that the flags were given to a session whose
/// transport (`transport`) has no MS/TP station to schedule.
#[cfg_attr(not(feature = "tui"), allow(dead_code))]
pub(crate) fn warn_unused(transport: &str) {
    static WARNED: std::sync::Once = std::sync::Once::new();
    WARNED.call_once(|| {
        tracing::warn!(
            "--mstp-rt-priority and --mstp-cpus apply only to an MS/TP station; \
             this session's transport is {transport}"
        );
    });
}

/// How a started transport's loop runs, for the MS/TP screen: as
/// [`describe`] for a thread of its own, from what the transport read back.
#[cfg_attr(not(feature = "tui"), allow(dead_code))]
pub(crate) fn status(status: Option<&bacnet_transport::mstp::MstpExecutionStatus>) -> String {
    match status {
        Some(status) if status.dedicated_thread => {
            describe(status.realtime_priority, &status.cpu_affinity)
        }
        Some(_) => "the application's runtime".into(),
        None => "not reported".into(),
    }
}

/// How a station's thread runs, for the MS/TP screen, the join
/// confirmation and the join's audit record: `FIFO 40, CPUs 3` or
/// `normal`.
#[cfg_attr(not(feature = "tui"), allow(dead_code))]
pub(crate) fn describe(realtime_priority: Option<u8>, cpus: &[usize]) -> String {
    let mut parts = Vec::new();
    if let Some(priority) = realtime_priority {
        parts.push(format!("FIFO {priority}"));
    }
    if !cpus.is_empty() {
        let list: Vec<String> = cpus.iter().map(usize::to_string).collect();
        let noun = if cpus.len() == 1 { "CPU" } else { "CPUs" };
        parts.push(format!("{noun} {}", list.join(",")));
    }
    if parts.is_empty() {
        "normal".into()
    } else {
        parts.join(", ")
    }
}

/// The CLI runs a station's loop on a thread of its own, so a busy
/// application runtime doesn't delay token handling; `thread` says how
/// that thread is scheduled.
#[cfg_attr(not(any(feature = "serial", feature = "tui")), allow(dead_code))]
pub(crate) fn mode(thread: &MstpThreadConfig) -> MstpExecutionMode {
    MstpExecutionMode::DedicatedThread(thread.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Parse a command line on a large stack, as `main` does (#953).
    fn parse(args: Vec<&'static str>) -> Result<crate::args::Cli, clap::Error> {
        crate::parse_on_large_stack(move || {
            use clap::{CommandFactory, FromArgMatches};
            let mut matches = crate::args::Cli::command().try_get_matches_from(args)?;
            crate::args::capture_sources(&matches)?;
            crate::args::Cli::from_arg_matches_mut(&mut matches)
        })
    }

    /// A one-shot command takes the flags only with `--mstp`; the terminal
    /// UI takes them alone, for a station it joins later.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_flags_need_a_station_to_schedule() {
        let error = parse(vec!["bacnet", "discover", "--mstp-cpus", "0"]).unwrap_err();
        let error = error.to_string();
        assert!(
            error.contains("--mstp-cpus requires --mstp PORT"),
            "{error}"
        );
        for args in [
            vec![
                "bacnet",
                "tui",
                "--mstp-rt-priority",
                "50",
                "--mstp-cpus",
                "1,0",
            ],
            vec!["bacnet", "--mstp-rt-priority", "50", "--mstp-cpus", "1,0"],
            vec![
                "bacnet",
                "--mstp",
                "/dev/ttyUSB0",
                "--mstp-mac",
                "3",
                "discover",
                "--mstp-rt-priority",
                "50",
                "--mstp-cpus",
                "1,0",
            ],
        ] {
            let cli = parse(args.clone()).unwrap();
            let thread = from_cli(&cli);
            assert_eq!(thread.realtime_priority, Some(50), "{args:?}");
            assert_eq!(thread.cpu_affinity, [1, 0], "{args:?}");
            assert_eq!(mode(&thread), MstpExecutionMode::DedicatedThread(thread));
        }
        let cli = parse(vec!["bacnet", "tui"]).unwrap();
        assert_eq!(from_cli(&cli), MstpThreadConfig::new());
    }

    /// Elsewhere the flags fail as they are parsed, with the reason.
    #[cfg(not(target_os = "linux"))]
    #[test]
    fn the_flags_are_refused_off_linux() {
        for flag in ["--mstp-rt-priority", "--mstp-cpus"] {
            let error = parse(vec!["bacnet", "tui", flag, "1"]).unwrap_err();
            let error = error.to_string();
            assert!(error.contains("Linux only"), "{flag}: {error}");
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn priorities_outside_1_to_99_are_refused() {
        for bad in ["0", "100", "-1", "x"] {
            assert_eq!(
                parse_rt_priority(bad),
                Err("must be 1 to 99".into()),
                "{bad}"
            );
        }
        let error = parse_cpu("1024").unwrap_err();
        assert!(error.contains("CPU 1024"), "{error}");
        assert!(parse_cpu("two").is_err());
    }

    #[test]
    fn the_thread_is_described_in_a_few_words() {
        assert_eq!(describe(None, &[]), "normal");
        assert_eq!(describe(Some(40), &[3]), "FIFO 40, CPU 3");
        assert_eq!(describe(None, &[2, 3]), "CPUs 2,3");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_takes_both() {
        assert_eq!(parse_rt_priority("50"), Ok(50));
        assert_eq!(parse_cpu(" 3"), Ok(3));
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn elsewhere_both_are_refused() {
        let priority = parse_rt_priority("50").unwrap_err();
        assert!(
            priority.starts_with("--mstp-rt-priority is Linux only"),
            "{priority}"
        );
        let cpus = parse_cpu("3").unwrap_err();
        assert!(cpus.starts_with("--mstp-cpus is Linux only"), "{cpus}");
    }
}
