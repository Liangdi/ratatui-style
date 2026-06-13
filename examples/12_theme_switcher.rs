//! L2 — Switch the stylesheet at runtime to restyle the whole UI.
//!
//! Four themes are kept as CSS *content* strings and parsed once at startup
//! into [`Stylesheet`] values. Cycling the active index re-renders the entire
//! dashboard from the newly selected sheet — the markup (node names, classes,
//! ids, pseudo-states) is identical across themes; only the CSS differs. This
//! is the purest "switch CSS → change style" demonstration.
//!
//! You can also hand it `.css` files on the command line — each is parsed with
//! the same [`Stylesheet::parse`] and appended to the theme list, so switching
//! between *files* works exactly like switching between the built-in contents.
//!
//! ```sh
//! # built-in themes only
//! cargo run -p ratatui-style --example 12_theme_switcher
//!
//! # add your own CSS files as extra switchable themes
//! echo 'Root { background: #1a1a2e; } Header { color: #e94560; }' > /tmp/red.css
//! cargo run -p ratatui-style --example 12_theme_switcher -- /tmp/red.css
//! ```
//!
//! Keys: `←/→` switch theme · `1`-`9` jump to theme · `Tab` move focus · `q`/`Esc` quit.

use std::io::{self, Stdout};
use std::path::Path;
use std::time::Duration;

