//! L2 — Compile-time embedding + runtime override, rendered as a live TUI.
//!
//! Embeds a CSS theme at compile time via the `css!` macro (rules tagged
//! `Origin::Theme`), then optionally layers a runtime CSS file on top (rules
//! tagged `Origin::User`) via `RuntimeStyle`. Pass a `.css` path to override.
//!
//! ```sh
//! # embedded theme only
//! cargo run -p ratatui-style --example 09_runtime_override
//!
//! # override Button.primary at runtime (e.g. flip accent to red)
//! echo 'Button.primary { border-color: red; color: red; }' > /tmp/override.css
//! cargo run -p ratatui-style --example 09_runtime_override -- /tmp/override.css
//! ```

use std::io::{self, Stdout};
use std::path::Path;
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

use ratatui_style::{css, ComputeScratch, NodeRef, RuntimeStyle, Stylesheet};

type Term = Terminal<CrosstermBackend<Stdout>>;

/// Embedded at compile time — `theme.css` sits next to this source file.
static THEME: LazyLock<Stylesheet> = css!("theme.css");

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut style = RuntimeStyle::new(&THEME);
    let mut override_msg = String::from("embedded theme only (no override)");

    if let Some(path) = std::env::args().nth(1) {
        let p = Path::new(&path);
        match style.load_override(p) {
            Ok(()) if style.has_override() => {
                override_msg = format!("override loaded: {}", p.display());
            }
            Ok(()) => {
                override_msg = format!("override not found, using embedded: {}", p.display());
            }
            Err(e) => {
                eprintln!("error loading runtime CSS: {e}");
                std::process::exit(1);
            }
        }
    }

    let mut terminal = setup()?;
    // Reused across every frame — zero allocation once warmed up.
    let mut scratch = ComputeScratch::new();
    loop {
        terminal.draw(|f| draw(f, &style, &override_msg, &mut scratch))?;
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

fn draw(frame: &mut ratatui::Frame<'_>, style: &RuntimeStyle, override_msg: &str, scratch: &mut ComputeScratch) {
    let area = frame.area();
    let root = style.compute_with(&NodeRef::new("Root"), None, scratch);
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
    let header = style.compute_with(&NodeRef::new("Header"), None, scratch);
    frame.render_widget(
        Paragraph::new(Line::from(" ◆ css! — embedded + runtime override"))
            .style(header.to_style()),
        chunks[0],
    );

    // Button row: Button · Button.primary · #save
    let btn_rects = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(34), Constraint::Percentage(33), Constraint::Percentage(33)])
        .split(chunks[1]);

    // NodeRef borrows &'static str — zero allocation per frame.
    let nodes: [(NodeRef<'_>, &str); 3] = [
        (NodeRef::new("Button"), "Button"),
        (NodeRef::new("Button").classes(&["primary"]), "primary"),
        (NodeRef::new("Button").id("save"), "#save"),
    ];
    for (rect, (node, label)) in btn_rects.iter().zip(nodes.iter()) {
        let computed = style.compute_with(node, None, scratch);
        let para = Paragraph::new(Line::from(format!(" {label} ")))
            .style(computed.to_style())
            .alignment(Alignment::Center);
        frame.render_widget(para.block(computed.to_block()), *rect);
    }

    // Status line.
    let text = style.compute_with(&NodeRef::new("Text"), None, scratch);
    let lines = [
        format!(" • status: {override_msg}"),
        " • try: echo 'Button.primary { color: red; }' > o.css".to_string(),
        " • then rerun with that path as the first argument".to_string(),
    ];
    let merged: Vec<Line<'_>> = lines.into_iter().map(|l| Line::from(l).style(text.to_style())).collect();
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
