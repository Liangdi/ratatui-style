//! A **Tailwind-style utility-first** design system, built entirely on
//! ratatui-style's CSS cascade.
//!
//! Tailwind's mental model maps cleanly onto this engine:
//!
//! | Tailwind              | ratatui-style                                  |
//! |-----------------------|------------------------------------------------|
//! | design tokens (theme) | `:root { --blue-500: #3b82f6; }` + `var()`     |
//! | atomic utilities      | single-property class rules: `.bg-blue-500` …  |
//! | `className="… … …"`   | many classes on one node, merged by the cascade|
//! | `focus:` / `disabled:`| native `:focus` / `:disabled` pseudo-states    |
//!
//! The headline trick: because every utility targets a *different* CSS field
//! (`background`, `color`, `padding`, `border-style`, …), a node listing many
//! utilities composes them with zero conflicts — and `border-style` + `border-color`
//! even merge into one border. Variant prefixes become higher-specificity
//! state-qualified rules, so `focus:` is literally just `.btn:focus { … }`.
//!
//! `←/→` moves focus across the buttons (watch the `:focus` variant recolor
//! them live), `d` toggles `:disabled`, `q`/`Esc` quits.
//!
//! ```sh
//! cargo run -p ratatui-style --example 06_tailwind
//! ```

use std::io::{self, Stdout};
use std::time::Duration;

use crossterm::{
    event::{self, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    Terminal, backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    text::{Line, Span},
    widgets::{Block, Paragraph},
};

use ratatui_style::{ComputeScratch, NodeRef, State, Stylesheet};

type Term = Terminal<CrosstermBackend<Stdout>>;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let sheet = Stylesheet::parse(TAILWIND)?;

    let buttons = [
        Button { label: "Deploy", classes: &["btn", "primary"] },
        Button { label: "Preview", classes: &["btn", "outline"] },
        Button { label: "Cancel", classes: &["btn", "ghost"] },
    ];
    let n = buttons.len();
    let mut focus = 0usize;
    let mut disabled = false;

    let mut terminal = setup()?;
    let mut scratch = ComputeScratch::new();
    loop {
        terminal.draw(|f| draw(f, &sheet, &mut scratch, &buttons, focus, disabled))?;

        if !event::poll(Duration::from_millis(100))? {
            continue;
        }
        if let Event::Key(key) = event::read()? {
            match key.code {
                KeyCode::Left => focus = (focus + n - 1) % n,
                KeyCode::Right | KeyCode::Tab => focus = (focus + 1) % n,
                KeyCode::Char('d') => disabled = !disabled,
                KeyCode::Char('q') | KeyCode::Esc => break,
                _ => {}
            }
        }
    }
    teardown(&mut terminal)?;
    Ok(())
}

struct Button {
    label: &'static str,
    classes: &'static [&'static str],
}

// --- the stylesheet: tokens → atomic utilities → component + variant layer --------

const TAILWIND: &str = r##"
/* ============================================================= *
 * 1. Design tokens — the Tailwind color palette, as CSS          *
 *    custom properties. Utilities reference them via var().      *
 * ============================================================= */
:root {
    --slate-900: #0f172a;
    --slate-800: #1e293b;
    --slate-700: #334155;
    --slate-500: #64748b;
    --slate-400: #94a3b8;
    --slate-300: #cbd5e1;
    --slate-100: #f1f5f9;

    --blue-300:  #93c5fd;
    --blue-500:  #3b82f6;
    --blue-600:  #2563eb;

    --green-500: #22c55e;
    --amber-500: #f59e0b;
    --red-500:   #ef4444;
}

/* ============================================================= *
 * 2. Atomic utilities — one declaration each. Because every     *
 *    utility writes a *different* CSS field, an element that     *
 *    carries many of them composes with no conflicts.            *
 * ============================================================= */

/* background */
.bg-slate-900 { background: var(--slate-900); }
.bg-slate-800 { background: var(--slate-800); }
.bg-slate-700 { background: var(--slate-700); }
.bg-blue-500  { background: var(--blue-500); }
.bg-green-500 { background: var(--green-500); }
.bg-amber-500 { background: var(--amber-500); }
.bg-red-500   { background: var(--red-500); }

