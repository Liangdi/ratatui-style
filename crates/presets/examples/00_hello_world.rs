//! Hello, presets! — the smallest possible app using `ratatui-style-presets`.
//!
//! No hand-written CSS: load the bundled default theme, ask it for the computed
//! style of a `Panel` node, and render a styled "Hello, presets!" panel. The
//! whole look (rounded border, surface fill, padding, text color) comes from the
//! preset — this is a real terminal render, so the colors show up natively.
//!
//! Press `q` or `Esc` to quit.
//!
//! ```sh
//! cargo run -p ratatui-style-presets --example 00_hello_world
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

use ratatui_style::OwnedNode;
use ratatui_style_presets::default_theme;

type Term = Terminal<CrosstermBackend<Stdout>>;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. The bundled default theme + base component classes — no CSS to write.
    //    `default_theme()` is always available (no feature flag).
    let sheet = default_theme();

    // 2. Ask the cascade for the computed style of a `Panel` node. Every field
    //    below (border, background, padding, inherited text color) is resolved
    //    from the preset's `:root` tokens + `.Panel` rule.
    let computed = sheet.compute(&OwnedNode::new("Panel"), None);

    // 3. Render it every frame until the user quits.
    let mut terminal = setup()?;
    loop {
        terminal.draw(|f| {
            let area = centered(38, 5, f.area());
            // One call yields the block, the content style, and the inner rect.
            let (block, content_style, inner) = computed.layout(area);
            f.render_widget(block.title(" presets "), area);
            f.render_widget(
                Paragraph::new(Line::from("Hello, presets!")).style(content_style),
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
