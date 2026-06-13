//! Real ratatui integration — render a computed style into an offscreen
//! `Buffer` and dump it as text. No TTY needed: fully deterministic.
//!
//! Shows the one-shot [`ComputedStyle::layout`] bridge: a single call replaces
//! the manual `apply_margin → to_block → block.inner → to_style` sequence.
//!
//! ```sh
//! cargo run -p ratatui-style --example 04_render
//! ```

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    text::Line,
    widgets::{Paragraph, Widget},
};

// One glob import pulls in the common engine/style/node types.
use ratatui_style::prelude::*;

fn main() {
    let mut sheet = Stylesheet::new();
    sheet
        .add(
            "Card",
            CssStyle::new()
                .border((BorderStyle::Rounded, "#00d4ff"))
                .padding(1u16)
                .background("#1e1e2e"),
            Origin::User,
        )
        .unwrap();

    let computed = sheet.compute(&OwnedNode::new("Card"), None);

    let area = Rect::new(0, 0, 44, 8);
    let mut buf = Buffer::empty(area);

    // One call instead of: apply_margin → to_block → block.inner → to_style.
    // `layout` returns (block, content_style, inner). We add a title to the
    // block here for the dump; the inner/style come straight from layout().
    let (block, content_style, inner) = computed.layout(area);
    block.title(" ratatui-style ").render(area, &mut buf);

    println!("Rendered `Card` block into a Buffer ({area:?}):\n");
    dump(&buf, area);
    println!("\nInner content area: {inner:?}");
    println!("Content style:      {content_style:?}");

    // Render a content widget into a buffer sized like the block's inner area,
    // applying the computed foreground style — the same shape `render_computed`
    // automates (which does this but driven by a `Frame` + closure). The buffer
    // starts at (0,0) so widget coordinates line up.
    let content_area = Rect::new(0, 0, inner.width, inner.height);
    let mut content_buf = Buffer::empty(content_area);
    Paragraph::new(Line::from(" rendered into the block's inner area "))
        .style(content_style)
        .render(content_area, &mut content_buf);
    println!("\nContent (sized like the inner area {inner:?}):\n");
    dump(&content_buf, content_area);
}

fn dump(buf: &Buffer, area: Rect) {
    for y in 0..area.height {
        let mut line = String::new();
        for x in 0..area.width {
            line.push_str(buf[(x, y)].symbol());
        }
        println!("│{line}│");
    }
}
