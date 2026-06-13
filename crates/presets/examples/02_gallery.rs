//! Preset gallery — browse **every** preset in one place, preview any of them.
//!
//! Gathers **all** presets — palette themes (`Default`, `Catppuccin`, `Nord`,
//! `Dracula`) *and* the other kinds (`Tailwind` utilities, `Widgets` type
//! defaults) — into a single selectable list, and previews the selected one
//! live.
//!
//! Two modes, toggled with `c`:
//! - **Compare** (default): the chrome (header / sidebar / footer / the labeled
//!   preview frame) is pinned to the stable `Default` theme, so only the
//!   **preview content** reflects the chosen preset — easy to compare.
//! - **Restyle**: the chrome follows the active palette too, so swapping a base
//!   theme restyles the *whole* frame (header, panel, list, buttons, …) with
//!   zero code changes — the headline theme-swap story, live.
//!
//! Each sidebar entry carries a color swatch sampled from that preset's brand
//! color. Only presets whose feature is enabled appear; run with
//! `--all-features` to see the full set.
//!
//! ```sh
//! cargo run -p ratatui-style-presets --example 02_gallery
//! cargo run -p ratatui-style-presets --example 02_gallery --all-features
//! ```
//!
//! Keys: `↑`/`↓` (or `Tab`) switch preset · `1`–`9` jump · `←`/`→` button focus
//! in the preview · `d` toggle `:disabled` · `c` compare/restyle · `q`/`Esc`
//! quits.

use std::io::{self, Stdout};
use std::time::Duration;

use crossterm::{
    event::{self, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color as RColor, Style},
    text::{Line, Span},
    widgets::{Block, List, ListItem, Paragraph},
    Terminal, backend::CrosstermBackend,
};
use ratatui_style::{Color, ComputeScratch, NodeRef, State, Stylesheet};

use ratatui_style_presets::{default_theme, merge, Preset};

type Term = Terminal<CrosstermBackend<Stdout>>;

/// How a preset wants to be previewed. Palette/component themes share one demo
/// (built from `Button`/`Panel`/`ListRow`/`Text`/`Badge`); Tailwind is its own
/// self-contained utility system, so it gets a utility-flavored demo.
#[derive(Clone, Copy)]
enum Preview {
    Theme,
    #[cfg(feature = "tailwind")]
    Tailwind,
}

/// One gallery entry: the preset, its pre-merged preview sheet, a brand color
/// for the sidebar swatch, and which preview to render.
struct Entry {
    name: &'static str,
    desc: &'static str,
    sheet: Stylesheet,
    accent: Option<Color>,
    preview: Preview,
}

impl Entry {
    /// Build an entry, computing the preview sheet + brand swatch up front so
    /// the draw loop never rebuilds them.
    fn theme(name: &'static str, desc: &'static str, presets: &[Preset]) -> Self {
        let sheet = merge(presets);
        // `Text.title` resolves to `--accent` in every theme → the brand color.
        let accent = sheet
            .compute(&NodeRef::new("Text").classes(&["title"]), None)
            .style
            .color
            .clone();
        Self {
            name,
            desc,
            sheet,
            accent,
            preview: Preview::Theme,
        }
    }
}

/// The presets available in this build, in gallery order. A cfg-gated array
/// keeps it declaration-style, so it compiles under any feature combination —
/// with none enabled, `Default` still appears.
#[allow(clippy::vec_init_then_push)] // cfg-gated pushes can't be one `vec![]` literal
fn available_presets() -> Vec<Entry> {
    let mut v = Vec::new();
    v.push(Entry::theme(
        "Default",
        "default theme + base components",
        &[Preset::Default],
    ));
    #[cfg(feature = "catppuccin")]
    v.push(Entry::theme(
        "Catppuccin",
        "Catppuccin Mocha palette",
        &[Preset::Default, Preset::Catppuccin],
    ));
    #[cfg(feature = "nord")]
    v.push(Entry::theme(
        "Nord",
        "Nord palette",
        &[Preset::Default, Preset::Nord],
    ));
    #[cfg(feature = "dracula")]
    v.push(Entry::theme(
        "Dracula",
        "Dracula palette",
        &[Preset::Default, Preset::Dracula],
    ));
    #[cfg(feature = "widgets")]
    v.push(Entry::theme(
        "Widgets",
        "ratatui widget-type defaults",
        &[Preset::Default, Preset::Widgets],
    ));
    #[cfg(feature = "tailwind")]
    {
        // Tailwind ships its own palette + `.btn` component, so it previews
        // standalone (no `Default` merge — that would mask its look).
        let sheet = merge(&[Preset::Tailwind]);
        let accent = sheet
            .compute(&NodeRef::new("Div").classes(&["btn", "primary"]), None)
            .style
            .background
            .clone();
        v.push(Entry {
            name: "Tailwind",
            desc: "atomic utility classes",
            sheet,
            accent,
            preview: Preview::Tailwind,
        });
    }
    v
}

