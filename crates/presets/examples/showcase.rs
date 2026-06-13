//! Showcase of `ratatui-style-presets` — an **interactive** theme switcher.
//!
//! Renders a panel (header, list, button row, footer) from CSS and drives the
//! cascade's pseudo-states live: navigate the list (`↑`/`↓`), move button focus
//! (`←`/`→`), toggle `:disabled` (`d`), and switch the active palette (`1`–`9`
//! / `Tab`) to watch the whole frame restyle. The `widgets` preset (when
//! enabled) layers ratatui-widget defaults on top; every color flows from the
//! active theme's `:root` tokens, so swapping the base preset restyles the whole
//! UI with no other changes.
//!
//! ```sh
//! cargo run -p ratatui-style-presets --example showcase
//! cargo run -p ratatui-style-presets --example showcase --features widgets
//! cargo run -p ratatui-style-presets --example showcase --all-features
//! ```
//!
//! Keys: `↑`/`↓` list · `←`/`→` button focus · `d` toggle disabled · `1`–`9`
//! theme · `Tab` cycle theme · `q`/`Esc` quits.

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
    text::{Line, Text},
    widgets::{List, ListItem, Paragraph},
};
use ratatui_style::{ComputeScratch, NodeRef, State, Stylesheet};

use ratatui_style_presets::{merge, Preset};

type Term = Terminal<CrosstermBackend<Stdout>>;

const LIST_LEN: usize = 4;
const BTN_LEN: usize = 4;

/// The themes available in this build, in selection order. Only presets whose
/// feature is enabled appear.
fn available_themes() -> Vec<(&'static str, Preset)> {
    // A cfg-gated array literal keeps the list declaration-style (no mut/push),
    // so it compiles cleanly under any feature combination.
    [
        ("Default", Preset::Default),
        #[cfg(feature = "catppuccin")]
        ("Catppuccin", Preset::Catppuccin),
        #[cfg(feature = "nord")]
        ("Nord", Preset::Nord),
        #[cfg(feature = "dracula")]
        ("Dracula", Preset::Dracula),
    ]
    .into_iter()
    .collect()
}

