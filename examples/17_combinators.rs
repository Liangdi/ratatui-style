//! P3–P5 — Combinators: descendant `A B`, child `A > B`, adjacent `A + B`,
//! general sibling `A ~ B`, and nested chains.
//!
//! Combinators match against the **ancestor and sibling chain**, so they need a
//! real tree walk. The one-shot `compute(node, parent)` path has no ancestor
//! identity and does **not** match combinator selectors (documented in
//! design.md §11). Instead we drive [`CascadeContext::enter`]/[`leave`], which
//! maintains an ancestor-identity + prior-sibling stack when the stylesheet
//! `has_combinators()`.
//!
//! The tree here is built to make each combinator *visible*:
//!
//! ```text
//! App
//! └ Panel
//!   ├ Title        ← direct child of Panel
//!   ├ List
//!   │ ├ Title      ← nested Title: descendant of Panel, NOT a direct child
//!   │ ├ Row
//!   │ ├ Row        ← immediately follows a Row (adjacent `+`)
//!   │ └ Row        ← follows a Row (adjacent `+`)
//!   └ Caption
//! ```
//!
//! So `Panel Title` matches **both** Titles, but `Panel > Title` matches only
//! the first — proving descendant ≠ child. And `Row + Row` hits the 2nd and 3rd
//! rows but not the 1st.
//!
//! ```sh
//! cargo run -p ratatui-style --example 17_combinators
//! ```

use ratatui::style::Style;

use ratatui_style::{CascadeContext, OwnedNode, Stylesheet};

const SHEET: &str = r##"
:root {
    --accent: #89b4fa;
    --panel:  #313244;
    --text:   #cdd6f4;
    --sep:    #45475a;
    --muted:  #6c7086;
}

Root    { background: #1e1e2e; color: var(--text); }
Panel   { color: var(--text); border: rounded var(--panel); }
Footer  { color: var(--muted); }

/* descendant: ANY Title under Panel (direct child OR nested). */
Panel Title     { color: var(--accent); font-weight: bold; }

/* child: ONLY a Title that is a DIRECT child of Panel — underlines the panel
 * title but not the Title nested inside List. The two rules compose: the
 * direct-child Title is both colored (descendant) and underlined (child). */
Panel > Title   { text-decoration: underline; }

/* adjacent sibling: a Row that immediately follows another Row. Row #1 has no
 * preceding Row, so it does NOT match; Rows #2 and #3 do. */
Row + Row       { color: var(--sep); }

/* general sibling: a Row with ANY preceding sibling. Same set as `+` here, but
 * `~` would also catch a Row following a non-Row sibling. Adds italic. */
Row ~ Row       { font-style: italic; }
"##;

fn show(label: &str, s: Style) {
    println!(
        "  {label:42} fg={:<14?} mod={:?}",
        s.fg, s.add_modifier
    );
}

fn main() {
    let sheet = Stylesheet::parse(SHEET).expect("parse");
    // `has_combinators()` flips true the moment the parser sees ` `, `>`, `+`,
    // `~` — that is what makes `enter` maintain the ancestor/sibling stacks.
    assert!(sheet.has_combinators(), "stylesheet should contain combinators");

    let mut ctx = CascadeContext::new(&sheet);

    // --- walk the tree; enter/leave keep the identity stack in sync ---------
    let _root = ctx.enter(&OwnedNode::new("App"));

    let panel = ctx.enter(&OwnedNode::new("Panel"));
    show("Panel (base)", panel.to_style());

    let title_direct = ctx.enter(&OwnedNode::new("Title"));
    show("Panel > Title   (direct child: colored + underlined)", title_direct.to_style());
    ctx.leave();

    let _list = ctx.enter(&OwnedNode::new("List"));

    let title_nested = ctx.enter(&OwnedNode::new("Title"));
    show("Panel Title nested in List (colored, NOT underlined)", title_nested.to_style());
    ctx.leave(); // nested Title

    // Three sibling rows. enter/leave records each as a prior sibling for the
    // next, so `Row + Row` / `Row ~ Row` resolve correctly.
    let row1 = ctx.enter(&OwnedNode::new("Row"));
    show("Row #1 (no preceding Row: base color)", row1.to_style());
    ctx.leave();

    let row2 = ctx.enter(&OwnedNode::new("Row"));
    show("Row #2 (Row + Row: dimmed + italic)", row2.to_style());
    ctx.leave();

    let row3 = ctx.enter(&OwnedNode::new("Row"));
    show("Row #3 (Row + Row: dimmed + italic)", row3.to_style());
    ctx.leave();

    ctx.leave(); // List
    ctx.leave(); // Panel

    let footer = ctx.enter(&OwnedNode::new("Footer"));
    show("Footer", footer.to_style());

    println!();
    println!("Key takeaway: the two `Title` nodes diverge only because of the");
    println!("combinator — `Panel > Title` (child) excludes the nested one,");
    println!("while `Panel Title` (descendant) catches both. And `Row + Row`");
    println!("skips the first row because it has no preceding sibling.");
    println!();
    println!("This only works through `CascadeContext::enter`/`leave`; the");
    println!("one-shot `compute(node, parent)` path carries no ancestor chain.");
}
