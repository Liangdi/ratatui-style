//! Server-driven styling via serde — load `CssStyle` from JSON.
//!
//! Demonstrates the "server-driven UI" value prop from design.md §1: deserialize
//! JSON into `CssStyle`, then overlay it via the inline API. Press `c` to cycle
//! through server configs; UI restyles live without recompiling. Also exercises
//! `CascadeContext` for automatic style inheritance down a component tree.
//!
//! ```sh
//! cargo run -p ratatui-style --example 14_data_driven
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
    widgets::{Block, ListItem, Paragraph},
};
use ratatui_style::{CascadeContext, ComputeScratch, CssStyle, OwnedNode, Stylesheet};

type Term = Terminal<CrosstermBackend<Stdout>>;

// Three "server configs" — in production these would come from a network.
const CONFIG_ALERT: &str = r##"{
    "color": "#f38ba8",
    "background": "#45475a",
    "font-weight": "bold"
}"##;

const CONFIG_SUCCESS: &str = r##"{
    "color": "#a6e3a1",
    "background": "#1e1e2e"
}"##;

const CONFIG_WARN: &str = r##"{
    "color": "#f9e2af",
    "background": "#313244",
    "italic": true
}"##;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let sheet = Stylesheet::parse(
        r##"
        :root {
            --bg: #1e1e2e;
            --panel: #313244;
            --text: #cdd6f4;
        }

        Root           { background: var(--bg); }
        Panel          { background: var(--panel); color: var(--text); border: rounded; padding: 1; }
        Item           { color: var(--text); }
        "##,
    )?;

    let mut terminal = setup()?;
    let mut scratch = ComputeScratch::new();
    let mut config_index = 0;
    let configs = [CONFIG_ALERT, CONFIG_SUCCESS, CONFIG_WARN];

    loop {
        let active_json = configs[config_index];
        // Deserialize JSON into CssStyle — serde is on by default.
        let server_style: CssStyle = serde_json::from_str(active_json)?;

        terminal.draw(|f| draw(f, &sheet, &mut scratch, &server_style))?;

        if event::poll(Duration::from_millis(120))?
            && let Event::Key(key) = event::read()?
        {
            match key.code {
                KeyCode::Char('c') => {
                    config_index = (config_index + 1) % configs.len();
                }
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
    _scratch: &mut ComputeScratch,
    server_style: &CssStyle,
) {
    let area = frame.area();

    // Use CascadeContext to walk a component tree with automatic inheritance.
    // This eliminates manual `Some(&parent)` threading.
    let mut ctx = CascadeContext::new(sheet);

    // Root level.
    let root_node = OwnedNode::new("Root");
    let root = ctx.enter(&root_node);
    frame.render_widget(Block::default().style(root.to_style()), area);
    ctx.leave();

    // Outer vertical layout.
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(1), Constraint::Length(2)])
        .split(area);

    // Header (static, no server override).
    let header = Paragraph::new(Line::from(" ◆ Server-driven styling — press c to cycle configs"))
        .style(root.to_style())
        .alignment(Alignment::Left);
    frame.render_widget(header, chunks[0]);

    // Main panel.
    let panel_node = OwnedNode::new("Panel");
    let panel = ctx.enter(&panel_node);

    // Demonstrate the inline overlay API: `with_inline` layers the deserialized
    // JSON style on top of the cascade result. Origin::Inline wins over all
    // stylesheet rules, regardless of specificity.
    let panel_with_server = panel.with_inline(server_style);

    // Render the panel with the server-driven style applied.
    let panel_block = panel_with_server.to_block();
    let inner = panel_block.inner(chunks[1]);
    frame.render_widget(panel_block, chunks[1]);

    // Render a few item rows inside the panel. They inherit from the panel
    // automatically because we're using CascadeContext.
    let items = ["Item 1", "Item 2", "Item 3"];
    let list_items: Vec<ListItem<'_>> = items
        .iter()
        .map(|text| {
            let item_node = OwnedNode::new("Item");
            let item = ctx.enter(&item_node);
            ListItem::new(Line::from(*text)).style(item.to_style())
        })
        .collect();

    let list = ratatui::widgets::List::new(list_items).style(panel_with_server.to_style());
    frame.render_widget(list, inner);

    ctx.leave(); // Leave panel context.

    // Footer showing the current active config.
    let footer_text = match (
        server_style.color.as_ref(),
        server_style.background.as_ref(),
        server_style.weight,
    ) {
        (Some(c), Some(bg), Some(w)) => {
            format!(" Active config: color={c:?}, bg={bg:?}, weight={w:?} — press c to cycle, q to quit ")
        }
        _ => " Active config — press c to cycle, q to quit ".to_string(),
    };

    let footer = Paragraph::new(Line::from(footer_text))
        .style(root.to_style())
        .alignment(Alignment::Center);
    frame.render_widget(footer, chunks[2]);
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
