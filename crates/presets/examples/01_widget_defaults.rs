//! Widget defaults — dress **real ratatui widgets** via the `widgets` preset.
//!
//! The trick: name each style node after the ratatui widget it dresses
//! (`Tabs`, `Tab`, `Table`, `Header`, `Row`, `Gauge`) and the `widgets` preset
//! supplies coherent defaults; the default theme supplies the palette through
//! the shared `var()` tokens. No per-widget CSS is hand-written here — only the
//! node *type names* are chosen, and the resolved style is applied to the
//! ratatui widget with `.style()` / `.highlight_style()` / `.gauge_style()`.
//!
//! ```sh
//! cargo run -p ratatui-style-presets --example 01_widget_defaults --features widgets
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
    layout::{Constraint, Direction, Layout},
    widgets::{Block, Gauge, Row, Table, Tabs},
};
use ratatui_style::{ComputeScratch, NodeRef};

use ratatui_style_presets::{merge, Preset};

type Term = Terminal<CrosstermBackend<Stdout>>;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Default theme (palette + base classes) layered with the widget-type
    // defaults. Each `Preset` is a `&'static Stylesheet`; `merge` stacks them.
    let sheet = merge(&[Preset::Default, Preset::Widgets]);

    let mut terminal = setup()?;
    let mut scratch = ComputeScratch::new();
    loop {
        terminal.draw(|f| draw(f, &sheet, &mut scratch))?;

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

fn draw(frame: &mut ratatui::Frame<'_>, sheet: &ratatui_style::Stylesheet, scratch: &mut ComputeScratch) {
    let area = frame.area();

    let root = sheet.compute_with(&NodeRef::new("Root"), None, scratch);
    frame.render_widget(Block::default().style(root.to_style()), area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0), Constraint::Length(3)])
        .split(area);

    // --- Tabs: the bar style + the active-tab highlight both come from the
    //     preset. `Tabs` (the bar) and `Tab.active` (the selected tab).
    let tabs_style = sheet.compute_with(&NodeRef::new("Tabs"), None, scratch).to_style();
    let active_tab = sheet
        .compute_with(&NodeRef::new("Tab").classes(&["active"]), None, scratch)
        .to_style();
    let tabs = Tabs::new(vec!["Overview", "Details", "Logs"])
        .select(0)
        .style(tabs_style)
        .highlight_style(active_tab);
    frame.render_widget(tabs, chunks[0]);

    // --- Table: `Table`, `Header`, `Row` (+ `.active`) node types.
    let table_style = sheet.compute_with(&NodeRef::new("Table"), None, scratch).to_style();
    let header_style = sheet.compute_with(&NodeRef::new("Header"), None, scratch).to_style();
    let row_style = sheet.compute_with(&NodeRef::new("Row"), None, scratch).to_style();
    let row_active = sheet
        .compute_with(&NodeRef::new("Row").classes(&["active"]), None, scratch)
        .to_style();

    let header = Row::new(vec!["Service", "Status", "Latency"]).style(header_style);
    let r1 = Row::new(vec!["api-gateway", "ok", "12ms"]).style(row_active);
    let r2 = Row::new(vec!["postgres", "ok", "3ms"]).style(row_style);
    let r3 = Row::new(vec!["redis", "degraded", "88ms"]).style(row_style);
    let table = Table::new(
        vec![header, r1, r2, r3],
        [
            Constraint::Length(16),
            Constraint::Length(12),
            Constraint::Min(0),
        ],
    )
    .style(table_style);
    frame.render_widget(table, chunks[1]);

    // --- Gauge: accent fill + accent-fg label, straight from the theme.
    let gauge_style = sheet.compute_with(&NodeRef::new("Gauge"), None, scratch).to_style();
    let gauge = Gauge::default().gauge_style(gauge_style).percent(72);
    frame.render_widget(gauge, chunks[2]);
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
