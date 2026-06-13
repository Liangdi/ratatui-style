//! The cascade engine — turns a [`Stylesheet`] + a [`StyledNode`] into a fully
//! resolved [`ComputedStyle`].
//!
//! Pipeline per element:
//! 1. Collect matching rules.
//! 2. Sort ascending by `(origin, specificity, source_order)`.
//! 3. Fold declarations via [`CssStyle::overlay`] (later = higher priority).
//! 4. Fold explicit `inherit` keywords and auto-inherited properties from the
//!    parent [`ComputedStyle`].
//! 5. Resolve `var()` references against the token table.

use ratatui::{
    layout::{Alignment, Constraint, Rect},
    style::Style as RStyle,
    widgets::Block,
};

use crate::box_model::Length;
use crate::color::Color;
use crate::node::{Classes, StyledNode};
use crate::style::CssStyle;
use crate::stylesheet::Stylesheet;
use crate::token::{self, ThemeTokens};

/// A fully-resolved style: all `var()`s turned into literals, inheritable
/// properties filled from the parent. Project onto ratatui via the delegate
/// methods.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ComputedStyle {
    pub style: CssStyle,
}

impl ComputedStyle {
    pub fn new(style: CssStyle) -> Self {
        Self { style }
    }
    pub fn to_style(&self) -> RStyle {
        self.style.to_style()
    }
    pub fn to_block(&self) -> Block<'_> {
        self.style.to_block()
    }
    pub fn apply_margin(&self, area: Rect) -> Rect {
        self.style.apply_margin(area)
    }
    pub fn constraints(&self) -> Option<(Constraint, Constraint)> {
        self.style.constraints()
    }
    pub fn alignment(&self) -> Option<Alignment> {
        self.style.alignment()
    }

    /// Layer a single inline declaration on top of the computed style in place.
    ///
    /// Because `Origin::Inline` is applied last in the cascade, an inline
    /// declaration *wins* over every rule regardless of specificity. This
    /// method reproduces that post-compute: it overlays `inline` via
    /// [`CssStyle::overlay`], so any field `Some` in `inline` replaces the
    /// matching computed field.
    ///
    /// Note: colors passed here should already be literals — `compute` has
    /// already resolved `var()` against the token table, and this method does
    /// not re-resolve.
    pub fn apply_inline(&mut self, inline: &CssStyle) {
        self.style.overlay(inline);
    }

    /// Consuming builder form of [`apply_inline`](Self::apply_inline): overlay
    /// an inline declaration and return `self`.
    pub fn with_inline(mut self, inline: &CssStyle) -> Self {
        self.apply_inline(inline);
        self
    }

    /// One-shot box-model projection: resolve the full `margin → block →
    /// block.inner → content style` sequence in a single call.
    ///
    /// Returns `(block, content_style, inner_area)` where:
    /// - `block` is [`ComputedStyle::to_block`] (borders/padding/background, no
    ///   margin — margin is applied to the area first);
    /// - `content_style` is [`ComputedStyle::to_style`] (the foreground
    ///   decoration to apply to the inner widget);
    /// - `inner_area` is the area left for content after margin shrink *and*
    ///   the block's padding/borders, i.e. it equals
    ///   `to_block().inner(apply_margin(area))`.
    ///
    /// This matches the hand-written sequence downstream widgets previously had
    /// to thread themselves:
    ///
    /// ```text
    /// let shrunk = computed.apply_margin(area);
    /// let block  = computed.to_block();
    /// let inner  = block.inner(shrunk);
    /// let style  = computed.to_style();
    /// ```
    ///
    /// Box-model order is `margin (outer) → border → padding → content`, so the
    /// margin shrink happens *outside* the block and `block.inner` only removes
    /// padding/borders — never the margin.
    pub fn layout(&self, area: Rect) -> (Block<'_>, RStyle, Rect) {
        let shrunk = self.apply_margin(area);
        let block = self.to_block();
        let inner = block.inner(shrunk);
        let style = self.to_style();
        (block, style, inner)
    }
}

