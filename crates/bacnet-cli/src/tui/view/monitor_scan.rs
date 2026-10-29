//! Compact scan evidence, including incomplete results and restoration status.
use super::Theme;
use crate::tui::app::monitor::MonitorState;
use crate::tui::monitor::scan::{RowStatus, ScanPhase, Suggestion};
use ratatui::{
    Frame,
    layout::Rect,
    text::Line,
    widgets::{Paragraph, Wrap},
};

pub(super) fn draw(frame: &mut Frame, area: Rect, monitor: &MonitorState, theme: &Theme) {
    let Some(scan) = monitor.scan.as_ref() else {
        frame.render_widget(
            Paragraph::new("Start: :mstp scan SECS (1..3600 per rate). Cancel: :mstp scan cancel")
                .wrap(Wrap { trim: false }),
            area,
        );
        return;
    };
    let phase = match &scan.phase {
        ScanPhase::Retiring => "Closing the original monitor and capture".into(),
        ScanPhase::Candidates => format!(
            "Measuring six rates; {} s requested per rate",
            scan.window.as_secs()
        ),
        ScanPhase::Restoring => format!(
            "Restoring original {} baud; awaiting admission",
            scan.original
        ),
        ScanPhase::Restored => format!(
            "Original {} baud restored; fresh monitor counters",
            scan.original
        ),
        ScanPhase::Stopped(error) => format!("STOPPED: {error}"),
    };
    let wide = area.width >= 108;
    let mut lines = vec![
        Line::styled(phase, theme.accent()),
        Line::raw(if wide {
            "  Baud   Status       Seconds   Valid   H-CRC   D-CRC  Malform   Abort    Noise   Score"
        } else {
            "  Baud   Status       Seconds     Valid    Errors     Noise   Score"
        }),
    ];
    for row in &scan.rows {
        let status = match row.status {
            RowStatus::Pending => "pending",
            RowStatus::Opening => "opening",
            RowStatus::Measuring => "measuring",
            RowStatus::Closing => "closing",
            RowStatus::Complete => "complete",
            RowStatus::Incomplete(_) => "incomplete",
            RowStatus::Failed(_) => "failed",
        };
        let e = row.evidence;
        let score = e
            .percent()
            .map_or_else(|| "--".into(), |p| format!("{p:.1}%"));
        let text = if wide {
            format!(
                "{:>6}   {:<10} {:>7.2} {:>7} {:>7} {:>7} {:>8} {:>7} {:>8} {:>7}",
                row.baud,
                status,
                row.elapsed.as_secs_f64(),
                e.valid,
                e.errors[0],
                e.errors[1],
                e.errors[2],
                e.errors[3],
                e.noise,
                score
            )
        } else {
            format!(
                "{:>6}   {:<10} {:>7.2} {:>9} {:>9} {:>9} {:>7}",
                row.baud,
                status,
                row.elapsed.as_secs_f64(),
                e.valid,
                e.errors(),
                e.noise,
                score
            )
        };
        lines.push(Line::raw(text));
    }
    let result = match scan.suggestion() {
        Suggestion::Rate(baud) => {
            format!("Suggestion: {baud} baud. No rate was applied; inspect the counts.")
        }
        Suggestion::Ambiguous(rates) => {
            format!("Ambiguous: equal best ratios at {rates:?}; no count tiebreak.")
        }
        Suggestion::Insufficient => "Insufficient traffic: no valid frames at any rate.".into(),
        Suggestion::Partial => {
            if scan.active() {
                "In progress: no global suggestion until every rate completes.".into()
            } else {
                "Partial scan: no global best across incomplete/unmeasured rates.".into()
            }
        }
        Suggestion::Saturated => "Counter saturation: exact ranking unavailable.".into(),
    };
    lines.push(Line::styled(result, theme.accent()));
    if !wide && let Some(row) = scan.rows.get(monitor.scan_selected) {
        let e = row.evidence.errors;
        lines.push(Line::raw(format!(
            "{} detail: H-CRC {}  D-CRC {}  malformed {}  aborted {}",
            row.baud, e[0], e[1], e[2], e[3]
        )));
    }
    if scan.cancelled {
        lines.push(Line::raw(
            "Cancelled by operator; retained partial evidence.",
        ));
    }
    for row in &scan.rows {
        if let RowStatus::Failed(reason) | RowStatus::Incomplete(reason) = &row.status {
            lines.push(Line::raw(format!("{}: {reason}", row.baud)));
        }
    }
    lines.push(Line::raw(
        "Score = valid / (valid + H-CRC + D-CRC + malformed + aborted).",
    ));
    lines.push(Line::raw(
        "Noise is separate. Sparse samples are not baud/UART confidence.",
    ));
    lines.push(Line::raw(
        ":mstp scan cancel  |  Up/Down: detail  |  Tab: monitor views",
    ));
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
}
