//! The write guard for one-shot commands (#972): a command that isn't a
//! [read](bacnet_ops::class::Class::Read) needs `--allow-write`, and a
//! device-management one also needs `--yes`, or the target typed at a
//! terminal. A refused command sends nothing.

use std::io::{BufRead, IsTerminal, Write};

use serde::Serialize;

use bacnet_ops::class::Class;
use bacnet_ops::target::{Target, parse_target};

/// The process exit status of a refused one-shot command.
/// 1 is a runtime error, 2 a usage error from clap, and 3 an `sc-check`
/// that could not complete.
pub(crate) const EXIT_REFUSED: u8 = 4;

/// What the command line allows.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Policy {
    /// `--allow-write`.
    pub(crate) allow_write: bool,
    /// `--yes`.
    pub(crate) yes: bool,
}

/// Where a device-management confirmation comes from.
pub(crate) trait Confirm {
    /// True when a person can answer: stdin is a terminal.
    fn interactive(&self) -> bool;
    /// Show `prompt` and read one line; `None` at the end of input.
    fn ask(&mut self, prompt: &str) -> Option<String>;
}

/// The process's own stdin, prompting on stderr.
pub(crate) struct Stdin;

impl Confirm for Stdin {
    fn interactive(&self) -> bool {
        std::io::stdin().is_terminal()
    }

    fn ask(&mut self, prompt: &str) -> Option<String> {
        let mut stderr = std::io::stderr().lock();
        let _ = write!(stderr, "{prompt}");
        let _ = stderr.flush();
        let mut line = String::new();
        match std::io::stdin().lock().read_line(&mut line) {
            Ok(0) | Err(_) => None,
            Ok(_) => Some(line),
        }
    }
}

/// Why a command was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Reason {
    /// `--allow-write` is missing (and maybe `--yes` too).
    AllowWriteRequired,
    /// Device management with stdin not a terminal, without `--yes`.
    YesRequired,
    /// The typed confirmation didn't match, or input ended.
    NotConfirmed,
    /// The command has no class, so it is not run.
    Unclassified,
}

/// A refused command. Its JSON form is the stable shape documented in
/// docs/CLI.md ("Write guard").
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct Refusal {
    /// Always `"refused"`.
    error: &'static str,
    /// Why.
    pub(crate) reason: Reason,
    /// The command as its canonical name (or the word given).
    pub(crate) command: String,
    /// Its class, `null` when it has none.
    pub(crate) class: Option<Class>,
    /// The flags that would have let it run.
    pub(crate) missing: Vec<&'static str>,
    /// A sentence for people.
    pub(crate) message: String,
}

impl Refusal {
    fn new(
        reason: Reason,
        command: &str,
        class: Option<Class>,
        missing: Vec<&'static str>,
    ) -> Self {
        let what = class.map_or("an unclassified".to_string(), |class| {
            format!("a {}", class.name())
        });
        let message = match reason {
            Reason::AllowWriteRequired | Reason::YesRequired => {
                let why = if missing.contains(&"--yes") {
                    " (stdin is not a terminal, so it can't be confirmed by typing)"
                } else {
                    ""
                };
                format!(
                    "{command} is {what} command and needs {}{why}; nothing was sent",
                    missing.join(" and ")
                )
            }
            Reason::NotConfirmed => format!("{command} was not confirmed; nothing was sent"),
            Reason::Unclassified => format!("{command} is {what} command; nothing was sent"),
        };
        Self {
            error: "refused",
            reason,
            command: command.to_string(),
            class,
            missing,
            message,
        }
    }

    /// The single-line JSON form.
    pub(crate) fn to_json(&self) -> String {
        serde_json::to_string(self).expect("a refusal serializes")
    }
}

/// What to type to confirm device management of `target`: its device
/// instance when it names one, otherwise the target as given, as the TUI
/// asks for a device instance (or a BBMD address) at T3.
fn confirmation(target: &str) -> (String, String) {
    match parse_target(target) {
        Ok(Target::Instance(instance) | Target::Routed { instance, .. }) => (
            format!("the device instance ({instance})"),
            instance.to_string(),
        ),
        _ => (format!("the target ({target})"), target.to_string()),
    }
}