/// Render a computed style's full box model in one shot.
///
/// Resolves `(block, content_style, inner)` via [`ComputedStyle::layout`],
/// renders the `block` into the margin-shrunk area, then renders the widget
/// returned by `make(inner, content_style)` into the block's inner area. The
/// closure receives the inner `Rect` and the foreground `Style` and is expected
/// to apply the style to the widget it builds (this mirrors how most ratatui
/// widgets carry a `.style(...)`).
///
/// Use this to collapse the `margin → block → content` boilerplate into a
/// single call.
///
/// ```rust,ignore
/// use ratatui::widgets::Paragraph;
/// use ratatui_style::{ComputedStyle, render_computed};
///
/// render_computed(frame, &computed, area, |inner, style| {
///     Paragraph::new("hello").style(style)
/// });
/// ```
pub fn render_computed<W, F>(
    frame: &mut ratatui::Frame<'_>,
    computed: &ComputedStyle,
    area: Rect,
    make: F,
) where
    F: FnOnce(Rect, RStyle) -> W,
    W: ratatui::widgets::Widget,
{
    let shrunk = computed.apply_margin(area);
    let (block, style, inner) = computed.layout(area);
    frame.render_widget(block, shrunk);
    frame.render_widget(make(inner, style), inner);
}

/// A cascade tree-walker: holds a [`Stylesheet`] reference, a reusable
/// [`ComputeScratch`], and a parent [`ComputedStyle`] stack.
///
/// `enter(node)` computes the node's style using the current stack top (if any)
/// as its parent, pushes the result, and returns an owned copy; `leave()` pops
/// it. This lets a downstream component-tree traversal inherit styles
/// automatically without the caller manually threading `Some(&parent)` into
/// every child's `compute` call.
///
/// # Why `enter` returns an owned value
///
/// `enter` returns an *owned* [`ComputedStyle`] rather than `&ComputedStyle`.
/// Returning a borrow would lock `&mut self` for the returned value's lifetime,
/// making it impossible to nest a second `enter` for a child while holding the
/// parent's style. The owned return avoids that entirely — the caller can hold
/// the parent's computed style freely and still call `enter` for children.
///
/// # Pushed clone is stack-only memcpy
///
/// After `compute`, a [`ComputedStyle`] holds only `Literal`/`Reset`/`Copy`
/// fields — every `var()` has been resolved against the token table, so no
/// [`Color::Var`] (the only [`Color`] variant carrying a heap `String` / `Box`)
/// survives. Every other field (`BoxEdges`, `BorderSpec`, `Weight`, `Length`,
/// …) is a fixed-size, stack-resident enum/struct. The `computed.clone()` that
/// backs the internal stack is therefore a plain stack memcpy with no heap
/// allocation, and is cheap to ignore.
///
/// # Example — walking a three-level tree
///
/// ```rust,ignore
/// use ratatui_style::{CascadeContext, OwnedNode, Stylesheet};
///
/// let sheet: Stylesheet = /* … */;
/// let mut ctx = CascadeContext::new(&sheet);
///
/// // Root
/// let root = ctx.enter(&OwnedNode::new("Root"));
/// // …render root…
///
/// // Panel (child of Root)
/// let panel = ctx.enter(&OwnedNode::new("Panel"));
/// // …render panel…
///
/// // Text (child of Panel) — inherits Panel's color automatically
/// let text = ctx.enter(&OwnedNode::new("Text"));
/// // …render text…
/// ctx.leave(); // back to Panel context
///
/// ctx.leave(); // back to Root context
/// ctx.leave(); // done
/// ```
pub struct CascadeContext<'s> {
    sheet: &'s Stylesheet,
    scratch: ComputeScratch,
    stack: Vec<ComputedStyle>,
}

impl<'s> CascadeContext<'s> {
    /// Build a walker over `sheet` with an empty parent stack and a fresh
    /// reusable scratch buffer.
    pub fn new(sheet: &'s Stylesheet) -> Self {
        Self {
            sheet,
            scratch: ComputeScratch::new(),
            stack: Vec::new(),
        }
    }

    /// Compute `node`'s style using the current stack top as its parent, push
    /// the result onto the stack, and return an owned copy.
    ///
    /// The returned value is what the caller uses directly for rendering; the
    /// clone pushed onto the stack serves as the parent for subsequent `enter`
    /// calls within this subtree.
    pub fn enter(&mut self, node: &dyn StyledNode) -> ComputedStyle {
        let parent = self.stack.last();
        let computed = self.sheet.compute_with(node, parent, &mut self.scratch);
        self.stack.push(computed.clone());
        computed
    }

