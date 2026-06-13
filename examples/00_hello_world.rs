//! Hello, World! — the smallest possible ratatui-style TUI app.
//!
//! Define one CSS rule, resolve it for a node, and render a styled
//! "Hello, World!" panel. This is a real terminal render, so the colors show
//! up natively. Press `q` or `Esc` to quit.
//!
//! ```sh
//! cargo run -p ratatui-style --example 00_hello_world
//! ```

use std::io::{self, Stdout};
use std::time::Duration;

use crossterm::{
    event::{self, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    Terminal, backend::CrosstermBackend, layout::Rect, text::Line, widgets::Paragraph,
};

use ratatui_style::{BorderStyle, CssStyle, Origin, OwnedNode, Stylesheet};

type Term = Terminal<CrosstermBackend<Stdout>>;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. One CSS rule: any `Box` node gets a rounded cyan border on a dark panel.
    let mut sheet = Stylesheet::new();
    sheet.add(
        "Box",
        CssStyle::new()
            .color("#00d4ff")
            .background("#1e1e2e")
            .border((BorderStyle::Rounded, "#00d4ff"))
            .padding(1u16),
        Origin::User,
    )?;

    // 2. Ask the cascade engine for the computed style of a `Box` node.
    let computed = sheet.compute(&OwnedNode::new("Box"), None);

    // 3. Render it every frame until the user quits.
    let mut terminal = setup()?;
    loop {
        terminal.draw(|f| {
            let area = centered(34, 5, f.area());
            // One call gives the block, the content style, and the inner rect.
            let (block, content_style, inner) = computed.layout(area);
            f.render_widget(block.title(" ratatui-style "), area);
            f.render_widget(
                Paragraph::new(Line::from("Hello, World!")).style(content_style),
                inner,
            );
        })?;

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

/// Center a `w` × `h` rect inside `area`.
fn centered(w: u16, h: u16, area: Rect) -> Rect {
    let x = area.x + area.width.saturating_sub(w) / 2;
    let y = area.y + area.height.saturating_sub(h) / 2;
    Rect::new(x, y, w.min(area.width), h.min(area.height))
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
