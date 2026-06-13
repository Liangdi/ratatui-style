//! CSS-driven layout — width/height declarations → ratatui Constraint.
//!
//! Demonstrates CSS `width`/`height` driving ratatui layout via
//! `ComputedStyle::constraints()` (returns `Option<(Constraint, Constraint)>`)
//! and `ComputedStyle::alignment()`. Press `w` to toggle a `.wide` class on the
//! sidebar and watch the layout reshape live from CSS; `q`/`Esc` quits.
//!
//! ```sh
//! cargo run -p ratatui-style --example 13_sizing
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
    widgets::Paragraph,
};

use ratatui_style::{ComputeScratch, NodeRef, Stylesheet};

type Term = Terminal<CrosstermBackend<Stdout>>;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let sheet = Stylesheet::parse(
        r##"
        :root {
            --accent: #89b4fa;
            --bg: #1e1e2e;
            --panel: #313244;
            --text: #cdd6f4;
            --side-w: 22;
        }

        Root           { background: var(--bg); }
        Header         { color: var(--accent); background: var(--panel); font-weight: bold; }
        Sidebar        { width: var(--side-w); background: var(--panel); border: rounded; padding: 1; }
        Sidebar.wide   { width: 40; }
        Main           { background: var(--panel); }
        Panel          { color: var(--text); height: min(3); border: rounded; padding: 1; }
        Panel.short    { height: min(1); }
        Footer         { color: #6c7086; }
        "##,
    )?;

    let mut terminal = setup()?;
    let mut scratch = ComputeScratch::new();
    let mut sidebar_wide = false;

    loop {
        terminal.draw(|f| draw(f, &sheet, &mut scratch, sidebar_wide))?;

        if event::poll(Duration::from_millis(120))?
            && let Event::Key(key) = event::read()?
        {
            match key.code {
                KeyCode::Char('w') => sidebar_wide = !sidebar_wide,
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
    scratch: &mut ComputeScratch,
    sidebar_wide: bool,
) {
    let area = frame.area();
    let root = sheet.compute_with(&NodeRef::new("Root"), None, scratch);
    frame.render_widget(ratatui::widgets::Block::default().style(root.to_style()), area);

    // Outer vertical split: header | body | footer.
    // Footer has a fixed height of 1; body gets Min(1).
    let body = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(1), Constraint::Length(1)])
        .split(area);

    // Header.
    let header = sheet.compute_with(&NodeRef::new("Header"), None, scratch);
    frame.render_widget(
        Paragraph::new(Line::from(" ◆ CSS-driven layout — press w to toggle sidebar width"))
            .style(header.to_style())
            .alignment(Alignment::Left),
        body[0],
    );

    // Body: horizontal split.
    // The critical demo: we derive Constraints from CSS, not hardcoded values!
    let sidebar_classes: &[&str] = if sidebar_wide { &["wide"] } else { &[] };
    let sidebar_node = NodeRef::new("Sidebar").classes(sidebar_classes);
    let sidebar = sheet.compute_with(&sidebar_node, None, scratch);

    // `constraints()` returns the CSS width/height as ratatui Constraints.
    let (width_constraint, _height_constraint) = sidebar.constraints().unwrap_or((
        Constraint::Min(1),
        Constraint::Min(1),
    ));

    let body_split = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([width_constraint, Constraint::Min(1)])
        .split(body[1]);

    // Render sidebar using `render_computed` to exercise the box-model helper.
    use ratatui_style::render_computed;
    render_computed(frame, &sidebar, body_split[0], |_inner, style| {
        Paragraph::new(Line::from("Sidebar"))
            .style(style)
            .alignment(Alignment::Center)
    });

    // Main area: stacked panels.
    let main = sheet.compute_with(&NodeRef::new("Main"), None, scratch);
    frame.render_widget(ratatui::widgets::Block::default().style(main.to_style()), body_split[1]);

    let panel_areas = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(1),
            Constraint::Min(1),
        ])
        .split(body_split[1]);

    // Panel 1 — standard height (min 3 from CSS).
    let panel1 = sheet.compute_with(&NodeRef::new("Panel"), None, scratch);
    // Derive the panel's height from CSS.
    let (_, p1_h) = panel1.constraints().unwrap_or((Constraint::Min(1), Constraint::Min(1)));
    let p1_constraint = match p1_h {
        Constraint::Length(n) => Constraint::Length(n),
        _ => Constraint::Length(3),
    };
    let p1_area = Layout::default()
        .direction(Direction::Vertical)
        .constraints([p1_constraint, Constraint::Min(1)])
        .split(panel_areas[3])[0];
    render_computed(frame, &panel1, p1_area, |_inner, style| {
        Paragraph::new(Line::from("Panel 1 — height: min(3)"))
            .style(style)
    });

    // Panel 2 — short variant (height: min(1)).
    let panel2 = sheet.compute_with(&NodeRef::new("Panel").classes(&["short"]), None, scratch);
    let (_, p2_h) = panel2.constraints().unwrap_or((Constraint::Min(1), Constraint::Min(1)));
    let p2_constraint = match p2_h {
        Constraint::Length(n) => Constraint::Length(n),
        _ => Constraint::Length(1),
    };
    let p2_area = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), p2_constraint, Constraint::Min(1)])
        .split(panel_areas[3])[1];
    render_computed(frame, &panel2, p2_area, |_inner, style| {
        Paragraph::new(Line::from("Panel 2 — height: min(1)"))
            .style(style)
    });

    // Footer.
    let footer = sheet.compute_with(&NodeRef::new("Footer"), None, scratch);
    frame.render_widget(
        Paragraph::new(Line::from(" w=toggle width | q=quit"))
            .style(footer.to_style())
            .alignment(Alignment::Center),
        body[2],
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