/* text color */
.text-slate-100 { color: var(--slate-100); }
.text-slate-300 { color: var(--slate-300); }
.text-slate-400 { color: var(--slate-400); }
.text-slate-500 { color: var(--slate-500); }
.text-slate-900 { color: var(--slate-900); }
.text-blue-300  { color: var(--blue-300); }
.text-white     { color: #ffffff; }

/* spacing (terminal cells) */
.p-1  { padding: 1; }
.px-1 { padding: 0 1; }
.px-2 { padding: 0 2; }

/* border — `border-style` and `border-color` are independent utilities that
 * merge into one border (the cascade composes sub-fields), so
 * `rounded border-slate-700` yields a rounded, slate-colored border. */
.rounded           { border-style: rounded; }
.border-slate-500  { border-color: var(--slate-500); }
.border-slate-700  { border-color: var(--slate-700); }
.border-blue-500   { border-color: var(--blue-500); }

/* typography */
.font-bold    { font-weight: bold; }
.italic       { font-style: italic; }
.underline    { text-decoration: underline; }
.line-through { text-decoration: line-through; }
.text-left    { text-align: left; }
.text-center  { text-align: center; }

/* ============================================================= *
 * 3. Component + variant layer. Tailwind writes `focus:bg-blue-600`*
 *    inline; here the same idea is a state-qualified rule on a    *
 *    base class. Specificity climbs with each qualifier, so the   *
 *    focused/disabled variant cleanly overrides the base.         *
 * ============================================================= */
.btn             { padding: 0 2; font-weight: bold; text-align: center; }
.btn.primary     { background: var(--blue-500); color: var(--slate-900); border-style: rounded; }
.btn.outline     { border-style: rounded; border-color: var(--blue-500); color: var(--blue-300); }
.btn.ghost       { color: var(--slate-300); border-style: rounded; }

.btn:focus            { background: var(--slate-700); color: var(--slate-100); }
.btn.primary:focus    { background: var(--blue-600); color: var(--slate-100); }
.btn.outline:focus    { background: var(--blue-500); color: var(--slate-900); }
.btn:disabled         { color: var(--slate-500); }
.btn.primary:disabled { background: var(--slate-700); color: var(--slate-500); }
"##;

// --- rendering ---------------------------------------------------------------------

fn draw(frame: &mut ratatui::Frame<'_>, sheet: &Stylesheet, scratch: &mut ComputeScratch, buttons: &[Button], focus: usize, disabled: bool) {
    let area = frame.area();

    // Root fill: a single utility class.
    let root = sheet.compute_with(&NodeRef::new("Root").classes(&["bg-slate-900"]), None, scratch);
    frame.render_widget(Block::default().style(root.to_style()), area);

    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),  // header
            Constraint::Min(0),    // gallery
            Constraint::Length(3), // buttons
            Constraint::Length(2), // footer
        ])
        .split(area);

    header(frame, sheet, scratch, outer[0]);
    gallery(frame, sheet, scratch, outer[1]);
    button_row(frame, sheet, scratch, outer[2], buttons, focus, disabled);
    footer(frame, sheet, scratch, outer[3]);
}

fn header(frame: &mut ratatui::Frame<'_>, sheet: &Stylesheet, scratch: &mut ComputeScratch, area: Rect) {
    // Utilities composed straight onto a header "element".
    let st = sheet.compute_with(&NodeRef::new("Div").classes(&["bg-slate-800", "text-slate-100", "font-bold", "text-center"]), None, scratch);
    frame.render_widget(
        Paragraph::new(Line::from(" tailwind-style · utility-first cascade"))
            .style(st.to_style()),
        area,
    );
}