/// Pull a concrete ratatui color out of a resolved `Color` (post-cascade every
/// color is `Literal` or `Reset`), for the sidebar swatch.
fn paint(c: Option<Color>) -> Option<RColor> {
    match c? {
        Color::Literal(rc) => Some(rc),
        _ => None,
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let entries = available_presets();
    let n = entries.len();
    let mut cur = 0usize; // active preset index
    let mut focus = 0usize; // preview button focus (Button:focus / .btn:focus)
    let mut disabled = false; // preview :disabled toggle
    let mut restyle = false; // chrome follows the active palette (theme-swap demo)

    let mut terminal = setup()?;
    let mut scratch = ComputeScratch::new();
    loop {
        terminal.draw(|f| {
            draw(f, &entries, cur, focus, disabled, restyle, &mut scratch);
        })?;

        if !event::poll(Duration::from_millis(120))? {
            continue;
        }
        let Event::Key(key) = event::read()? else {
            continue;
        };
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => break,

            // Switch the active preset.
            KeyCode::Up | KeyCode::Char('k') => cur = (cur + n - 1) % n,
            KeyCode::Down | KeyCode::Char('j') => cur = (cur + 1) % n,
            KeyCode::Tab => cur = (cur + 1) % n,

            // Preview button focus + :disabled pseudo-state.
            KeyCode::Left | KeyCode::Char('h') => focus = (focus + BTN_LEN - 1) % BTN_LEN,
            KeyCode::Right | KeyCode::Char('l') => focus = (focus + 1) % BTN_LEN,
            KeyCode::Char('d') => disabled = !disabled,

            // Compare (chrome pinned to Default) vs restyle (chrome follows the
            // active palette → the whole frame restyles on a theme swap).
            KeyCode::Char('c') => restyle = !restyle,

            // '1'..'9' → index 0..8 (only switches if in range).
            KeyCode::Char(c) if c.is_ascii_digit() => {
                let idx = (c as u8 - b'1') as usize;
                if idx < n {
                    cur = idx;
                }
            }
            _ => {}
        }
    }
    teardown(&mut terminal)?;
    Ok(())
}

const BTN_LEN: usize = 3;

fn draw(
    frame: &mut ratatui::Frame<'_>,
    entries: &[Entry],
    cur: usize,
    focus: usize,
    disabled: bool,
    restyle: bool,
    scratch: &mut ComputeScratch,
) {
    let area = frame.area();

    // Compare mode pins the chrome to the stable Default theme; restyle mode
    // drives the chrome from the active palette too, so a theme swap restyles
    // the whole frame. (Tailwind has no component vocabulary, so it can't
    // retheme the chrome — it stays Default.)
    let default = default_theme();
    let chrome: &Stylesheet = if restyle && matches!(entries[cur].preview, Preview::Theme) {
        &entries[cur].sheet
    } else {
        default
    };
    let root = chrome.compute_with(&NodeRef::new("Root"), None, scratch);
    frame.render_widget(Block::default().style(root.to_style()), area);

    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(1), Constraint::Length(2)])
        .split(area);

    // --- Header -----------------------------------------------------------
    let header = chrome.compute_with(&NodeRef::new("Header"), None, scratch);
    frame.render_widget(
        Paragraph::new(Line::from(format!(
            " ◆ ratatui-style-presets · Gallery  ({}/{})",
            cur + 1,
            entries.len()
        )))
        .style(header.to_style()),
        outer[0],
    );

    // --- Body: sidebar | preview -----------------------------------------
    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(26), Constraint::Min(1)])
        .split(outer[1]);

    draw_sidebar(frame, chrome, body[0], entries, cur, scratch);
    draw_preview(frame, chrome, body[1], &entries[cur], focus, disabled, scratch);

    // --- Footer -----------------------------------------------------------
    let footer = chrome.compute_with(&NodeRef::new("Footer"), None, scratch);
    let d_part = if disabled { "[d] disabled ON" } else { "d disable" };
    let c_part = if restyle { "[c] restyle ON" } else { "c restyle" };
    let hint = format!(" ↑↓/Tab preset · ←→ focus · {d_part} · {c_part} · q quit");
    frame.render_widget(
        Paragraph::new(Line::from(hint)).style(footer.to_style()),
        outer[2],
    );
}

