//! Fitting a row of parts (key hints, status-bar fields) into a width by
//! priority (#1894): when the row is too wide, the part that matters least
//! gives way first, by taking its shorter form or by going. Parts marked
//! [`ALWAYS`] never go, so `? help` and `q quit` stay at 80 columns.

use ratatui::style::Style;
use ratatui::text::{Line, Span};

use super::{Theme, text_width};

/// The priority of a part that is never dropped.
pub(super) const ALWAYS: u8 = u8::MAX;

/// One part of a row: its forms, longest first, each with the priority at
/// which it gives way to the next form (or, for the last, to nothing).
#[derive(Debug, Clone)]
pub(super) struct Part {
    forms: Vec<(u8, Vec<Span<'static>>)>,
}

impl Part {
    /// A part with one form.
    pub(super) fn new(keep: u8, spans: Vec<Span<'static>>) -> Self {
        Self {
            forms: vec![(keep, spans)],
        }
    }

    /// A shorter form, taken when the longer one gives way at `keep`.
    pub(super) fn or(mut self, keep: u8, spans: Vec<Span<'static>>) -> Self {
        self.forms.push((keep, spans));
        self
    }

    fn width(spans: &[Span]) -> usize {
        spans.iter().map(|span| text_width(&span.content)).sum()
    }
}

/// `parts` in `room` columns, each separated by `separator`: the parts that
/// fit, in their order, after the lowest priority ones gave way (the
/// rightmost first among equals). `ALWAYS` parts are kept even when they
/// alone overflow.
pub(super) fn fit(parts: Vec<Part>, separator: &Span<'static>, room: usize) -> Vec<Span<'static>> {
    // Each part's current form, or `None` once it went.
    let mut at: Vec<Option<usize>> = parts.iter().map(|_| Some(0)).collect();
    let gap = text_width(&separator.content);
    let width = |at: &[Option<usize>]| {
        let (mut total, mut shown) = (0, 0_usize);
        for (part, form) in parts.iter().zip(at) {
            if let Some(form) = form {
                total += Part::width(&part.forms[*form].1);
                shown += 1;
            }
        }
        total + gap * shown.saturating_sub(1)
    };
    // Each step taken: the part and the form it had.
    let mut steps = Vec::new();
    while width(&at) > room {
        let weakest = parts
            .iter()
            .zip(&at)
            .enumerate()
            .filter_map(|(index, (part, form))| {
                let keep = part.forms[(*form)?].0;
                (keep != ALWAYS).then_some((keep, index))
            })
            // The lowest priority; among equals the rightmost.
            .min_by_key(|(keep, index)| (*keep, usize::MAX - index));
        let Some((_, index)) = weakest else { break };
        steps.push((index, at[index]));
        let next = at[index].map(|form| form + 1);
        at[index] = next.filter(|form| *form < parts[index].forms.len());
    }
    // A later, larger step can leave room for an earlier one: undo the
    // steps that fit again, the most important first. A step is undone
    // only after every later step on the same part.
    let mut kept: Vec<usize> = Vec::new();
    for (index, before) in steps.into_iter().rev() {
        if kept.contains(&index) {
            continue;
        }
        let after = std::mem::replace(&mut at[index], before);
        if width(&at) > room {
            at[index] = after;
            kept.push(index);
        }
    }
    let mut out = Vec::new();
    for (part, form) in parts.into_iter().zip(at) {
        let Some(form) = form else { continue };
        if !out.is_empty() {
            out.push(separator.clone());
        }
        out.extend(
            part.forms
                .into_iter()
                .nth(form)
                .map(|f| f.1)
                .unwrap_or_default(),
        );
    }
    out
}

/// One key hint: `key` in the accent style and what it does, dim.
pub(super) fn hint(theme: &Theme, keep: u8, key: &str, what: &str) -> Part {
    Part::new(keep, hint_spans(theme, key, what))
}

fn hint_spans(theme: &Theme, key: &str, what: &str) -> Vec<Span<'static>> {
    vec![
        Span::styled(key.to_string(), theme.accent()),
        Span::styled(format!(" {what}"), theme.dim()),
    ]
}

impl Part {
    /// A shorter wording of a key hint, taken when the longer gives way.
    pub(super) fn or_hint(self, theme: &Theme, keep: u8, key: &str, what: &str) -> Self {
        self.or(keep, hint_spans(theme, key, what))
    }
}

/// Key hints fitted to `room` columns, two spaces apart.
pub(super) fn hints(parts: Vec<Part>, room: usize) -> Line<'static> {
    Line::from(fit(parts, &Span::raw("  "), room))
}

/// The width the parts kept down to priority `keep` take as key hints, in
/// their shortest forms: what a notice beside them leaves room for.
pub(super) fn reserve(parts: &[Part], keep: u8) -> usize {
    let widths: Vec<usize> = parts
        .iter()
        .filter_map(|part| part.forms.last())
        .filter(|(at, _)| *at >= keep)
        .map(|(_, spans)| Part::width(spans))
        .collect();
    widths.iter().sum::<usize>() + 2 * widths.len().saturating_sub(1)
}

/// A plain span, for parts built from text.
pub(super) fn span(text: impl Into<String>, style: Style) -> Span<'static> {
    Span::styled(text.into(), style)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(spans: &[Span]) -> String {
        spans.iter().map(|span| span.content.as_ref()).collect()
    }

    fn part(keep: u8, text: &str) -> Part {
        Part::new(keep, vec![Span::raw(text.to_string())])
    }

    #[test]
    fn the_lowest_priority_goes_first_and_always_stays() {
        let parts = || {
            vec![
                part(3, "aaaa"),
                part(1, "bbbb"),
                part(2, "cccc"),
                part(ALWAYS, "help"),
            ]
        };
        let sep = Span::raw(" ");
        assert_eq!(text(&fit(parts(), &sep, 19)), "aaaa bbbb cccc help");
        assert_eq!(text(&fit(parts(), &sep, 18)), "aaaa cccc help");
        assert_eq!(text(&fit(parts(), &sep, 13)), "aaaa help");
        assert_eq!(text(&fit(parts(), &sep, 2)), "help");
    }

    #[test]
    fn a_part_comes_back_when_a_wider_one_went_after_it() {
        let sep = Span::raw(" ");
        let parts = vec![part(1, "ab"), part(5, &"x".repeat(20)), part(ALWAYS, "h")];
        assert_eq!(text(&fit(parts, &sep, 10)), "ab h");
    }

    #[test]
    fn ties_drop_the_rightmost_and_forms_shrink_before_going() {
        let sep = Span::raw(" ");
        let parts = vec![
            part(1, "left"),
            part(1, "right"),
            Part::new(5, vec![Span::raw("long form")]).or(2, vec![Span::raw("lf")]),
        ];
        assert_eq!(text(&fit(parts.clone(), &sep, 20)), "left right long form");
        assert_eq!(text(&fit(parts.clone(), &sep, 15)), "left long form");
        assert_eq!(text(&fit(parts.clone(), &sep, 9)), "long form");
        assert_eq!(text(&fit(parts.clone(), &sep, 4)), "lf");
        assert_eq!(text(&fit(parts, &sep, 1)), "");
    }
}