/// One theme + the widget defaults (if enabled) merged into a usable sheet.
fn build_sheet(theme: Preset) -> Stylesheet {
    #[cfg(feature = "widgets")]
    let presets: &[Preset] = &[theme, Preset::Widgets];
    #[cfg(not(feature = "widgets"))]
    let presets: &[Preset] = &[theme];
    merge(presets)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let themes = available_themes();
    let n_themes = themes.len();
    let mut cur = 0usize; // active theme index
    let mut sheet = build_sheet(themes[cur].1);

    let mut selected = 0usize; // list selection (ListRow.active)
    let mut focus = 0usize; // button focus (Button:focus)
    let mut disabled = false; // global :disabled toggle

    let mut terminal = setup()?;
    let mut scratch = ComputeScratch::new();
    loop {
        terminal.draw(|f| {
            draw(f, &sheet, &themes, cur, selected, focus, disabled, &mut scratch)
        })?;

        if !event::poll(Duration::from_millis(120))? {
            continue;
        }
        let Event::Key(key) = event::read()? else {
            continue;
        };
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => break,

            // List navigation — moves which row carries the `active` class.
            KeyCode::Up | KeyCode::Char('k') => selected = (selected + LIST_LEN - 1) % LIST_LEN,
            KeyCode::Down | KeyCode::Char('j') => selected = (selected + 1) % LIST_LEN,

            // Button focus — moves which button is `:focus`.
            KeyCode::Left | KeyCode::Char('h') => focus = (focus + BTN_LEN - 1) % BTN_LEN,
            KeyCode::Right | KeyCode::Char('l') => focus = (focus + 1) % BTN_LEN,

            // Toggle :disabled across the button row.
            KeyCode::Char('d') => disabled = !disabled,

            // Theme switching.
            KeyCode::Tab => {
                cur = (cur + 1) % n_themes;
                sheet = build_sheet(themes[cur].1);
            }
            KeyCode::Char(c) if c.is_ascii_digit() => {
                // '1'..'9' → index 0..8 (only switches if in range).
                let idx = (c as u8 - b'1') as usize;
                if idx < n_themes {
                    cur = idx;
                    sheet = build_sheet(themes[cur].1);
                }
            }
            _ => {}
        }
    }
    teardown(&mut terminal)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn draw(
    frame: &mut ratatui::Frame<'_>,
    sheet: &Stylesheet,
    themes: &[(&'static str, Preset)],
    cur: usize,
    selected: usize,
    focus: usize,
    disabled: bool,
    scratch: &mut ComputeScratch,
) {
    let area = frame.area();

    let root = sheet.compute_with(&NodeRef::new("Root"), None, scratch);
    frame.render_widget(ratatui::widgets::Block::default().style(root.to_style()), area);

    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(1), Constraint::Length(2)])
        .split(area);

    // Header names the active theme.
    let header = sheet.compute_with(&NodeRef::new("Header"), None, scratch);
    frame.render_widget(
        Paragraph::new(Line::from(format!(
            " ◆ ratatui-style-presets · {}",
            themes[cur].0
        )))
        .style(header.to_style()),
        outer[0],
    );

    // Body: a panel with a list + a button row.
    let panel = sheet.compute_with(&NodeRef::new("Panel"), None, scratch);
    let block = panel.to_block();
    let inner = block.inner(outer[1]);
    frame.render_widget(block, outer[1]);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(3)])
        .split(inner);

    // Long-lived class slices (NodeRef borrows them).
    let active_cls: &[&str] = &["active"];
    let empty_cls: &[&str] = &[];

    // List — the selected row carries the `active` class; the rest inherit.
    let rows = ["Inbox", "Drafts", "Sent", "Archive"];
    let items: Vec<ListItem<'_>> = rows
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let cls = if i == selected { active_cls } else { empty_cls };
            let st = sheet
                .compute_with(&NodeRef::new("ListRow").classes(cls), Some(&panel), scratch)
                .to_style();
            ListItem::new(Line::from(format!(" {r}"))).style(st)
        })
        .collect();
    frame.render_widget(List::new(items), chunks[0]);

    // Button row — focus + disabled drive the :focus / :disabled pseudo-states.
    let btn_rects = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(25); 4])
        .split(chunks[1]);
    let primary_cls: &[&str] = &["primary"];
    let outline_cls: &[&str] = &["outline"];
    let ghost_cls: &[&str] = &["ghost"];
    let buttons: &[(&str, &[&str])] = &[
        ("Deploy", primary_cls),
        ("Preview", outline_cls),
        ("Cancel", ghost_cls),
        ("Remove", primary_cls),
    ];
    for (i, (rect, (label, cls))) in btn_rects.iter().zip(buttons.iter()).enumerate() {
        let node = NodeRef::new("Button").classes(cls).state(State {
            focus: i == focus && !disabled,
            disabled,
            ..State::empty()
        });
        let computed = sheet.compute_with(&node, None, scratch);
        let para = Paragraph::new(Line::from(format!(" {label} "))).alignment(Alignment::Center);
        frame.render_widget(para.block(computed.to_block()), *rect);
    }

    // Footer — theme picker (line 1) + key hints (line 2).
    let footer = sheet.compute_with(&NodeRef::new("Footer"), None, scratch);
    let picker: String = themes
        .iter()
        .enumerate()
        .map(|(i, (name, _))| {
            let idx = i + 1;
            if i == cur {
                format!("[{idx}]{name}")
            } else {
                format!(" {idx} {name} ")
            }
        })
        .collect::<Vec<_>>()
        .join("  ");
    let hints = if disabled {
        " ↑↓ list · ←→ focus · [d] disabled ON · q quit"
    } else {
        " ↑↓ list · ←→ focus · d disable · q quit"
    };
    frame.render_widget(
        Paragraph::new(Text::from(vec![
            Line::from(format!(" {picker}")),
            Line::from(hints),
        ]))
        .style(footer.to_style()),
        outer[2],
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