fn gallery(frame: &mut ratatui::Frame<'_>, sheet: &Stylesheet, scratch: &mut ComputeScratch, area: Rect) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(8), // cards
            Constraint::Length(2), // badges
            Constraint::Min(0),    // typography
        ])
        .split(area);

    // --- Two cards. Each is built from atomic utilities; the caption echoes
    //     the exact className so you can see what composed it.
    let card_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .horizontal_margin(1)
        .split(rows[0]);

    // First card lines - pre-compute spans to avoid double borrow of scratch
    let card1_lines = [
        Line::from(span(sheet, scratch, &["text-slate-100", "font-bold"], " Utility card")),
        Line::from(span(sheet, scratch, &["text-slate-400"], " Four atomic utilities, one element.")),
        Line::from(""),
        Line::from(span(
            sheet,
            scratch,
            &["text-slate-500"],
            " bg-slate-800 · rounded · border-slate-700 · p-1",
        )),
    ];

    let card2_lines = [
        Line::from(span(sheet, scratch, &["text-slate-900", "font-bold"], " Brand card")),
        Line::from(span(sheet, scratch, &["text-slate-900"], " Same cascade, swapped tokens.")),
        Line::from(""),
        Line::from(span(sheet, scratch, &["text-slate-900"], " bg-blue-500 · rounded · p-1")),
    ];

    card(
        frame,
        sheet,
        scratch,
        card_cols[0],
        &["bg-slate-800", "rounded", "border-slate-700", "p-1"],
        &card1_lines,
    );

    card(
        frame,
        sheet,
        scratch,
        card_cols[1],
        &["bg-blue-500", "rounded", "p-1"],
        &card2_lines,
    );

    // --- Badges: each a different color utility (filled) or border utility (outline).
    let badge_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(5),
            Constraint::Length(8),
            Constraint::Length(8),
            Constraint::Length(8),
            Constraint::Length(9),
        ])
        .horizontal_margin(1)
        .split(rows[1]);

    let st = sheet.compute_with(&NodeRef::new("Div").classes(&["text-slate-400"]), None, scratch);
    frame.render_widget(Paragraph::new(Line::from(" CI:")).style(st.to_style()), badge_cols[0]);

    badge(frame, sheet, scratch, badge_cols[1], &["bg-green-500", "text-slate-900", "font-bold"], "build");
    badge(frame, sheet, scratch, badge_cols[2], &["bg-amber-500", "text-slate-900", "font-bold"], "lint");
    badge(frame, sheet, scratch, badge_cols[3], &["bg-red-500", "text-slate-900", "font-bold"], "tests");
    badge(
        frame,
        sheet,
        scratch,
        badge_cols[4],
        &["bg-slate-700", "text-slate-300", "font-bold"],
        "docs",
    );

    // --- Typography utilities.
    let typo = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1); 4])
        .horizontal_margin(1)
        .split(rows[2]);
    let lines: &[(&[&'static str], &str)] = &[
        (&["text-slate-300"], "The quick brown fox"),
        (&["text-slate-100", "font-bold"], "jumps over the lazy dog"),
        (&["text-slate-300", "italic"], "in italic, with a flourish"),
        (&["text-blue-300", "underline"], "and an underlined finish line"),
    ];
    for (rect, (cls, txt)) in typo.iter().zip(lines.iter()) {
        let st = sheet.compute_with(&NodeRef::new("Div").classes(cls), None, scratch);
        frame.render_widget(Paragraph::new(Line::from(format!(" {txt}"))).style(st.to_style()), *rect);
    }
}

/// Render a card: a bordered/padded block (its utilities) wrapping content lines.
fn card(
    frame: &mut ratatui::Frame<'_>,
    sheet: &Stylesheet,
    scratch: &mut ComputeScratch,
    area: Rect,
    classes: &[&'static str],
    lines: &[Line<'_>],
) {
    let computed = sheet.compute_with(&NodeRef::new("Div").classes(classes), None, scratch);
    let block = computed.to_block();
    let inner = block.inner(area);
    frame.render_widget(block, area);
    frame.render_widget(Paragraph::new(lines.to_vec()), inner);
}

/// Render an inline badge: a colored utility pill (bg fill + centered text).
fn badge(
    frame: &mut ratatui::Frame<'_>,
    sheet: &Stylesheet,
    scratch: &mut ComputeScratch,
    area: Rect,
    classes: &[&'static str],
    text: &str,
) {
    let computed = sheet.compute_with(&NodeRef::new("Div").classes(classes), None, scratch);
    frame.render_widget(
        Paragraph::new(Line::from(format!(" {text} ")))
            .style(computed.to_style())
            .alignment(Alignment::Center),
        area,
    );
}

fn button_row(
    frame: &mut ratatui::Frame<'_>,
    sheet: &Stylesheet,
    scratch: &mut ComputeScratch,
    area: Rect,
    buttons: &[Button],
    focus: usize,
    disabled: bool,
) {
    let rects = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(34), Constraint::Percentage(33), Constraint::Percentage(33)])
        .horizontal_margin(1)
        .split(area);

    for (i, b) in buttons.iter().enumerate() {
        let node = NodeRef::new("Div")
            .classes(b.classes)
            .state(State {
                focus: i == focus && !disabled,
                disabled,
                ..State::empty()
            });
        let computed = sheet.compute_with(&node, None, scratch);
        let para = Paragraph::new(Line::from(format!(" {} ", b.label)))
            .alignment(Alignment::Center);
        frame.render_widget(para.block(computed.to_block()), rects[i]);
    }
}

fn footer(frame: &mut ratatui::Frame<'_>, sheet: &Stylesheet, scratch: &mut ComputeScratch, area: Rect) {
    let st = sheet.compute_with(&NodeRef::new("Div").classes(&["text-slate-500"]), None, scratch);
    frame.render_widget(
        Paragraph::new(Line::from(format!(
            " {:<-10} focus · {:<-10} toggle disabled · q to quit",
            "←/→", "d"
        )))
        .style(st.to_style()),
        area,
    );
}

// --- helpers -----------------------------------------------------------------------

/// Resolve one utility list to a styled `Span` for inline text (titles, captions).
fn span<'a>(sheet: &Stylesheet, scratch: &mut ComputeScratch, classes: &[&'static str], text: &'a str) -> Span<'a> {
    Span::styled(text, sheet.compute_with(&NodeRef::new("Div").classes(classes), None, scratch).to_style())
}

fn setup() -> io::Result<Term> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    Terminal::new(backend)
}

fn teardown(term: &mut Term) -> io::Result<()> {
    disable_raw_mode()?;
    execute!(term.backend_mut(), LeaveAlternateScreen)?;
    Ok(())
}