/// The preset list, each row prefixed with a brand-color swatch. The sidebar is
/// rendered from the chrome sheet (Default in compare mode, the active palette
/// in restyle mode); only the swatch color always comes from the entry.
fn draw_sidebar(
    frame: &mut ratatui::Frame<'_>,
    chrome: &Stylesheet,
    area: Rect,
    entries: &[Entry],
    cur: usize,
    scratch: &mut ComputeScratch,
) {
    let panel = chrome.compute_with(&NodeRef::new("Panel"), None, scratch);
    let block = panel.to_block().title(" Presets ");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let active: &[&str] = &["active"];
    let idle: &[&str] = &[];
    let items: Vec<ListItem<'_>> = entries
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let (marker, cls) = if i == cur { ("▸ ", active) } else { ("  ", idle) };
            let swatch = match paint(e.accent.clone()) {
                Some(c) => Span::styled("● ", Style::default().fg(c)),
                None => Span::raw("● "),
            };
            let name_st = chrome
                .compute_with(&NodeRef::new("ListRow").classes(cls), None, scratch)
                .to_style();
            let name = Span::styled(format!("{marker}{}", e.name), name_st);
            ListItem::new(Line::from(vec![swatch, name]))
        })
        .collect();
    frame.render_widget(List::new(items), inner);
}

/// The preview frame is chrome (so it stays stable); only its *content* is
/// styled by the selected preset's own sheet.
fn draw_preview(
    frame: &mut ratatui::Frame<'_>,
    chrome: &Stylesheet,
    area: Rect,
    entry: &Entry,
    focus: usize,
    disabled: bool,
    scratch: &mut ComputeScratch,
) {
    let panel = chrome.compute_with(&NodeRef::new("Panel"), None, scratch);
    let block = panel
        .to_block()
        .title(format!(" Preview · {} — {} ", entry.name, entry.desc));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    match entry.preview {
        Preview::Theme => draw_theme_content(frame, &entry.sheet, inner, focus, disabled, scratch),
        #[cfg(feature = "tailwind")]
        Preview::Tailwind => {
            draw_tailwind_content(frame, &entry.sheet, inner, focus, disabled, scratch)
        }
    }
}

/// Demo for palette/component themes: text variants, a list, buttons, badges —
/// all resolved from `sheet`.
fn draw_theme_content(
    frame: &mut ratatui::Frame<'_>,
    sheet: &Stylesheet,
    area: Rect,
    focus: usize,
    disabled: bool,
    scratch: &mut ComputeScratch,
) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // text variants
            Constraint::Min(2),    // list
            Constraint::Length(3), // buttons
            Constraint::Length(3), // badges
        ])
        .split(area);

    // Text variants — each span picks up its `Text.<class>` color.
    let variants: &[(&str, &[&str])] = &[
        ("Text", &[]),
        ("Title", &["title"]),
        ("Success", &["success"]),
        ("Warning", &["warning"]),
        ("Danger", &["danger"]),
        ("Info", &["info"]),
        ("Muted", &["muted"]),
    ];
    let spans: Vec<Span<'_>> = variants
        .iter()
        .map(|(t, cls)| {
            let st = sheet
                .compute_with(&NodeRef::new("Text").classes(cls), None, scratch)
                .to_style();
            Span::styled(format!(" {t}"), st)
        })
        .collect();
    frame.render_widget(Paragraph::new(Line::from(spans)), chunks[0]);

    // List — first row carries `active` to show the selected state.
    let active: &[&str] = &["active"];
    let idle: &[&str] = &[];
    let rows = ["Inbox", "Drafts", "Sent", "Archive"];
    let items: Vec<ListItem<'_>> = rows
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let cls = if i == 0 { active } else { idle };
            let st = sheet
                .compute_with(&NodeRef::new("ListRow").classes(cls), None, scratch)
                .to_style();
            ListItem::new(Line::from(format!(" {r}"))).style(st)
        })
        .collect();
    frame.render_widget(List::new(items), chunks[1]);

    // Buttons — primary / outline / ghost, with live :focus + :disabled.
    let buttons: &[(&str, &[&str])] = &[
        ("Deploy", &["primary"]),
        ("Preview", &["outline"]),
        ("Cancel", &["ghost"]),
    ];
    render_button_row(frame, sheet, chunks[2], "Button", buttons, focus, disabled, scratch);

    // Badges — one per semantic state.
    let badges: &[(&str, &[&str])] = &[
        (" SUCCESS ", &["success"]),
        (" WARNING ", &["warning"]),
        (" DANGER ", &["danger"]),
        (" INFO ", &["info"]),
    ];
    let rects = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(25); 4])
        .split(chunks[3]);
    for (rect, (label, cls)) in rects.iter().zip(badges.iter()) {
        let st = sheet
            .compute_with(&NodeRef::new("Badge").classes(cls), None, scratch)
            .to_style();
        frame.render_widget(
            Paragraph::new(Line::from(*label))
                .alignment(Alignment::Center)
                .style(st),
            *rect,
        );
    }
}