    /// Pop the most recently `enter`ed node (leaving its subtree).
    pub fn leave(&mut self) -> Option<ComputedStyle> {
        self.stack.pop()
    }

    /// The current stack top — the style a subsequent `enter` will inherit
    /// from. `None` at the root (depth 0).
    pub fn current(&self) -> Option<&ComputedStyle> {
        self.stack.last()
    }

    /// Number of nodes currently on the stack (the tree depth).
    pub fn depth(&self) -> usize {
        self.stack.len()
    }

    /// The backing stylesheet.
    pub fn sheet(&self) -> &Stylesheet {
        self.sheet
    }
}

/// A reusable cascade scratch buffer.
///
/// Held across many [`Stylesheet::compute_with`] calls, it retains its
/// capacity so the per-`compute` matching buffer stops allocating once it has
/// warmed up. It stores **rule indices** (`Vec<usize>`), not references, so it
/// carries no lifetime parameter and can be owned long-term by the caller
/// without borrowing the stylesheet.
///
/// ```rust,ignore
/// let mut scratch = ComputeScratch::new();
/// // reuse across the whole draw loop:
/// for node in &nodes {
///     let style = sheet.compute_with(node, parent, &mut scratch);
/// }
/// ```
pub struct ComputeScratch {
    matching: Vec<usize>,
}

impl ComputeScratch {
    pub fn new() -> Self {
        Self { matching: Vec::new() }
    }
}

impl Default for ComputeScratch {
    fn default() -> Self {
        Self::new()
    }
}

impl Stylesheet {
    /// Compute the resolved style for `node`, optionally inheriting from
    /// `parent`.
    ///
    /// Thin wrapper over [`compute_with`](Self::compute_with) with a fresh
    /// [`ComputeScratch`]. Behavior is identical to `compute_with` — this
    /// exists for one-shot callers and backwards compatibility.
    pub fn compute(&self, node: &dyn StyledNode, parent: Option<&ComputedStyle>) -> ComputedStyle {
        let mut scratch = ComputeScratch::new();
        self.compute_with(node, parent, &mut scratch)
    }

    /// Compute using a caller-provided [`ComputeScratch`] so the matching
    /// buffer is reused across calls (zero allocation once warmed up).
    ///
    /// This is the allocation-conscious entry point for the draw loop. Three
    /// per-frame allocations are eliminated relative to `compute`:
    ///
    /// 1. **Classes** are fetched from the node exactly once (hoisted out of
    ///    the rule loop) and matched via [`Selector::matches_values`], so the
    ///    R-rules × 1-node cost is one `Classes` materialization, not R.
    /// 2. **Matching buffer** lives in `scratch` and is `clear()`-ed, not
    ///    re-allocated.
    /// 3. When the node is a [`NodeRef`](crate::node::NodeRef), the classes
    ///    materialization itself is zero-allocation.
    pub fn compute_with(
        &self,
        node: &dyn StyledNode,
        parent: Option<&ComputedStyle>,
        scratch: &mut ComputeScratch,
    ) -> ComputedStyle {
        // Hoist node fields out of the per-rule loop: this is the single most
        // important change — `classes()` is called once per node, not once per
        // rule. For OwnedNode that's one Vec allocation; for NodeRef, zero.
        let type_name = node.type_name();
        let id = node.id();
        let classes: Classes<'_> = node.classes();
        let state = node.state();

        let rules = self.rules();

        // 1. Collect matching rule *indices* into the reused scratch buffer.
        scratch.matching.clear();
        for (i, r) in rules.iter().enumerate() {
            if r.selector.matches_values(type_name, id, &classes, state) {
                scratch.matching.push(i);
            }
        }

        // 2. Sort ascending by (origin, specificity, source_order).
        scratch.matching.sort_unstable_by_key(|&i| {
            let r = &rules[i];
            (r.origin, r.selector.specificity(), r.order)
        });

        // 3. Fold declarations (later = higher priority).
        let mut own = CssStyle::new();
        for &i in &scratch.matching {
            own.overlay(&rules[i].style);
        }

        // 4. Inheritance.
        if let Some(parent) = parent {
            resolve_explicit_inherit(&mut own, &parent.style);
            own.inherit_from(&parent.style);
        }

        // 5. var() resolution against the stylesheet's token table.
        resolve_vars_in_place(&mut own, self.tokens());

        ComputedStyle::new(own)
    }
}