use crossterm::{
    event::{self, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    Terminal, backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    text::Line,
    widgets::{Block, Paragraph},
};

use ratatui_style::{OwnedNode, State, Stylesheet};

type Term = Terminal<CrosstermBackend<Stdout>>;

/// A theme is just a name + its CSS content. Selectors are identical across
/// every entry, so swapping the sheet changes every color/border at once.
const BUILTIN_THEMES: &[(&str, &str)] = &[
    ("Catppuccin Mocha", r##"
        :root {
            --accent: #89b4fa;
            --bg: #1e1e2e;
            --panel: #313244;
            --text: #cdd6f4;
            --muted: #6c7086;
        }
        Root            { background: var(--bg); }
        Header          { color: var(--accent); background: var(--panel); font-weight: bold; }
        Panel           { color: var(--text); border: rounded var(--panel); padding: 1 2; }
        PanelTitle      { color: var(--accent); font-weight: bold; }
        Text            { color: var(--text); }
        Label           { color: var(--muted); }
        Button          { color: var(--text); padding: 0 2; }
        Button.primary  { background: var(--accent); color: #1e1e2e; font-weight: bold; }
        Button.ghost    { border: rounded var(--muted); }
        Button:focus            { background: var(--panel); color: var(--accent); }
        Button.primary:focus    { background: #b4befe; }
        Footer          { color: var(--muted); }
        Hint            { color: var(--accent); }
    "##),
    ("Gruvbox Dark", r##"
        :root {
            --accent: #fabd2f;
            --bg: #282828;
            --panel: #3c3836;
            --text: #ebdbb2;
            --muted: #928374;
        }
        Root            { background: var(--bg); }
        Header          { color: var(--accent); background: var(--panel); font-weight: bold; }
        Panel           { color: var(--text); border: plain var(--muted); padding: 1 2; }
        PanelTitle      { color: var(--accent); font-weight: bold; }
        Text            { color: var(--text); }
        Label           { color: var(--muted); }
        Button          { color: var(--text); padding: 0 2; }
        Button.primary  { background: var(--accent); color: #282828; font-weight: bold; }
        Button.ghost    { border: plain var(--muted); }
        Button:focus            { background: var(--panel); color: var(--accent); }
        Button.primary:focus    { background: #d79921; }
        Footer          { color: var(--muted); }
        Hint            { color: var(--accent); }
    "##),
    ("Tokyo Night", r##"
        :root {
            --accent: #7aa2f7;
            --bg: #1a1b26;
            --panel: #24283b;
            --text: #c0caf5;
            --muted: #565f89;
        }
        Root            { background: var(--bg); }
        Header          { color: var(--accent); background: var(--panel); font-weight: bold; }
        Panel           { color: var(--text); border: double var(--panel); padding: 1 2; }
        PanelTitle      { color: var(--accent); font-weight: bold; }
        Text            { color: var(--text); }
        Label           { color: var(--muted); }
        Button          { color: var(--text); padding: 0 2; }
        Button.primary  { background: var(--accent); color: #1a1b26; font-weight: bold; }
        Button.ghost    { border: double var(--muted); }
        Button:focus            { background: var(--panel); color: var(--accent); }
        Button.primary:focus    { background: #bb9af7; }
        Footer          { color: var(--muted); }
        Hint            { color: var(--accent); }
    "##),
    ("Solarized Light", r##"
        :root {
            --accent: #268bd2;
            --bg: #fdf6e3;
            --panel: #eee8d5;
            --text: #586e75;
            --muted: #93a1a1;
        }
        Root            { background: var(--bg); }
        Header          { color: var(--accent); background: var(--panel); font-weight: bold; }
        Panel           { color: var(--text); border: rounded var(--muted); padding: 1 2; }
        PanelTitle      { color: var(--accent); font-weight: bold; }
        Text            { color: var(--text); }
        Label           { color: var(--muted); }
        Button          { color: var(--text); padding: 0 2; }
        Button.primary  { background: var(--accent); color: #fdf6e3; font-weight: bold; }
        Button.ghost    { border: rounded var(--muted); }
        Button:focus            { background: var(--panel); color: var(--accent); }
        Button.primary:focus    { background: #2aa198; }
        Footer          { color: var(--muted); }
        Hint            { color: var(--accent); }
    "##),
];

struct Button {
    label: &'static str,
    class: &'static str,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Parse every theme once. Switching themes later is just indexing into
    // this Vec — no re-parsing, no allocation in the render loop.
    let mut themes: Vec<(String, Stylesheet)> = Vec::new();
    for (name, css) in BUILTIN_THEMES {
        themes.push(((*name).to_string(), Stylesheet::parse(css).expect("builtin theme parses")));
    }

    // Extra themes come from `.css` files passed on the command line. The same
    // `Stylesheet::parse` handles file contents and inline content identically.
    for path in std::env::args().skip(1) {
        match std::fs::read_to_string(&path) {
            Ok(css) => match Stylesheet::parse(&css) {
                Ok(sheet) => {
                    let name = Path::new(&path)
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or(&path)
                        .to_string();
                    themes.push((name, sheet));
                }
                Err(e) => eprintln!("warning: failed to parse {path}: {e}"),
            },
            Err(e) => eprintln!("warning: cannot read {path}: {e}"),
        }
    }

    let buttons = [
        Button { label: "Open", class: "primary" },
        Button { label: "Save", class: "ghost" },
        Button { label: "Export", class: "ghost" },
    ];
    let n_btns = buttons.len();

    let mut theme = 0usize;
    let mut focus = 0usize;

    let mut terminal = setup()?;
    loop {
        terminal.draw(|f| draw(f, &themes[theme].1, &themes[theme].0, theme, themes.len(), &buttons, focus))?;

        if !event::poll(Duration::from_millis(100))? {
            continue;
        }
        if let Event::Key(key) = event::read()? {
            match key.code {
                // Switch theme — the headline action.
                KeyCode::Right => theme = (theme + 1) % themes.len(),
                KeyCode::Left => theme = (theme + themes.len() - 1) % themes.len(),
                // Jump straight to a theme by number.
                KeyCode::Char(c @ '1'..='9') => {
                    let i = (c as u8 - b'1') as usize;
                    if i < themes.len() {
                        theme = i;
                    }
                }
                // Move button focus to show :focus restyle within each theme.
                KeyCode::Tab => focus = (focus + 1) % n_btns,
                KeyCode::BackTab => focus = (focus + n_btns - 1) % n_btns,
                KeyCode::Char('q') | KeyCode::Esc => break,
                _ => {}
            }
        }
    }
    teardown(&mut terminal)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn draw(
    frame: &mut ratatui::Frame<'_>,
    sheet: &Stylesheet,
    theme_name: &str,
    theme_idx: usize,
    theme_count: usize,
    buttons: &[Button],
    focus: usize,
) {
    let area = frame.area();

    // Root background fill — changes per theme.
    let root = sheet.compute(&OwnedNode::new("Root"), None);
    frame.render_widget(Block::default().style(root.to_style()), area);

    let cols = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(1), Constraint::Length(3)])
        .split(area);

    // Header: shows which theme is active.
    let header = sheet.compute(&OwnedNode::new("Header"), None);
    frame.render_widget(
        Paragraph::new(Line::from(format!(
            " ◆ {theme_name}   [{theme_idx}/{theme_count}]"
        )))
        .style(header.to_style()),
        cols[0],
    );

    // Panel with a title and a button row inside it.
    let panel = sheet.compute(&OwnedNode::new("Panel"), None);
    let inner = panel_block(frame, panel.to_block(), cols[1]);

    let title = sheet.compute(&OwnedNode::new("PanelTitle"), None);
    let text = sheet.compute(&OwnedNode::new("Text"), None);
    let label = sheet.compute(&OwnedNode::new("Label"), None);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1), Constraint::Min(1), Constraint::Length(3)])
        .split(inner);

    frame.render_widget(Paragraph::new(Line::from(" Switch the stylesheet to restyle everything").style(title.to_style())), rows[0]);
    frame.render_widget(Paragraph::new(Line::from(" Same markup, different CSS → different look").style(label.to_style())), rows[1]);

    let body = [
        " The node names, classes, and ids are identical across themes.",
        " Only the CSS content changes — colors, borders, weights all swap.",
        " Focus a button (Tab) to see :focus restyle within each theme.",
    ];
    let body_lines: Vec<Line<'_>> = body.into_iter().map(|l| Line::from(l).style(text.to_style())).collect();
    frame.render_widget(ratatui::widgets::List::new(body_lines), rows[2]);

    // Button row.
    let btn_rects = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(34), Constraint::Percentage(33), Constraint::Percentage(33)])
        .split(rows[3]);

    for (i, b) in buttons.iter().enumerate() {
        let node = OwnedNode::new("Button")
            .with_classes([b.class])
            .with_state(State {
                focus: i == focus,
                ..State::empty()
            });
        let computed = sheet.compute(&node, None);
        let para = Paragraph::new(Line::from(format!(" {} ", b.label)))
            .style(computed.to_style())
            .alignment(Alignment::Center);
        frame.render_widget(para.block(computed.to_block()), btn_rects[i]);
    }

    // Footer hint.
    let footer = sheet.compute(&OwnedNode::new("Footer"), None);
    frame.render_widget(
        Paragraph::new(Line::from(format!(
            " {:<-10} switch theme · {:<-8} jump · {:<-10} move focus · q to quit",
            "←/→",
            format!("1-{}", theme_count.min(9)),
            "Tab"
        )))
        .style(footer.to_style()),
        cols[2],
    );
}

/// Render a panel `Block`, returning the inner content area.
fn panel_block(frame: &mut ratatui::Frame<'_>, block: Block<'_>, area: Rect) -> Rect {
    let inner = block.inner(area);
    frame.render_widget(block, area);
    inner
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
