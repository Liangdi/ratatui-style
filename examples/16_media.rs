//! P3–P6 — Responsive styling via `@media`.
//!
//! The whole point of `@media` in a terminal: the UI restyles **and** relayouts
//! as the terminal is resized, plus a color-capability branch for truecolor vs
//! 16-color. This example drives the cascade with a host-supplied
//! [`MediaContext`] (terminal `cols`/`rows`/`truecolor`/`no_color`) taken from
//! `frame.area()` every render.
//!
//! What it shows, mapped to the roadmap:
//!
//! | Roadmap | Feature | Here |
//! |---------|---------|------|
//! | P3 | `@media (min-width: n)` / `(max-width: n)` | wide/medium/narrow breakpoints |
//! | P4 | `@media` `not` / `and` / comma (OR)        | `not (truecolor)`, `(min-width) and (min-height)` |
//! | P4 | media-scoped `:root` tokens                | `--accent` overridden inside a breakpoint |
//! | P6 | per-term `not`                              | `@media not (truecolor) { … }` |
//!
//! `←/→` resize the *virtual* terminal (so it works in any real size);
//! `t` toggles the truecolor flag to flip the palette branch; `q`/`Esc` quits.
//!
//! ```sh
//! cargo run -p ratatui-style --example 16_media
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
    layout::{Alignment, Constraint, Direction, Layout},
    text::Line,
    widgets::{Block, Paragraph},
};

use ratatui_style::{CascadeContext, MediaContext, NodeRef, Stylesheet};

type Term = Terminal<CrosstermBackend<Stdout>>;

const SHEET: &str = r##"
:root {
    --bg: #1e1e2e;
    --panel: #313244;
    --text: #cdd6f4;
    --accent: #89b4fa;     /* truecolor default; narrowed below on 16-color terms */
    --muted: #6c7086;
}

Root    { background: var(--bg); color: var(--text); }
Header  { color: var(--accent); background: var(--panel); font-weight: bold; }
Footer  { color: var(--muted); }
Card    { color: var(--text); background: var(--panel); border: rounded var(--accent); padding: 0 1; }
Card.title { color: var(--accent); font-weight: bold; }

/* ---- P3: size breakpoints ------------------------------------------------ *
 * Wide terminals get a richer accent; narrow terminals collapse padding and
 * switch to a muted border so three cards still fit.                         */