/// Replace explicit `inherit` keyword colors with the parent's value, for all
/// three color fields (CSS `inherit` forces inheritance even for
/// non-inheritable properties like `background`).
fn resolve_explicit_inherit(own: &mut CssStyle, parent: &CssStyle) {
    if matches!(own.color, Some(Color::Inherit)) {
        own.color = parent.color.clone();
    }
    if matches!(own.background, Some(Color::Inherit)) {
        own.background = parent.background.clone();
    }
    if matches!(own.underline_color, Some(Color::Inherit)) {
        own.underline_color = parent.underline_color.clone();
    }
}

/// Resolve every `var()` / leftover `inherit` in the color and length fields
/// to a literal — including the `Color` nested inside a `border` spec. Color
/// fields degrade to `Reset` on failure; length fields degrade to `Auto` —
/// both lenient, neither panics.
fn resolve_vars_in_place(style: &mut CssStyle, tokens: &ThemeTokens) {
    resolve_color_field(&mut style.color, tokens);
    resolve_color_field(&mut style.background, tokens);
    resolve_color_field(&mut style.underline_color, tokens);
    // The border color is a `Color` nested inside `Option<BorderSpec>`, so it is
    // not covered by the top-level field passes above. Resolve it here too, or a
    // `border: rounded var(--dim)` survives the cascade as a `Var` and `paint`
    // drops it — the border then draws with no explicit color.
    if let Some(border) = style.border.as_mut() {
        resolve_color_field(&mut border.color, tokens);
    }
    resolve_length_field(&mut style.width, tokens);
    resolve_length_field(&mut style.height, tokens);
}

fn resolve_color_field(field: &mut Option<Color>, tokens: &ThemeTokens) {
    if let Some(inner) = field {
        match inner {
            Color::Literal(_) | Color::Reset => {} // already concrete
            Color::Var { .. } | Color::Inherit => {
                *field = Some(Color::Literal(token::resolve(inner, tokens)));
            }
        }
    }
}

