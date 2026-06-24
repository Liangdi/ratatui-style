//! `opacity` → `Modifier::DIM` — the terminal's approximation of CSS alpha.
//!
//! Terminals have no alpha channel, so `opacity` collapses to a single DIM
//! bit: any value below fully opaque (`0.5`, `50%`, `0`) dims the cell, while
//! `1` / `100%` / `normal` stays bright. Like real CSS, `opacity` does **not**
//! inherit.
//!
//! This renders three identical panels at `opacity: 1` / `0.5` / `0` side by
//! side so the DIM progression is visible — same text, same color, only the
//! opacity differs. Press `q` or `Esc` to quit.
//!
//! ```sh
//! cargo run -p ratatui-style --example 22_opacity
//! ```

use std::io::{self, Stdout};
use std::time::Duration;

use crossterm::{
    event::{self, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    layout::{Constraint, Layout},
    Terminal, backend::CrosstermBackend, text::Line, widgets::Paragraph,
};

use ratatui_style::{OwnedNode, Stylesheet};

type Term = Terminal<CrosstermBackend<Stdout>>;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // One stylesheet, parsed from CSS text so the property syntax is the star
    // of the example. A base `Panel` look, plus three opacity variants.
    let sheet = Stylesheet::parse(
        r#"
        Panel {
            color: #cdd6f4;
            background: #1e1e2e;
            border: rounded #45475a;
            padding: 1;
        }
        /* opacity < 1 collapses to DIM (terminals have no alpha); 1/100%/normal
         * add no modifier. */
        .full { opacity: 1; }
        .half { opacity: 0.5; }
        .zero { opacity: 0; }
        "#,
    )?;

    // Resolve each variant once — the computed styles are reused every frame.
    let full = sheet.compute(&OwnedNode::new("Panel").with_classes(["full"]), None);
    let half = sheet.compute(&OwnedNode::new("Panel").with_classes(["half"]), None);
    let zero = sheet.compute(&OwnedNode::new("Panel").with_classes(["zero"]), None);

    let mut terminal = setup()?;
    loop {
        terminal.draw(|f| {
            // Three equal columns.
            let cols = Layout::horizontal([Constraint::Fill(1); 3])
                .spacing(1)
                .split(f.area());
            let panels = [
                ("opacity: 1", &full),
                ("opacity: 0.5", &half),
                ("opacity: 0", &zero),
            ];
            for ((label, computed), col) in panels.into_iter().zip(cols.iter()) {
                // `layout` hands back the block, the content style, and the
                // inner rect in one call.
                let (block, content_style, inner) = computed.layout(*col);
                f.render_widget(block.title(format!(" {label} ")), *col);
                f.render_widget(
                    Paragraph::new(Line::from("Same text, same color —\nonly opacity differs."))
                        .style(content_style),
                    inner,
                );
            }
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
