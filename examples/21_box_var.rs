//! P6 — `var()` for the box model: `padding`/`margin` (`BoxEdges`) and
//! `border-style`/`border` (`BorderStyle`).
//!
//! Before P6, `var()` resolved only for *colors*. P6 extends token resolution
//! to the box model, so a design system can drive spacing and frame style from
//! tokens — one variable change reshapes the whole UI.
//!
//! | CSS | Token | Resolves to |
//! |-----|-------|-------------|
//! | `padding: var(--gap)` | `--gap: 2` | `padding: 2` (all four edges) |
//! | `margin: var(--outer, 1)` | `--outer` unset | fallback `1` |
//! | `border-style: var(--frame)` | `--frame: rounded` | `rounded` |
//! | `border: var(--frame) var(--rim)` | both tokens | rounded + colored |
//!
//! This example renders two cards: one concrete (literal values) and one whose
//! spacing + frame come entirely from tokens — identical output, different
//! source. Then it flips `--frame` to prove one token edit changes the frame.
//!
//! ```sh
//! cargo run -p ratatui-style --example 21_box_var
//! ```

use ratatui::layout::Rect;
use ratatui::widgets::{Block, Widget};

use ratatui_style::{BorderStyle, CssStyle, OwnedNode, Stylesheet};

fn main() {
    let mut sheet = Stylesheet::parse(
        r##"
        :root {
            /* spacing tokens */
            --gap:   2;
            --outer: 1;

            /* frame tokens — a *style* and a *color*, both via var() */
            --frame: rounded;
            --rim:   #89b4fa;
        }

        /* concrete card: literals everywhere */
        Card.fixed {
            padding: 2;
            border: rounded #89b4fa;
        }

        /* token-driven card: every box-model value comes from a token */
        Card.token {
            padding: var(--gap);
            border-style: var(--frame);
            border-color: var(--rim);
        }

        /* fallback: undefined vars → degrade to each fallback. NOTE: a `var()`
         * fallback is parsed in the context of its property, so `border-style`
         * takes a style-only fallback and `border-color` a color-only fallback
         * (a combined `var(--x, plain #color)` would be rejected). */
        Card.fallback {
            padding: var(--nope, 1);
            border-style: var(--missing, plain);
            border-color: var(--other, #6c7086);
        }
        "##,
    )
    .expect("parse");

    let area = Rect::new(0, 0, 30, 6);

    println!("== fixed (literals) vs token (var()) — identical block ==\n");
    dump(&sheet, "Card.fixed", area);
    dump(&sheet, "Card.token", area);
    println!("  (same border + padding; the token card derives them from CSS vars)\n");

    println!("== undefined var → fallback ==\n");
    dump(&sheet, "Card.fallback", area);
    println!("  (padding→1, border-style→plain, border-color→#6c7086 — graceful)\n");

    println!("== flip one token (--frame → Double) → every token card's frame changes ==\n");
    // Mutating tokens bumps the generation; existing computed styles are stale
    // and re-resolve on next compute. (The live cache, if any, self-clears.)
    //
    // NOTE: a border-style token must be set with a TYPED `BorderStyle` value,
    // not a string — `insert("frame", "double")` would store "double" as a
    // color (it isn't one → Reset) and the border would silently disappear.
    // `:root { --frame: rounded }` worked because the text parser tries typed
    // parses; the runtime API goes through `From<&str>` → always a color.
    sheet.tokens_mut().insert("frame", BorderStyle::Double);
    dump(&sheet, "Card.token", area);
    println!("  (only --frame changed in the token table; the card now uses a double frame)");
}

/// Render a single card's block into a fresh buffer and dump it as text, so the
/// border characters make the resolved `BorderStyle` obvious.
fn dump(sheet: &Stylesheet, sel: &str, area: Rect) {
    // Resolve the node, then project to a ratatui Block — border + padding
    // (box model) are exactly what P6 var() resolution touches.
    let node = OwnedNode::new("Card").with_classes([sel.split('.').nth(1).unwrap_or("")]);
    let computed = sheet.compute(&node, None);
    let block: Block<'_> = computed.to_block().title(format!(" {sel} "));

    let mut buf = ratatui::buffer::Buffer::empty(area);
    block.render(area, &mut buf);
    for y in 0..area.height {
        let mut line = String::new();
        for x in 0..area.width {
            line.push_str(buf[(x, y)].symbol());
        }
        println!("  │{line}│");
    }
}

// Reference the typed-input API to show the non-var path as well (avoids an
// unused-import warning and documents the typed alternative).
#[allow(dead_code)]
fn _typed_border_demo() -> CssStyle {
    CssStyle::new().border((BorderStyle::Rounded, "#89b4fa")).padding(2u16)
}
