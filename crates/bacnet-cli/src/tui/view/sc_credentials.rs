//! The SC screen's credentials view (#968): this node's operational
//! certificate, chain and CA bundle as the pre-dial lint read them, after
//! every finding, the key match and the direct-issuer check. Each level is a word
//! as well as a colour.

use ratatui::style::Style;
use ratatui::text::{Line, Span};

use super::sc::Row;
use super::Theme;
use crate::sc_check::HUB_NAME_NOTE;
use crate::tui::sc::credentials::{Cert, Credentials, Level};

fn style(level: Level, theme: &Theme) -> Style {
    match level {
        Level::Ok => theme.accent(),
        Level::Notice => theme.dim(),
        Level::Warning => theme.warn(),
        Level::Alarm | Level::Failed => theme.danger(),
    }
}

fn mark<'a>(level: Level, theme: &Theme) -> Span<'a> {
    Span::styled(format!("{} ", level.word()), style(level, theme))
}

/// Where a certificate's or check's text starts: mark and label.
const DETAIL: usize = 21;
/// Where a finding's text starts: after the mark.
const MARKED: usize = 7;

fn cert<'a>(cert: &Cert, theme: &Theme) -> [Row<'a>; 2] {
    let first = Line::from(vec![
        mark(cert.level, theme),
        Span::styled(format!("{:<14}", cert.label), theme.dim()),
        Span::styled(cert.subject.clone(), theme.accent()),
        Span::styled(
            format!("  {} (until {})", cert.window, cert.not_after),
            style(cert.level, theme),
        ),
    ]);
    let second = Line::styled(
        format!(
            "{:DETAIL$}issued by {}  key {}  SAN {}  EKU {}",
            "", cert.issuer, cert.key_algorithm, cert.san, cert.eku
        ),
        theme.dim(),
    );
    [Row::hang(first, DETAIL), Row::hang(second, DETAIL)]
}

/// The view's rows; `None` before any SC dial.
pub(super) fn rows<'a>(
    credentials: Option<&Result<Credentials, String>>,
    theme: &Theme,
) -> Vec<Row<'a>> {
    let credentials = match credentials {
        None if cfg!(feature = "sc-tls") => {
            return vec![Row::flow(Line::styled(
                "No dial yet: the credentials are checked just before each one.",
                theme.dim(),
            ))]
        }
        None => {
            return vec![Row::flow(Line::styled(
                "Credential checks need a build with the sc-tls feature.",
                theme.dim(),
            ))]
        }
        Some(Err(error)) => {
            let line = Line::from(vec![
                mark(Level::Failed, theme),
                Span::styled(error.clone(), theme.danger()),
            ]);
            return vec![
                Row::hang(line, MARKED),
                Row::flow(Line::styled(
                    "The dial was refused before connecting.",
                    theme.danger(),
                )),
            ];
        }
        Some(Ok(credentials)) => credentials,
    };
    // The verdicts first, so a short pane keeps them; the details after.
    let mut lines = vec![Row::flow(Line::styled(
        format!(
            "Checked at {} before the dial. Findings, then each certificate.",
            credentials.inspected_at
        ),
        theme.dim(),
    ))];
    for (level, text) in &credentials.findings {
        let line = Line::from(vec![
            mark(*level, theme),
            Span::styled(text.clone(), style(*level, theme)),
        ]);
        lines.push(Row::hang(line, MARKED));
    }
    if credentials
        .findings
        .iter()
        .any(|(level, _)| *level == Level::Failed)
    {
        lines.push(Row::flow(Line::styled(
            "The dial was refused before connecting: fix the failures above.",
            theme.danger(),
        )));
    }
    for (label, (level, text)) in [("Key", &credentials.key), ("Issuer", &credentials.issuer)] {
        let line = Line::from(vec![
            mark(*level, theme),
            Span::styled(format!("{label:<14}"), theme.dim()),
            Span::styled(text.clone(), style(*level, theme)),
        ]);
        lines.push(Row::hang(line, DETAIL));
    }
    for c in &credentials.certificates {
        lines.extend(cert(c, theme));
    }
    lines.push(Row::flow(Line::styled(HUB_NAME_NOTE, theme.dim())));
    lines
}
