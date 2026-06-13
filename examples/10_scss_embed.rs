//! `scss` feature — compile-time SCSS embedding rendered as a live TUI.
//!
//! The `.scss` source is embedded at build time; `grass` compiles it to CSS on
//! first access. The TUI shows buttons + text driven by `$variables`, `&`
//! nesting, and `#{$var}` interpolation.
//!
//! ```sh
//! cargo run -p ratatui-style --example 10_scss_embed --features scss
//! ```

use std::io::{self, Stdout};
use std::sync::LazyLock;
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

use ratatui_style::{ComputeScratch, NodeRef, scss, State, Stylesheet};

type Term = Terminal<CrosstermBackend<Stdout>>;

/// Embedded + compiled from SCSS. `scss_embed.scss` sits next to this file.
static THEME: LazyLock<Stylesheet> = scss!("scss_embed.scss");

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut terminal = setup()?;
    let mut scratch = ComputeScratch::new();
    loop {
        terminal.draw(|f| draw(f, &THEME, &mut scratch))?;
        if event::poll(Duration::from_millis(120))?
            && let Event::Key(key) = event::read()?
            && matches!(key.code, KeyCode::Char('q') | KeyCode::Esc)
        {
            break;
        }
    }
    teardown(&mut terminal)?;
    Ok(())
}

fn draw(frame: &mut ratatui::Frame<'_>, sheet: &Stylesheet, scratch: &mut ComputeScratch) {
    let area = frame.area();
    let root = sheet.compute_with(&NodeRef::new("Root"), None, scratch);
    frame.render_widget(Block::default().style(root.to_style()), area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(area);

    // Header.
    let header = sheet.compute_with(&NodeRef::new("Header"), None, scratch);
    frame.render_widget(
        Paragraph::new(Line::from(" ◆ scss! — compiled by grass"))
            .style(header.to_style()),
        chunks[0],
    );

    // Button row: Button · Button.primary · Button.primary:focus
    let btn_rects = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(34), Constraint::Percentage(33), Constraint::Percentage(33)])
        .split(chunks[1]);

    let buttons: [(bool, bool, &str); 3] = [
        (false, false, "Button"),
        (true, false, "primary"),
        (true, true, "primary + focus"),
    ];
    for (rect, (primary, focus, label)) in btn_rects.iter().zip(buttons.iter()) {
        let classes: &[&str] = if *primary { &["primary"] } else { &[] };
        let node = NodeRef::new("Button")
            .classes(classes)
            .state(if *focus { State::focus() } else { State::empty() });
        let computed = sheet.compute_with(&node, None, scratch);
        let para = Paragraph::new(Line::from(format!(" {label} ")))
            .style(computed.to_style())
            .alignment(Alignment::Center);
        frame.render_widget(para.block(computed.to_block()), *rect);
    }

    // Text showcase.
    let text = sheet.compute_with(&NodeRef::new("Text"), None, scratch);
    let lines = [
        "SCSS $variables resolve at compile time (grass)",
        "Nesting (&.primary, &:focus) flattens to plain CSS",
        ":root tokens via #{$accent} interpolation",
    ];
    let mut merged: Vec<Line<'_>> = Vec::new();
    for l in lines {
        merged.push(Line::from(format!(" • {l}")).style(text.to_style()));
    }
    frame.render_widget(ratatui::widgets::List::new(merged), chunks[2]);

    // Footer.
    frame.render_widget(
        Paragraph::new(Line::from(" q to quit")).style(text.to_style()),
        chunks[3],
    );
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
