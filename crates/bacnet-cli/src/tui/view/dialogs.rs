//! The Who-Is form and the interface picker.

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Padding, Paragraph, Wrap};
use ratatui::Frame;

use super::{centered, Theme};
use crate::tui::app::picker::Picker;
use crate::tui::app::whois::{Field, ScopeChoice, WhoIsForm};
use crate::tui::app::App;

/// The Who-Is form.
pub(super) fn who_is(frame: &mut Frame, area: Rect, app: &App, form: &WhoIsForm, theme: &Theme) {
    let width = area.width.saturating_sub(4).min(84);
    let box_area = centered(area, width, 11);
    frame.render_widget(Clear, box_area);

    let target_hint = match (form.scope.needs_target(), form.scope) {
        (false, _) => "not used for this scope",
        (true, ScopeChoice::Network) => "network number, 1-65534",
        (true, _) => crate::tui::app::whois::directed_forms(app.style),
    };
    let mut lines = vec![
        field(
            form,
            Field::Scope,
            "Scope",
            &format!("< {} >", form.scope.label()),
            "Left/Right to change",
            theme,
        ),
        field(
            form,
            Field::Target,
            "Target",
            &form.target,
            target_hint,
            theme,
        ),
        field(
            form,
            Field::Range,
            "Range",
            &form.range,
            "blank = all, N, or LOW-HIGH",
            theme,
        ),
        field(
            form,
            Field::Listen,
            "Listen",
            &form.listen,
            "seconds to wait for replies",
            theme,
        ),
        Line::raw(""),
    ];
    if let Some(error) = &form.error {
        lines.push(Line::styled(error.clone(), theme.danger()));
    } else if let Some(warning) = &form.warning {
        lines.push(Line::styled(format!("Warning: {warning}"), theme.warn()));
        lines.push(Line::styled(
            "Press Enter again to send, or change a field.",
            theme.warn(),
        ));
    }
    let block = Block::bordered()
        .title(Line::styled(" Who-Is ", theme.accent()))
        .title_bottom(Line::styled(
            " Enter send | Tab next field | Esc close ",
            theme.dim(),
        ))
        .border_style(theme.accent())
        .padding(Padding::horizontal(1));
    frame.render_widget(
        Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false }),
        box_area,
    );
}

fn field<'a>(
    form: &WhoIsForm,
    which: Field,
    label: &'a str,
    value: &str,
    hint: &'a str,
    theme: &Theme,
) -> Line<'a> {
    let focused = form.focus == which;
    let enabled = which != Field::Target || form.scope.needs_target();
    let label_style = if focused {
        theme.accent()
    } else {
        Style::new()
    };
    let mut value_text = value.to_string();
    if focused && which != Field::Scope {
        // The cursor.
        value_text.push('_');
    }
    let value_text = format!("{value_text:<24}");
    let value_style = match (focused, enabled) {
        (true, _) => theme.selected(),
        (false, false) => theme.dim(),
        (false, true) => Style::new(),
    };
    Line::from(vec![
        Span::styled(
            format!("{}{label:<8}", if focused { ">" } else { " " }),
            label_style,
        ),
        Span::styled(value_text, value_style),
        Span::styled(format!("  {hint}"), theme.dim()),
    ])
}

/// The interface picker.
pub(super) fn picker(frame: &mut Frame, area: Rect, picker: &Picker, theme: &Theme) {
    let rows = picker
        .items
        .len()
        .min(usize::from(area.height.saturating_sub(10)).max(1));
    let height = rows as u16 + 5;
    let box_area = centered(area, 72, height);
    frame.render_widget(Clear, box_area);
    let offset = picker.selected.saturating_sub(rows - 1);
    let mut lines: Vec<Line> = picker
        .items
        .iter()
        .enumerate()
        .skip(offset)
        .take(rows)
        .map(|(i, iface)| {
            let selected = i == picker.selected;
            let text = format!(
                "{} {:>2}  {:<10} {:<16} broadcast {}",
                if selected { ">" } else { " " },
                i + 1,
                iface.name,
                iface.ip,
                iface.broadcast
            );
            if selected {
                Line::styled(text, theme.selected())
            } else {
                Line::raw(text)
            }
        })
        .collect();
    lines.push(Line::raw(""));
    lines.push(Line::styled(
        "Enter use | Up/Down move | 1-9 pick | Esc quit",
        theme.dim(),
    ));
    lines.push(Line::styled("Tip: -i <IP> skips this dialog.", theme.dim()));
    let block = Block::bordered()
        .title(Line::styled(
            " Choose the BACnet/IP interface ",
            theme.accent(),
        ))
        .border_style(theme.accent())
        .padding(Padding::horizontal(1));
    frame.render_widget(Paragraph::new(lines).block(block), box_area);
}