/// May `command` (a one-shot subcommand name) of `class`
/// run under `policy`? The caller gives the class: [`bacnet_ops::class::of`] by name,
/// or [`bacnet_ops::class::of_subscribe`] when the arguments decide it. `None` refuses
/// it as unclassified. `target` is what a device-management confirmation
/// names. Device management asks `confirm` only when the flags allow it
/// and `--yes` is absent at a terminal.
pub(crate) fn check(
    command: &str,
    class: Option<Class>,
    target: Option<&str>,
    policy: Policy,
    confirm: &mut dyn Confirm,
) -> Result<(), Refusal> {
    let refuse = |reason, class, missing| Err(Refusal::new(reason, command, class, missing));
    let Some(class) = class else {
        return refuse(Reason::Unclassified, None, vec![]);
    };
    match class {
        Class::Read => Ok(()),
        Class::Write if policy.allow_write => Ok(()),
        Class::Write => refuse(
            Reason::AllowWriteRequired,
            Some(class),
            vec!["--allow-write"],
        ),
        Class::DeviceManagement => {
            let interactive = confirm.interactive();
            if !policy.allow_write {
                let mut missing = vec!["--allow-write"];
                if !policy.yes && !interactive {
                    missing.push("--yes");
                }
                return refuse(Reason::AllowWriteRequired, Some(class), missing);
            }
            if policy.yes {
                return Ok(());
            }
            if !interactive {
                return refuse(Reason::YesRequired, Some(class), vec!["--yes"]);
            }
            let Some(target) = target else {
                return refuse(Reason::NotConfirmed, Some(class), vec![]);
            };
            let (what, expected) = confirmation(target);
            let prompt = format!(
                "{command} is a device-management command. Type {what} and Enter to send: "
            );
            match confirm.ask(&prompt) {
                Some(answer) if answer.trim() == expected => Ok(()),
                _ => refuse(Reason::NotConfirmed, Some(class), vec![]),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// [`super::check`], classified by name.
    fn check(
        command: &str,
        target: Option<&str>,
        policy: Policy,
        confirm: &mut dyn Confirm,
    ) -> Result<(), Refusal> {
        super::check(
            command,
            bacnet_ops::class::of(command),
            target,
            policy,
            confirm,
        )
    }

    /// A terminal, or not, with typed answers.
    struct Fake {
        interactive: bool,
        answers: Vec<&'static str>,
        prompts: Vec<String>,
    }

    impl Fake {
        fn new(interactive: bool, answers: &[&'static str]) -> Self {
            Self {
                interactive,
                answers: answers.iter().rev().copied().collect(),
                prompts: vec![],
            }
        }
    }

    impl Confirm for Fake {
        fn interactive(&self) -> bool {
            self.interactive
        }
        fn ask(&mut self, prompt: &str) -> Option<String> {
            self.prompts.push(prompt.to_string());
            self.answers.pop().map(str::to_string)
        }
    }

    const NONE: Policy = Policy {
        allow_write: false,
        yes: false,
    };
    const ALLOW: Policy = Policy {
        allow_write: true,
        yes: false,
    };
    const BOTH: Policy = Policy {
        allow_write: true,
        yes: true,
    };

    fn reason(result: Result<(), Refusal>) -> Option<(Reason, Vec<&'static str>)> {
        result.err().map(|r| (r.reason, r.missing))
    }

    #[test]
    fn reads_need_nothing_and_writes_need_allow_write() {
        for interactive in [false, true] {
            let mut tty = Fake::new(interactive, &[]);
            assert_eq!(check("read", None, NONE, &mut tty), Ok(()));
            assert_eq!(check("subscribe", None, NONE, &mut tty), Ok(()));
            for write in [
                "write",
                "wp",
                "writem",
                "file-write",
                "time-sync",
                "unregister",
            ] {
                assert_eq!(
                    reason(check(write, None, NONE, &mut tty)),
                    Some((Reason::AllowWriteRequired, vec!["--allow-write"])),
                    "{write}"
                );
                assert_eq!(check(write, None, ALLOW, &mut tty), Ok(()), "{write}");
            }
            assert!(tty.prompts.is_empty(), "a write never prompts");
        }
    }

    #[test]
    fn device_management_off_a_terminal_needs_yes() {
        let mut pipe = Fake::new(false, &["1234"]);
        for command in ["control", "dcc", "reinit"] {
            assert_eq!(
                reason(check(command, Some("1234"), NONE, &mut pipe)),
                Some((Reason::AllowWriteRequired, vec!["--allow-write", "--yes"]))
            );
            assert_eq!(
                reason(check(command, Some("1234"), ALLOW, &mut pipe)),
                Some((Reason::YesRequired, vec!["--yes"]))
            );
            assert_eq!(check(command, Some("1234"), BOTH, &mut pipe), Ok(()));
        }
        assert!(pipe.prompts.is_empty(), "nothing is asked off a terminal");
    }

    #[test]
    fn device_management_at_a_terminal_types_the_device_instance() {
        // Without --allow-write it is refused before any prompt.
        let mut tty = Fake::new(true, &["1234"]);
        assert_eq!(
            reason(check("reinit", Some("1234"), NONE, &mut tty)),
            Some((Reason::AllowWriteRequired, vec!["--allow-write"]))
        );
        assert!(tty.prompts.is_empty());
        assert_eq!(check("reinit", Some("1234"), ALLOW, &mut tty), Ok(()));
        assert_eq!(
            tty.prompts,
            [
                "reinit is a device-management command. Type the device instance (1234) and Enter to send: "
            ]
        );
        // A routed target types its instance; an address types itself.
        let mut tty = Fake::new(true, &["77\n", "10.0.0.5:47808\n"]);
        assert_eq!(check("dcc", Some("2:77"), ALLOW, &mut tty), Ok(()));
        assert_eq!(
            check("dcc", Some("10.0.0.5:47808"), ALLOW, &mut tty),
            Ok(())
        );
        // A wrong answer, or the end of input, declines.
        let mut tty = Fake::new(true, &["123"]);
        for _ in 0..2 {
            assert_eq!(
                reason(check("reinit", Some("1234"), ALLOW, &mut tty)),
                Some((Reason::NotConfirmed, vec![]))
            );
        }
        // --yes skips the prompt.
        let mut tty = Fake::new(true, &[]);
        assert_eq!(check("reinit", Some("1234"), BOTH, &mut tty), Ok(()));
        assert!(tty.prompts.is_empty());
    }

    #[test]
    fn an_unknown_command_is_refused() {
        let mut tty = Fake::new(true, &[]);
        assert_eq!(
            reason(check("frobnicate", None, BOTH, &mut tty)),
            Some((Reason::Unclassified, vec![]))
        );
    }

    /// The JSON shape scripts parse, documented in docs/CLI.md.
    #[test]
    fn the_json_refusal_shape_is_stable() {
        let mut pipe = Fake::new(false, &[]);
        let refusal = check("reinit", Some("1234"), NONE, &mut pipe).unwrap_err();
        assert_eq!(
            refusal.to_json(),
            concat!(
                r#"{"error":"refused","reason":"allow-write-required","command":"reinit","#,
                r#""class":"device-management","missing":["--allow-write","--yes"],"#,
                r#""message":"reinit is a device-management command and needs --allow-write "#,
                r#"and --yes (stdin is not a terminal, so it can't be confirmed by typing); "#,
                r#"nothing was sent"}"#
            )
        );
        let refusal = check("write", None, NONE, &mut pipe).unwrap_err();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&refusal.to_json()).unwrap(),
            serde_json::json!({
                "error": "refused",
                "reason": "allow-write-required",
                "command": "write",
                "class": "write",
                "missing": ["--allow-write"],
                "message": "write is a write command and needs --allow-write; nothing was sent",
            })
        );
        let reasons = [
            check("reinit", Some("1"), ALLOW, &mut pipe).unwrap_err(),
            check("reinit", None, ALLOW, &mut Fake::new(true, &[])).unwrap_err(),
            check("frobnicate", None, NONE, &mut pipe).unwrap_err(),
        ]
        .map(|r| serde_json::to_value(&r).unwrap());
        assert_eq!(reasons[0]["reason"], "yes-required");
        assert_eq!(reasons[1]["reason"], "not-confirmed");
        assert_eq!(reasons[2]["reason"], "unclassified");
        assert_eq!(reasons[2]["class"], serde_json::Value::Null);
    }
}