/// Demo for Tailwind: atomic utility rows that compose into one style each,
/// then the `.btn` component + variant layer.
#[cfg(feature = "tailwind")]
fn draw_tailwind_content(
    frame: &mut ratatui::Frame<'_>,
    sheet: &Stylesheet,
    area: Rect,
    focus: usize,
    disabled: bool,
    scratch: &mut ComputeScratch,
) {
    // Each row composes several single-property utilities (bg + text color) into
    // one resolved style — the whole point of utility-first composition.
    let utils: &[(&str, &[&str])] = &[
        (" Blue-500 ", &["bg-blue-500", "text-white"]),
        (" Green-500 ", &["bg-green-500", "text-white"]),
        (" Amber-500 text ", &["text-amber-500"]),
        (" Slate-100 on Slate-900 ", &["bg-slate-900", "text-slate-100"]),
    ];
    let mut constraints: Vec<Constraint> = utils.iter().map(|_| Constraint::Length(1)).collect();
    constraints.push(Constraint::Length(3)); // button row
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area);

    for ((label, cls), rect) in utils.iter().zip(chunks.iter()) {
        let st = sheet
            .compute_with(&NodeRef::new("Div").classes(cls), None, scratch)
            .to_style();
        frame.render_widget(Paragraph::new(Line::from(*label)).style(st), *rect);
    }

    // `.btn` component + primary/outline/ghost variants, with :focus/:disabled.
    let buttons: &[(&str, &[&str])] = &[
        ("Deploy", &["btn", "primary"]),
        ("Preview", &["btn", "outline"]),
        ("Cancel", &["btn", "ghost"]),
    ];
    render_button_row(frame, sheet, chunks[utils.len()], "Div", buttons, focus, disabled, scratch);
}

/// A horizontal row of buttons driven by the `:focus` / `:disabled`
/// pseudo-states. `type_name` is `"Button"` for component themes, `"Div"` for
/// Tailwind's `.btn` layer.
#[allow(clippy::too_many_arguments)]
fn render_button_row(
    frame: &mut ratatui::Frame<'_>,
    sheet: &Stylesheet,
    area: Rect,
    type_name: &'static str,
    buttons: &[(&'static str, &'static [&'static str])],
    focus: usize,
    disabled: bool,
    scratch: &mut ComputeScratch,
) {
    let rects = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(33); 3])
        .split(area);
    for (i, (rect, (label, cls))) in rects.iter().zip(buttons.iter()).enumerate() {
        let node = NodeRef::new(type_name).classes(cls).state(State {
            focus: i == focus && !disabled,
            disabled,
            ..State::empty()
        });
        let st = sheet.compute_with(&node, None, scratch).to_style();
        frame.render_widget(
            Paragraph::new(Line::from(format!(" {label} ")))
                .alignment(Alignment::Center)
                .style(st),
            *rect,
        );
    }
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