/* comma = OR: medium OR wide keeps the strong accent. */
@media (min-width: 60), (min-height: 30) {
    :root { --accent: #b4befe; }
}

/* Narrow: compact everything, demote the border color. */
@media (max-width: 39) {
    :root { --accent: #6c7086; }
    Card  { padding: 0; border: rounded var(--muted); }
    Card.title { color: var(--text); }
}

/* ---- P4: `and` conjunction + per-term `not` ------------------------------ *
 * Only when BOTH width and height are generous do we add bold titles; and the
 * color-capability branch uses per-term `not` so a 16-color term falls back.  */

@media (min-width: 60) and (min-height: 24) {
    Card.title { font-weight: bold; }
}

/* truecolor terminal → vivid accent. `not` binds to this one term only. */
@media (truecolor) {
    :root { --accent: #89b4fa; }
}
@media not (truecolor) {
    :root { --accent: blue; }   /* 16-color named fallback */
}
"##;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let sheet = Stylesheet::parse(SHEET)?;

    // We drive a *virtual* terminal size so the demo is reproducible in any
    // real terminal. ←/→ shrinks/grows it; the cascade responds every frame.
    let mut cols: u16 = 80;
    let mut rows: u16 = 24;
    let mut truecolor = true;

    let mut terminal = setup()?;
    loop {
        // Build the MediaContext the cascade gates `@media` against. In a real
        // app this is just `MediaContext { cols: area.width, rows: area.height, .. }`
        // read off `frame.area()`.
        let media = MediaContext { cols, rows, truecolor, no_color: false };

        terminal.draw(|f| draw(f, &sheet, media, cols, rows))?;

        if event::poll(Duration::from_millis(120))?
            && let Event::Key(key) = event::read()?
        {
            match key.code {
                KeyCode::Left => cols = cols.saturating_sub(4).max(12),
                KeyCode::Right => cols = cols.saturating_add(4).min(120),
                KeyCode::Down => rows = rows.saturating_sub(1).max(6),
                KeyCode::Up => rows = rows.saturating_add(1).min(48),
                KeyCode::Char('t') => truecolor = !truecolor,
                KeyCode::Char('q') | KeyCode::Esc => break,
                _ => {}
            }
        }
    }
    teardown(&mut terminal)?;
    Ok(())
}

fn draw(
    frame: &mut ratatui::Frame<'_>,
    sheet: &Stylesheet,
    media: MediaContext,
    cols: u16,
    rows: u16,
) {
    let area = frame.area();

    // A fresh CascadeContext per frame, told the current terminal capabilities.
    // `enter`/`leave` walk a tiny tree so children inherit from the root; the
    // `@media` rules are gated against `media`.
    let mut ctx = CascadeContext::new(sheet).with_media(media);

    let root = ctx.enter(&NodeRef::new("Root"));
    frame.render_widget(Block::default().style(root.to_style()), area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(1), Constraint::Length(2)])
        .split(area);

    // Header reports the active context so you can see which breakpoint fired.
    let header = ctx.enter(&NodeRef::new("Header"));
    let label = format!(" Responsive @media  ·  {cols}×{rows}  ·  truecolor={}", media.truecolor);
    frame.render_widget(
        Paragraph::new(Line::from(format!(" {label}"))).style(header.to_style()),
        chunks[0],
    );
    ctx.leave();

    // The app decides the LAYOUT (column count) from width; the cascade decides
    // the STYLE (colors/padding/border) from @media rules. Both react to size.
    let n_cards = if cols >= 60 { 3 } else if cols >= 30 { 2 } else { 1 };
    let card_rects = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(std::iter::repeat(Constraint::Percentage(100 / n_cards as u16)).take(n_cards))
        .horizontal_margin(1)
        .split(chunks[1]);

    let titles = ["Alpha", "Beta", "Gamma"];
    for (i, rect) in card_rects.iter().enumerate() {
        let card = ctx.enter(&NodeRef::new("Card"));
        let title = ctx.enter(&NodeRef::new("Card").classes(&["title"]));

        let block = card.to_block().title(format!(" {} ", titles[i]));
        let inner = block.inner(*rect);
        frame.render_widget(block, *rect);

        // The body line shows which breakpoint governed this card's accent.
        let branch = if cols <= 39 {
            "narrow breakpoint (≤39)"
        } else if cols >= 60 || rows >= 30 {
            "medium/wide breakpoint"
        } else {
            "base rules"
        };
        frame.render_widget(
            Paragraph::new(Line::from(format!(" accent: {}\n rules: {branch}", accent_name(media))))
                .style(title.to_style()),
            inner,
        );
        ctx.leave(); // Card.title
        ctx.leave(); // Card
    }

    let footer = ctx.enter(&NodeRef::new("Footer"));
    frame.render_widget(
        Paragraph::new(Line::from(format!(
            " {:<-9} resize · {:<-12} toggle truecolor · q to quit",
            "←/→ ↑/↓", "t"
        )))
        .style(footer.to_style())
        .alignment(Alignment::Center),
        chunks[2],
    );
    ctx.leave();
}

/// Human label for the resolved accent, so the on-screen text reflects which
/// `@media` color branch fired (truecolor → vivid; 16-color → `blue`).
fn accent_name(m: MediaContext) -> &'static str {
    if m.truecolor { "#89b4fa (truecolor)" } else { "blue (16-color fallback)" }
}

fn setup() -> io::Result<Term> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    Terminal::new(CrosstermBackend::new(stdout))
}

fn teardown(term: &mut Term) -> io::Result<()> {
    disable_raw_mode()?;
    execute!(term.backend_mut(), LeaveAlternateScreen)?;
    Ok(())
}