/// Mirrors [`resolve_color_field`] for the length path (width/height). A
/// `Length::Var` is resolved against the token table; anything else is left
/// untouched. Failures (undefined name, type mismatch, cycle) degrade to
/// [`Length::Auto`] — consistent with the lenient color path degrading to
/// `Reset`.
fn resolve_length_field(field: &mut Option<Length>, tokens: &ThemeTokens) {
    if let Some(inner) = field {
        if let Length::Var { .. } = inner {
            *field = Some(token::resolve_length(inner, tokens));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::{NodeRef, OwnedNode, State};
    use crate::stylesheet::Origin;
    use ratatui::style::Color as RC;

    fn sheet() -> Stylesheet {
        let mut s = Stylesheet::with_tokens(
            crate::token::ThemeTokens::new().set("accent", Color::literal(RC::Cyan)),
        );
        // type rule (low specificity)
        s.add("Button", CssStyle::new().color(RC::Gray), Origin::User).unwrap();
        // class rule (higher specificity)
        s.add("Button.primary", CssStyle::new().background(RC::Blue), Origin::User).unwrap();
        // id rule (highest specificity)
        s.add("#save", CssStyle::new().color(RC::Yellow), Origin::User).unwrap();
        // focus pseudo-state
        s.add("Button:focus", CssStyle::new().background(RC::Green), Origin::User).unwrap();
        // var() consumer
        s.add(".accented", CssStyle::new().color(Color::var("accent")), Origin::User).unwrap();
        // inline (origin) overrides specificity
        s
    }

    #[test]
    fn specificity_wins() {
        let s = sheet();
        let n = OwnedNode::new("Button").with_id("save").with_classes(["primary"]);
        let c = s.compute(&n, None);
        // #save (id) wins over Button (type) for color.
        assert_eq!(c.style.color, Some(Color::literal(RC::Yellow)));
        // .primary (class) wins over Button (type) for background.
        assert_eq!(c.style.background, Some(Color::literal(RC::Blue)));
    }

    #[test]
    fn pseudo_state_matches() {
        let s = sheet();
        let n = OwnedNode::new("Button").with_state(State::focus());
        let c = s.compute(&n, None);
        assert_eq!(c.style.background, Some(Color::literal(RC::Green)));
    }

    #[test]
    fn var_resolves_from_tokens() {
        let s = sheet();
        let n = OwnedNode::new("Text").with_classes(["accented"]);
        let c = s.compute(&n, None);
        assert_eq!(c.style.color, Some(Color::literal(RC::Cyan)));
    }

    #[test]
    fn border_color_var_resolves_from_tokens() {
        // Regression for the 0.1.1 limitation: a `var()` in the border
        // shorthand color must be resolved against the token table, not left as
        // a `Color::Var` (which `paint` would silently drop).
        //
        // This is the exact downstream DetailPanel case: `#003237` was used as a
        // literal in place of `var(--border-dim)` because the cascade did not
        // resolve border colors. With the fix, the token resolves.
        let sheet = Stylesheet::parse(
            ":root{--border-dim:#003237} .panel { border: rounded var(--border-dim); }",
        )
        .unwrap();
        let n = OwnedNode::new("Div").with_classes(["panel"]);
        let c = sheet.compute(&n, None);
        let border = c.style.border.expect("border present");
        assert_eq!(border.style, crate::box_model::BorderStyle::Rounded);
        assert_eq!(border.color, Some(Color::literal(RC::Rgb(0x00, 0x32, 0x37))));
    }

    #[test]
    fn border_color_var_via_subdeclaration_resolves() {
        // The `border-color: var(--x)` sub-declaration path must resolve too —
        // it lands in the same nested `BorderSpec.color` field.
        let sheet = Stylesheet::parse(
            ":root{--rim:#ff0000} .b { border-style: single; border-color: var(--rim); }",
        )
        .unwrap();
        let n = OwnedNode::new("Div").with_classes(["b"]);
        let c = sheet.compute(&n, None);
        let border = c.style.border.expect("border present");
        assert_eq!(border.color, Some(Color::literal(RC::Rgb(0xff, 0x00, 0x00))));
    }

    #[test]
    fn border_color_var_fallback_resolves() {
        // An undefined border-color var with a fallback degrades to the
        // fallback, mirroring the other color fields.
        let sheet = Stylesheet::parse(".b { border: rounded var(--nope, #00ff00); }").unwrap();
        let n = OwnedNode::new("Div").with_classes(["b"]);
        let c = sheet.compute(&n, None);
        let border = c.style.border.expect("border present");
        assert_eq!(border.color, Some(Color::literal(RC::Rgb(0x00, 0xff, 0x00))));
    }

    #[test]
    fn inheritance_from_parent() {
        let s = sheet();
        let parent_node = OwnedNode::new("Button").with_classes(["primary"]);
        let parent = s.compute(&parent_node, None);
        // Child Text has no color of its own; inherits parent's.
        let child = OwnedNode::new("Text");
        let computed = s.compute(&child, Some(&parent));
        assert_eq!(computed.style.color, Some(Color::literal(RC::Gray)));
    }

    #[test]
    fn origin_overrides_specificity() {
        let mut s = Stylesheet::new();
        s.add("Button", CssStyle::new().color(RC::Red), Origin::User).unwrap();
        // Inline origin wins despite identical selector.
        s.add("Button", CssStyle::new().color(RC::Blue), Origin::Inline).unwrap();
        let n = OwnedNode::new("Button");
        let c = s.compute(&n, None);
        assert_eq!(c.style.color, Some(Color::literal(RC::Blue)));
    }

    #[test]
    fn with_inline_overrides_specificity() {
        // An id selector has the highest specificity in the sheet, but an inline
        // declaration layered on top post-compute must still win — that is the
        // whole point of inline origin being applied last.
        let mut s = Stylesheet::new();
        s.add("#save", CssStyle::new().color(RC::Yellow), Origin::User).unwrap();
        let n = OwnedNode::new("Button").with_id("save");
        let c = s.compute(&n, None).with_inline(&CssStyle::new().color("red"));
        // The id rule set Yellow; inline red wins.
        assert_eq!(c.style.color, Some(Color::literal(RC::Red)));
    }

    #[test]
    fn apply_inline_in_place_overrides() {
        // Same semantics, mutating form.
        let mut s = Stylesheet::new();
        s.add("Button.primary", CssStyle::new().color(RC::Blue), Origin::User)
            .unwrap();
        let n = OwnedNode::new("Button").with_classes(["primary"]);
        let mut c = s.compute(&n, None);
        c.apply_inline(&CssStyle::new().color("red"));
        assert_eq!(c.style.color, Some(Color::literal(RC::Red)));
    }

    #[test]
    fn layout_inner_matches_handwritten_sequence() {
        // A fully-featured style: margin (outer), rounded border, padding.
        let computed = ComputedStyle::new(
            CssStyle::new().margin("2").padding("1").border("rounded #00d4ff"),
        );
        let area = Rect::new(0, 0, 44, 8);

        let (_block, _style, inner_from_layout) = computed.layout(area);

        // The hand-written sequence layout() must be equivalent to.
        let shrunk = computed.apply_margin(area);
        let block = computed.to_block();
        let inner_from_hand = block.inner(shrunk);

        assert_eq!(inner_from_layout, inner_from_hand);
        // Sanity: with margin 2 (each side) + 1 border + 1 padding, the inner
        // width drops by 2*(2+1+1) = 8, and height by 8 too.
        assert_eq!(inner_from_layout, Rect::new(4, 4, 36, 0));
    }

    #[test]
    fn layout_inner_equals_area_with_no_box_model() {
        let computed = ComputedStyle::new(CssStyle::new());
        let area = Rect::new(0, 0, 30, 10);
        let (_block, _style, inner) = computed.layout(area);
        assert_eq!(inner, area);
    }

    #[test]
    fn layout_content_style_matches_to_style() {
        let computed =
            ComputedStyle::new(CssStyle::new().color(RC::Cyan).bold().padding("1"));
        let area = Rect::new(0, 0, 20, 5);
        let (_block, style, _inner) = computed.layout(area);
        assert_eq!(style, computed.to_style());
    }

    // ---------------------------------------------------------------------
    // NodeRef / compute_with parity & reuse
    // ---------------------------------------------------------------------

    fn parity_sheet() -> Stylesheet {
        let mut s = Stylesheet::new();
        s.add("Button", CssStyle::new().color(RC::Gray), Origin::User).unwrap();
        s.add("Button.primary", CssStyle::new().background(RC::Blue), Origin::User).unwrap();
        s.add("#save", CssStyle::new().color(RC::Yellow), Origin::User).unwrap();
        s.add("Button:focus", CssStyle::new().background(RC::Green), Origin::User).unwrap();
        s
    }

    #[test]
    fn noderef_behavioral_parity() {
        // Same data via OwnedNode vs NodeRef → identical ComputedStyle across
        // all four selector dimensions (type, class, id, state).
        let sheet = parity_sheet();

        let owned = OwnedNode::new("Button")
            .with_id("save")
            .with_classes(["primary"])
            .with_state(State::focus());
        let borrowed = NodeRef::new("Button")
            .id("save")
            .classes(&["primary"])
            .state(State::focus());

        let c_owned = sheet.compute(&owned, None);
        let c_borrowed = sheet.compute(&borrowed, None);
        assert_eq!(c_owned, c_borrowed);
    }

    #[test]
    fn noderef_zero_string_construction() {
        // Pure &'static str path — no String/Vec heap allocation is possible.
        let sheet = parity_sheet();
        let node = NodeRef::new("Button").classes(&["primary"]).state(State::focus());
        let c = sheet.compute(&node, None);
        // type + class match for background; color from the type rule.
        // (Button.primary sets Blue, Button:focus sets Green; same specificity
        // (0,1,1) so source order wins → Green, added last.)
        assert_eq!(c.style.background, Some(Color::literal(RC::Green)));
        assert_eq!(c.style.color, Some(Color::literal(RC::Gray)));
    }

    #[test]
    fn compute_with_matches_compute() {
        let sheet = parity_sheet();
        let mut scratch = ComputeScratch::new();

        let cases: [(&str, OwnedNode); 5] = [
            ("plain", OwnedNode::new("Button")),
            ("primary", OwnedNode::new("Button").with_classes(["primary"])),
            ("id", OwnedNode::new("Button").with_id("save")),
            ("focus", OwnedNode::new("Button").with_state(State::focus())),
            ("combo", OwnedNode::new("Button").with_id("save").with_classes(["primary"]).with_state(State::focus())),
        ];

        for (name, node) in cases {
            let via_compute = sheet.compute(&node, None);
            let via_compute_with = sheet.compute_with(&node, None, &mut scratch);
            assert_eq!(via_compute, via_compute_with, "mismatch for case `{name}`");
        }
    }

    #[test]
    fn scratch_reuse_no_panic() {
        // Reuse the same scratch across many computes of varying sizes — the
        // clear()+push() path must stay correct and never leak prior results.
        let sheet = parity_sheet();
        let mut scratch = ComputeScratch::new();

        // A node that matches many rules, then one that matches none, then a
        // big one again — exercise the clear/reuse.
        let big = NodeRef::new("Button").id("save").classes(&["primary"]).state(State::focus());
        let none = NodeRef::new("NoSuchType");

        let c1 = sheet.compute_with(&big, None, &mut scratch);
        let c_none = sheet.compute_with(&none, None, &mut scratch);
        let c2 = sheet.compute_with(&big, None, &mut scratch);

        // The "none" node only matches the universal-less base → no rules.
        assert_eq!(c_none.style.color, None);
        // Re-running the big node after the empty one yields the same result.
        assert_eq!(c1, c2);
        assert_eq!(c1.style.color, Some(Color::literal(RC::Yellow)));
    }

    // ---------------------------------------------------------------------
    // CascadeContext
    // ---------------------------------------------------------------------

    fn context_sheet() -> Stylesheet {
        // Panel sets a color; Text sets none → Text inherits Panel's color.
        let mut s = Stylesheet::new();
        s.add("Panel", CssStyle::new().color("#cdd6f4"), Origin::User).unwrap();
        s
    }

    #[test]
    fn context_inherits_without_manual_threading() {
        // enter(Panel) then enter(Text) — Text should inherit Panel's color
        // without the test ever writing `Some(&parent)`.
        let sheet = context_sheet();
        let mut ctx = CascadeContext::new(&sheet);

        let _panel = ctx.enter(&OwnedNode::new("Panel"));
        let text = ctx.enter(&OwnedNode::new("Text"));

        assert_eq!(text.style.color, Some(Color::literal(RC::Rgb(0xcd, 0xd6, 0xf4))));
    }

    #[test]
    fn context_parity_with_manual_compute() {
        // Same small tree (Root→Panel→Text) computed two ways: CascadeContext
        // vs the hand-written compute(node, Some(&parent)) chain. Every node's
        // ComputedStyle must be identical.
        let mut sheet = Stylesheet::new();
        sheet.add("Root", CssStyle::new().color(RC::Red), Origin::User).unwrap();
        sheet.add("Panel", CssStyle::new().padding("1"), Origin::User).unwrap();
        // Text sets nothing → inherits everything inheritable from Panel/Root.
        sheet.add("Text", CssStyle::new(), Origin::User).unwrap();

        // --- CascadeContext path ---
        let mut ctx = CascadeContext::new(&sheet);
        let ctx_root = ctx.enter(&OwnedNode::new("Root"));
        let ctx_panel = ctx.enter(&OwnedNode::new("Panel"));
        let ctx_text = ctx.enter(&OwnedNode::new("Text"));

        // --- Manual threading path ---
        let man_root = sheet.compute(&OwnedNode::new("Root"), None);
        let man_panel = sheet.compute(&OwnedNode::new("Panel"), Some(&man_root));
        let man_text = sheet.compute(&OwnedNode::new("Text"), Some(&man_panel));

        assert_eq!(ctx_root, man_root);
        assert_eq!(ctx_panel, man_panel);
        assert_eq!(ctx_text, man_text);
    }

    #[test]
    fn context_leave_restores_parent() {
        // enter A (color), enter B (different color), leave, enter C (no color)
        // → C must inherit from A, not B.
        let mut sheet = Stylesheet::new();
        sheet.add("A", CssStyle::new().color(RC::Red), Origin::User).unwrap();
        sheet.add("B", CssStyle::new().color(RC::Blue), Origin::User).unwrap();
        // C has no color rule.
        sheet.add("C", CssStyle::new(), Origin::User).unwrap();

        let mut ctx = CascadeContext::new(&sheet);
        let _a = ctx.enter(&OwnedNode::new("A"));
        let _b = ctx.enter(&OwnedNode::new("B"));
        ctx.leave(); // drop B
        let c = ctx.enter(&OwnedNode::new("C"));

        // C inherits A's color (Red), not B's (Blue).
        assert_eq!(c.style.color, Some(Color::literal(RC::Red)));
    }

    #[test]
    fn context_depth() {
        let sheet = context_sheet();
        let mut ctx = CascadeContext::new(&sheet);

        assert_eq!(ctx.depth(), 0);
        ctx.enter(&OwnedNode::new("Panel"));
        assert_eq!(ctx.depth(), 1);
        ctx.enter(&OwnedNode::new("Text"));
        assert_eq!(ctx.depth(), 2);
        ctx.leave();
        assert_eq!(ctx.depth(), 1);
        ctx.leave();
        assert_eq!(ctx.depth(), 0);
        assert!(ctx.leave().is_none());
    }

    #[test]
    fn context_scratch_reused() {
        // Many consecutive enters of mixed nodes — the internal scratch buffer
        // is cleared and reused each time; correctness must not regress.
        let mut sheet = Stylesheet::new();
        sheet.add("A", CssStyle::new().color(RC::Red), Origin::User).unwrap();
        sheet.add("A.child", CssStyle::new().bold(), Origin::User).unwrap();
        sheet.add("NoMatch", CssStyle::new().color(RC::Green), Origin::User).unwrap();

        let mut ctx = CascadeContext::new(&sheet);

        // child matches two rules (A + A.child); NoMatch matches none.
        let child = ctx.enter(&OwnedNode::new("A").with_classes(["child"]));
        assert_eq!(child.style.color, Some(Color::literal(RC::Red)));

        let none = ctx.enter(&OwnedNode::new("TotallyUnknown"));
        // No matching rule, no inheritable parent value set on color here
        // (parent A had Red, and color is inheritable) → inherits Red.
        assert_eq!(none.style.color, Some(Color::literal(RC::Red)));

        // Re-run child-like after a no-match — must not leak prior matching set.
        ctx.leave();
        let child2 = ctx.enter(&OwnedNode::new("A").with_classes(["child"]));
        assert_eq!(child2.style.color, Some(Color::literal(RC::Red)));
    }

    // ---------------------------------------------------------------------
    // Length var() resolution (width/height)
    // ---------------------------------------------------------------------

    #[test]
    fn width_var_resolves() {
        let sheet = Stylesheet::parse(":root{--w:50%} .col { width: var(--w);}").unwrap();
        let node = OwnedNode::new("Div").with_classes(["col"]);
        let c = sheet.compute(&node, None);
        assert_eq!(c.style.width, Some(crate::box_model::Length::Percent(50)));
    }

    #[test]
    fn width_var_chain() {
        let sheet = Stylesheet::parse(
            ":root{--w: var(--w2); --w2: 10;} .x { width: var(--w); }",
        )
        .unwrap();
        let node = OwnedNode::new("Div").with_classes(["x"]);
        let c = sheet.compute(&node, None);
        assert_eq!(c.style.width, Some(crate::box_model::Length::Cells(10)));
    }

    #[test]
    fn width_var_undefined_degrades_to_auto() {
        // Lenient parse: an undefined var degrades to Auto, no error.
        let sheet = Stylesheet::parse(".x { width: var(--nope); }").unwrap();
        let node = OwnedNode::new("Div").with_classes(["x"]);
        let c = sheet.compute(&node, None);
        assert_eq!(c.style.width, Some(crate::box_model::Length::Auto));
    }

    #[test]
    fn width_var_mistype_degrades_to_auto() {
        // A name bound to a Color is a type mismatch on the length path → Auto.
        let sheet = Stylesheet::parse(":root{--c:#fff} .x { width: var(--c); }").unwrap();
        let node = OwnedNode::new("Div").with_classes(["x"]);
        let c = sheet.compute(&node, None);
        assert_eq!(c.style.width, Some(crate::box_model::Length::Auto));
    }

    #[test]
    fn height_var_resolves() {
        let sheet = Stylesheet::parse(":root{--h:max(8)} .row { height: var(--h); }").unwrap();
        let node = OwnedNode::new("Div").with_classes(["row"]);
        let c = sheet.compute(&node, None);
        assert_eq!(c.style.height, Some(crate::box_model::Length::Max(8)));
    }

    #[test]
    fn width_var_undefined_uses_fallback() {
        // An undefined width var WITH a fallback resolves to the fallback,
        // mirroring the color var() path. (Lenient parse; no error.)
        let sheet = Stylesheet::parse(".x { width: var(--nope, 7); }").unwrap();
        let node = OwnedNode::new("Div").with_classes(["x"]);
        let c = sheet.compute(&node, None);
        assert_eq!(c.style.width, Some(crate::box_model::Length::Cells(7)));
    }
}
