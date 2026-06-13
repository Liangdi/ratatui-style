//! Stylesheet — a collection of `Rule`s plus a token table, with a small CSS
//! text parser and a builder API.
//!
//! A stylesheet is parsed once and queried many times by the cascade engine.

use crate::box_model::Length;
use crate::color::Color;
use crate::error::{CssError, Loc, Result};
use crate::selector::Selector;
use crate::style::{Align, CssStyle, FontStyle, TextDecoration, Weight};
use crate::token::{ThemeTokens, Token};
use ratatui::widgets::Borders;

/// Cascade origin. Later origins override earlier ones at equal specificity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum Origin {
    /// Built-in defaults.
    #[default]
    UserAgent,
    /// Application-wide theme.
    Theme,
    /// End-user configuration (e.g. a config file).
    User,
    /// Inline declaration on the element itself — highest priority.
    Inline,
}

/// A flattened rule: one selector + one declaration block + provenance.
#[derive(Debug, Clone)]
pub struct RuleEntry {
    pub selector: Selector,
    pub style: CssStyle,
    pub origin: Origin,
    /// Insertion order within the stylesheet — the CSS "source order" tiebreaker.
    pub order: usize,
}

/// A parsed stylesheet.
#[derive(Debug, Clone, Default)]
pub struct Stylesheet {
    rules: Vec<RuleEntry>,
    tokens: ThemeTokens,
}

impl Stylesheet {
    pub fn new() -> Self {
        Self::default()
    }

    /// Construct with a token table (CSS custom properties).
    pub fn with_tokens(tokens: ThemeTokens) -> Self {
        Self { rules: Vec::new(), tokens }
    }

    pub fn tokens(&self) -> &ThemeTokens {
        &self.tokens
    }
    pub fn tokens_mut(&mut self) -> &mut ThemeTokens {
        &mut self.tokens
    }
    pub fn rules(&self) -> &[RuleEntry] {
        &self.rules
    }

    /// Add a rule from a selector string (may be a comma list) + style.
    pub fn add(&mut self, selectors: &str, style: CssStyle, origin: Origin) -> Result<&mut Self> {
        let order_base = self.rules.len();
        for sel in Selector::parse_list(selectors)? {
            self.rules.push(RuleEntry { selector: sel, style: style.clone(), origin, order: order_base });
        }
        Ok(self)
    }

    /// Add a single pre-parsed rule.
    pub fn add_rule(&mut self, selector: Selector, style: CssStyle, origin: Origin) -> &mut Self {
        let order = self.rules.len();
        self.rules.push(RuleEntry { selector, style, origin, order });
        self
    }

    /// Merge another stylesheet's rules and tokens into this one.
    pub fn extend(&mut self, other: &Stylesheet) {
        self.tokens.merge(&other.tokens);
        let offset = self.rules.len();
        for r in &other.rules {
            self.rules.push(RuleEntry { order: offset + r.order, ..r.clone() });
        }
    }

    /// Parse a CSS text document (lenient — the default).
    ///
    /// Supports `selector { prop: value; … }` blocks, comma selector lists,
    /// the universal `*`, `:root { --name: color; }` for tokens, and `/* … */`
    /// comments. Declarations use [`apply_decl`]'s property names.
    ///
    /// **Lenient behavior** (unchanged from earlier versions): unknown
    /// properties are silently ignored (forward-compat), and `var(--name)`
    /// references that are undefined at parse time are kept as-is and only
    /// degrade to `Reset` during the cascade. Use [`Stylesheet::parse_strict`]
    /// to turn both into hard errors.
    ///
    /// Parse errors now carry a 1-based [`Loc`] (line:column) pointing into the
    /// source text.
    pub fn parse(css: &str) -> Result<Self> {
        Self::parse_impl(css, /* strict */ false)
    }

    /// Parse a CSS text document in **strict** mode.
    ///
    /// Behaves like [`Stylesheet::parse`] except that two additional conditions
    /// become hard errors:
    ///
    /// 1. **Unknown property** — any declaration whose property name is not
    ///    recognized by [`apply_decl`] (and is not a `--`-prefixed custom
    ///    property). The error is [`CssErrorKind::UnknownProperty`] and is
    ///    tagged with the exact line:column of the property.
    /// 2. **Undefined variable** — any `var(--name)` with **no fallback** whose
    ///    name is not declared in this stylesheet's token table (either via a
    ///    `:root { --name: … }` block or an inline `--name:` declaration). The
    ///    error is [`CssErrorKind::UndefinedVariable`]. (It is reported with
    ///    `loc = None`; see the module note on var locations.) This covers both
    ///    color `var()` references and length `var()` references (`width`/`height`).
    ///
    /// A `var(--name, fallback)` always passes, since it has a fallback.
    ///
    /// [`CssErrorKind::UnknownProperty`]: crate::CssErrorKind::UnknownProperty
    /// [`CssErrorKind::UndefinedVariable`]: crate::CssErrorKind::UndefinedVariable
    pub fn parse_strict(css: &str) -> Result<Self> {
        Self::parse_impl(css, /* strict */ true)
    }

    fn parse_impl(css: &str, strict: bool) -> Result<Self> {
        // Position-preserving comment strip: the cleaned text is the same length
        // as `css` and has identical line structure, so a byte offset in
        // `cleaned` maps 1:1 to the same offset (and thus the same line:column)
        // in the original `css`.
        let cleaned = strip_comments(css);
        let mut sheet = Stylesheet::new();
        let mut rest = cleaned.as_str();
        // Byte offset of `rest` within `cleaned` (and thus within the original).
        let mut rest_off = 0usize;

        while let Some(rel) = rest.find('{') {
            let brace_off = rest_off + rel;
            // selector_part occupies cleaned[..brace_off] up to this rule; its
            // trim_start lands at the first non-whitespace char of the selector.
            let selector_part = rest[..rel].trim();
            rest = &rest[rel + 1..];
            rest_off = brace_off + 1;

            let close_rel = rest.find('}').ok_or_else(|| {
                CssError::invalid_selector("missing closing `}`")
                    .at(line_col(&cleaned, brace_off).line, 1)
            })?;
            let close_off = rest_off + close_rel;
            let body = &rest[..close_rel];
            let body_offset = rest_off;
            rest = &rest[close_rel + 1..];
            rest_off = close_off + 1;

            if selector_part.is_empty() {
                let loc = line_col(&cleaned, brace_off);
                return Err(CssError::invalid_selector("rule with no selector").at(loc.line, loc.column));
            }
            // Offset of the selector's first non-whitespace char (for loc).
            let sel_off = brace_off - selector_part.len();

            // `:root { --x: … }` declares tokens.
            let is_root = selector_part.split(',').all(|s| s.trim() == ":root");
            if is_root {
                for decl in split_declarations(body, body_offset) {
                    if let Some(name) = decl.prop.strip_prefix("--") {
                        let loc = line_col(&cleaned, decl.value_offset);
                        let token = parse_token_value(decl.value)
                            .map_err(|e| e.at(loc.line, loc.column))?;
                        sheet.tokens.insert(name.trim(), token);
                    }
                }
                continue;
            }

            let mut style = CssStyle::new();
            for decl in split_declarations(body, body_offset) {
                let prop = decl.prop.trim();
                let value = decl.value.trim();
                if prop.is_empty() {
                    continue;
                }
                if let Some(name) = prop.strip_prefix("--") {
                    let loc = line_col(&cleaned, decl.value_offset);
                    let token = parse_token_value(value).map_err(|e| e.at(loc.line, loc.column))?;
                    sheet.tokens.insert(name, token);
                } else {
                    if strict && !is_known_property(prop) {
                        let loc = line_col(&cleaned, decl.prop_offset);
                        return Err(CssError::unknown_property(prop).at(loc.line, loc.column));
                    }
                    let loc = line_col(&cleaned, decl.value_offset);
                    apply_decl(&mut style, prop, value).map_err(|e| e.at(loc.line, loc.column))?;
                }
            }
            // The selector string itself may be invalid (e.g. bad pseudo-class);
            // Selector::parse_list surfaces that as InvalidSelector. Tag it with
            // the selector's start offset if it does not already carry a loc.
            if let Err(mut e) = sheet.add(selector_part, style, Origin::User) {
                if e.loc.is_none() {
                    let loc = line_col(&cleaned, sel_off);
                    e = e.at(loc.line, loc.column);
                }
                return Err(e);
            }
        }

        if strict {
            // Any `var(--name)` with no fallback whose name is not in the token
            // table is an error. We scan every rule's declared colors …
            for rule in &sheet.rules {
                for color in color_refs(&rule.style) {
                    if let Some(Color::Var { name, fallback: None }) = color {
                        if sheet.tokens.get(name).is_none() {
                            return Err(CssError::undefined_variable(name.as_str()));
                        }
                    }
                }
                // … and its declared lengths (width/height Length::Var). A var
                // WITH a fallback is fine even if undefined; only a var with no
                // fallback and an unknown name is flagged — mirroring the color
                // strict check above.
                for length in length_refs(&rule.style) {
                    if let Some(Length::Var { name, fallback: None }) = length {
                        if sheet.tokens.get(name).is_none() {
                            return Err(CssError::undefined_variable(name.as_str()));
                        }
                    }
                }
            }
        }

        Ok(sheet)
    }

    /// Parse a CSS text document, tagging every rule with `origin`.
    ///
    /// Same parsing as [`parse`](Self::parse), but overrides the default
    /// [`Origin::User`] on each rule with `origin`. Used by the
    /// [`css!`](crate::css) macro (origin [`Origin::Theme`]) so that embedded
    /// rules can be overridden at runtime by [`Origin::User`] rules — see
    /// [`RuntimeStyle`](crate::RuntimeStyle).
    pub fn parse_with_origin(css: &str, origin: Origin) -> Result<Self> {
        let mut sheet = Self::parse(css)?;
        for rule in &mut sheet.rules {
            rule.origin = origin;
        }
        Ok(sheet)
    }
}

/// All `color`-carrying fields of a [`CssStyle`], for var scanning.
fn color_refs(style: &CssStyle) -> [Option<&Color>; 3] {
    [style.color.as_ref(), style.background.as_ref(), style.underline_color.as_ref()]
}

/// All `length`-carrying fields of a [`CssStyle`], for var scanning.
fn length_refs(style: &CssStyle) -> [Option<&Length>; 2] {
    [style.width.as_ref(), style.height.as_ref()]
}

/// Parse a custom-property value into a [`Token`], trying [`Color`] first then
/// [`Length`].
///
/// Color and length *literal* syntaxes don't overlap (`#fff`/`rgb()`/named
/// colors vs. `10`/`50%`/`auto`/`min(n)`), so for concrete values whichever
/// parser accepts the value wins. A bare `var(--other)` is the one case that is
/// syntactically valid in both grammars — its type is not knowable at parse
/// time, so it is stored as [`Token::Var`] and resolved by following the chain
/// via [`ThemeTokens::get_color`] / [`ThemeTokens::get_length`] at cascade time.
///
/// If both parsers reject the value, the error reported is the length parse
/// failure (which tends to be more descriptive for non-color garbage like
/// `width: banana`).
fn parse_token_value(value: &str) -> Result<Token> {
    // A bare var() reference defers its type until resolution.
    if let Ok(Color::Var { name, fallback: None }) = Color::parse(value) {
        return Ok(Token::Var { name });
    }
    match Color::parse(value) {
        Ok(c) => Ok(Token::Color(c)),
        Err(_) => Length::parse(value).map(Token::Length),
    }
}

/// Apply one `prop: value` declaration to a [`CssStyle`] (text form).
///
/// **Lenient by design**: unknown properties are silently ignored (the `_`
/// match arm) for forward-compatibility. The strict check lives in the parser
/// ([`Stylesheet::parse_strict`]) via [`is_known_property`] so that this public
/// helper stays non-breaking for ad-hoc callers.
pub fn apply_decl(style: &mut CssStyle, prop: &str, value: &str) -> Result<()> {
    let prop = prop.trim().to_ascii_lowercase();
    match prop.as_str() {
        "color" => style.color = Some(Color::parse(value)?),
        "background" | "background-color" => style.background = Some(Color::parse(value)?),
        "font-weight" => style.weight = Some(Weight::parse(value)?),
        "font-style" => style.font_style = Some(FontStyle::parse(value)?),
        "text-decoration" => style.decoration = Some(TextDecoration::parse(value)?),
        "underline-color" => style.underline_color = Some(Color::parse(value)?),
        "padding" => style.padding = Some(crate::box_model::BoxEdges::parse(value)?),
        "margin" => style.margin = Some(crate::box_model::BoxEdges::parse(value)?),
        "border" => {
            let mut spec = crate::box_model::BorderSpec::parse_shorthand(value)?;
            // The full shorthand declares a complete border → force all edges.
            spec.edges = Some(ratatui::widgets::Borders::ALL);
            style.border = Some(spec);
        }
        "border-style" => {
            let parsed = crate::box_model::BorderStyle::parse_keyword(value)
                .ok_or_else(|| CssError::invalid_length(format!("border-style: {value}")))?;
            style.border_mut().style = parsed;
        }
        "border-color" => {
            style.border_mut().color = Some(Color::parse(value)?);
        }
        // Per-edge declarations. Each parses the same `<style> [color]` value
        // grammar as `border`, then merges the resolved style/color *and* ORs
        // the corresponding edge(s) into the spec. This lets `.border-top` and
        // `.border-bottom` compose into a top+bottom set (edges accumulate via
        // `BorderSpec::merge` during the cascade), while a full `border: …`
        // still wins on style/color when it has higher priority.
        "border-top" => apply_per_edge(style, value, Borders::TOP)?,
        "border-right" => apply_per_edge(style, value, Borders::RIGHT)?,
        "border-bottom" => apply_per_edge(style, value, Borders::BOTTOM)?,
        "border-left" => apply_per_edge(style, value, Borders::LEFT)?,
        "border-x" => {
            apply_per_edge(style, value, Borders::LEFT | Borders::RIGHT)?
        }
        "border-y" => {
            apply_per_edge(style, value, Borders::TOP | Borders::BOTTOM)?
        }
        "text-align" => style.text_align = Some(Align::parse(value)?),
        "width" => style.width = Some(crate::box_model::Length::parse(value)?),
        "height" => style.height = Some(crate::box_model::Length::parse(value)?),
        _ => { /* unknown property → ignored (forward-compat) */ }
    }
    Ok(())
}

/// Apply a per-edge border declaration (`border-top`, `border-x`, …) to `style`.
///
/// The value follows the same `<style> [color]` grammar as the `border`
/// shorthand. The resolved style and color are merged onto the existing border
/// spec (so `.border-top: rounded` + a later `.border-color: red` still
/// compose), and the given edge set is OR-accumulated into `spec.edges` —
/// enabling `.border-top` and `.border-bottom` to add up to TOP | BOTTOM.
fn apply_per_edge(style: &mut CssStyle, value: &str, edges: Borders) -> Result<()> {
    // parse_shorthand yields style/color with edges=ALL; we override edges to
    // just the declared subset before merging, so the per-edge declaration
    // never widens the set to all four sides.
    let mut parsed = crate::box_model::BorderSpec::parse_shorthand(value)?;
    parsed.edges = Some(edges);
    style.border_mut().merge(&parsed);
    Ok(())
}

/// Whether `prop` is a property the text parser understands.
///
/// This is the **single source of truth** for the property name set; both
/// [`apply_decl`] (implicitly, via its match) and [`Stylesheet::parse_strict`]
/// (explicitly, via this predicate) consult the same list. A `--`-prefixed name
/// is a custom property and never counts as unknown.
fn is_known_property(prop: &str) -> bool {
    let p = prop.trim().to_ascii_lowercase();
    matches!(
        p.as_str(),
        "color"
            | "background"
            | "background-color"
            | "font-weight"
            | "font-style"
            | "text-decoration"
            | "underline-color"
            | "padding"
            | "margin"
            | "border"
            | "border-style"
            | "border-color"
            | "border-top"
            | "border-right"
            | "border-bottom"
            | "border-left"
            | "border-x"
            | "border-y"
            | "text-align"
            | "width"
            | "height"
    )
}

/// Replace `/* … */` comments with spaces, **keeping the input length and line
/// structure identical** so byte offsets map 1:1 back to the source.
///
/// Every character inside the comment (including the opening `/*` and closing
/// `*/`) becomes a space — **except** newlines, which are preserved. This keeps
/// `cleaned.len() == css.len()` and `cleaned.lines().count() == css.lines().count()`,
/// which is what makes [`line_col`] correct.
fn strip_comments(css: &str) -> String {
    let bytes = css.as_bytes();
    let mut out = String::with_capacity(css.len());
    let mut i = 0;
    while i < bytes.len() {
        if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            // Replace the opening `/*`.
            out.push(' ');
            out.push(' ');
            i += 2;
            // Blank out everything up to and including `*/`, preserving `\n`.
            while i < bytes.len() {
                if i + 1 < bytes.len() && bytes[i] == b'*' && bytes[i + 1] == b'/' {
                    out.push(' ');
                    out.push(' ');
                    i += 2;
                    break;
                }
                let b = bytes[i];
                out.push(if b == b'\n' { '\n' } else { ' ' });
                i += 1;
            }
            // Unterminated comment: the loop above runs off the end, which
            // preserves length — acceptable (no dedicated error).
        } else {
            // Push the original char (handles multi-byte UTF-8 safely because we
            // copy bytes that aren't part of a comment delimiter verbatim).
            let ch = css[i..].chars().next().expect("non-empty slice");
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

/// Convert a byte offset in `src` to a 1-based `Loc { line, column }`.
///
/// Column is counted in **bytes** from the start of the line (1-based). This is
/// the conventional choice for diagnostics and matches how editors report
/// columns for ASCII CSS; multi-byte characters will report a byte column.
fn line_col(src: &str, byte: usize) -> Loc {
    let byte = byte.min(src.len());
    let mut line: u32 = 1;
    let mut col: u32 = 1;
    for (i, b) in src.bytes().enumerate() {
        if i >= byte {
            break;
        }
        if b == b'\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }
    Loc::new(line, col)
}

/// One declaration parsed out of a rule body.
struct Decl<'a> {
    prop: &'a str,
    value: &'a str,
    /// Byte offset (within the original source) of the start of `prop`.
    prop_offset: usize,
    /// Byte offset (within the original source) of the start of `value`.
    value_offset: usize,
}

/// Split a rule body into declarations on `;`, honoring nested parentheses (so
/// `var(--x, rgb(1,2,3))` survives intact). `body_offset` is the byte offset of
/// `body` within the original source, used to compute per-declaration offsets.
fn split_declarations<'a>(body: &'a str, body_offset: usize) -> Vec<Decl<'a>> {
    let mut out = Vec::new();
    let mut depth: u32 = 0;
    let mut start = 0usize;
    let bytes = body.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'(' => depth += 1,
            b')' => depth = depth.saturating_sub(1),
            b';' if depth == 0 => {
                push_decl(&body[start..i], body_offset + start, &mut out);
                start = i + 1;
            }
            _ => {}
        }
    }
    push_decl(&body[start..], body_offset + start, &mut out);
    out
}

fn push_decl<'a>(chunk: &'a str, chunk_offset: usize, out: &mut Vec<Decl<'a>>) {
    // Find the trimmed prop/value, but compute their offsets relative to the
    // (already offset-adjusted) chunk start.
    let leading = chunk.len() - chunk.trim_start().len();
    let trimmed = &chunk[leading..];
    let trailing = trimmed.len() - trimmed.trim_end().len();
    let core = &trimmed[..trimmed.len() - trailing];
    if core.is_empty() {
        return;
    }
    let core_offset = chunk_offset + leading;
    if let Some(colon) = core.find(':') {
        let prop = &core[..colon];
        let value = &core[colon + 1..];
        out.push(Decl {
            prop,
            // value starts after the colon; trim leading whitespace for the
            // value *offset* so the loc points at the value, not the gap.
            value_offset: core_offset + colon + 1 + (value.len() - value.trim_start().len()),
            prop_offset: core_offset,
            value: value.trim(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::CssErrorKind;
    use crate::node::OwnedNode;
    use ratatui::style::Color as RColor;

    #[test]
    fn parse_text_stylesheet() {
        let css = r#"
            :root {
                --accent: #00d4ff;
            }
            /* a comment */
            Button.primary {
                color: var(--accent);
                background: blue;
                font-weight: bold;
            }
            #save:focus { text-decoration: underline; }
        "#;
        let sheet = Stylesheet::parse(css).unwrap();
        assert_eq!(sheet.tokens().get_color("accent"), Some(&Color::literal(RColor::Rgb(0, 212, 255))));
        // The `var()` reference is kept verbatim in the rule; resolved at compute time.
        let primary = sheet
            .rules()
            .iter()
            .find(|r| r.selector.classes.iter().any(|c| c == "primary"))
            .unwrap();
        assert_eq!(primary.style.color, Some(Color::var("accent")));
        assert!(sheet.rules().iter().any(|r| r.selector.id.as_deref() == Some("save")));
    }

    #[test]
    fn add_flattens_comma_list() {
        let mut sheet = Stylesheet::new();
        sheet.add("Text, .muted, #title", CssStyle::new(), Origin::User).unwrap();
        assert_eq!(sheet.rules().len(), 3);
    }

    #[test]
    fn parse_with_origin_sets_theme() {
        let sheet = Stylesheet::parse_with_origin("Button { color: red; }", Origin::Theme).unwrap();
        assert_eq!(sheet.rules()[0].origin, Origin::Theme);
    }

    #[test]
    fn border_style_and_color_compose_through_cascade() {
        // Two atomic utility classes — one for the border type, one for its
        // color — compose on a single element. This is the Tailwind idiom
        // (`rounded border-slate-700`) and only works because `border` cascades
        // at the sub-field level.
        let sheet = Stylesheet::parse(
            r#"
            .rounded          { border-style: rounded; }
            .border-slate-700 { border-color: #334155; }
            "#,
        )
        .unwrap();
        let node = OwnedNode::new("Div").with_classes(["rounded", "border-slate-700"]);
        let computed = sheet.compute(&node, None);
        let border = computed.style.border.expect("border present");
        assert_eq!(border.style, crate::box_model::BorderStyle::Rounded);
        assert_eq!(border.color, Some(Color::literal(RColor::Rgb(0x33, 0x41, 0x55))));
    }

    // -------------------------------------------------------------------------
    // Per-edge borders
    // -------------------------------------------------------------------------

    #[test]
    fn border_bottom_single_edge() {
        // A single per-edge declaration resolves to exactly that edge.
        let sheet = Stylesheet::parse(".b { border-bottom: rounded; }").unwrap();
        let node = OwnedNode::new("Div").with_classes(["b"]);
        let computed = sheet.compute(&node, None);
        let border = computed.style.border.expect("border present");
        assert_eq!(border.style, crate::box_model::BorderStyle::Rounded);
        assert_eq!(border.edges, Some(ratatui::widgets::Borders::BOTTOM));
        assert_eq!(border.borders(), ratatui::widgets::Borders::BOTTOM);
    }

    #[test]
    fn per_edge_cascade_accumulates_top_and_bottom() {
        // Two atomic per-edge classes compose into TOP | BOTTOM — the same
        // Tailwind-style composition idiom as `.rounded` + `.border-slate-700`.
        let sheet = Stylesheet::parse(
            r#"
            .bt { border-top: single; }
            .bb { border-bottom: single; }
            "#,
        )
        .unwrap();
        let node = OwnedNode::new("Div").with_classes(["bt", "bb"]);
        let computed = sheet.compute(&node, None);
        let border = computed.style.border.expect("border present");
        assert_eq!(border.edges, Some(ratatui::widgets::Borders::TOP | ratatui::widgets::Borders::BOTTOM));
    }

    #[test]
    fn per_edge_rendered_to_borders() {
        // A `border-bottom` rule computes to a block whose borders() yields only
        // BOTTOM — i.e. only the bottom edge is drawn.
        let sheet = Stylesheet::parse(".x { border-bottom: rounded red; }").unwrap();
        let node = OwnedNode::new("Div").with_classes(["x"]);
        let computed = sheet.compute(&node, None);
        let border = computed.style.border.expect("border present");
        assert_eq!(border.borders(), ratatui::widgets::Borders::BOTTOM);
        // color composes too.
        assert_eq!(border.color, Some(Color::literal(RColor::Red)));
    }

    #[test]
    fn per_edge_x_and_y_aliases() {
        // border-x → LEFT|RIGHT, border-y → TOP|BOTTOM.
        let sheet = Stylesheet::parse(
            r#"
            .bx { border-x: single; }
            .by { border-y: single; }
            "#,
        )
        .unwrap();
        let bx = sheet.compute(&OwnedNode::new("Div").with_classes(["bx"]), None);
        let by = sheet.compute(&OwnedNode::new("Div").with_classes(["by"]), None);
        assert_eq!(
            bx.style.border.as_ref().unwrap().edges,
            Some(ratatui::widgets::Borders::LEFT | ratatui::widgets::Borders::RIGHT)
        );
        assert_eq!(
            by.style.border.as_ref().unwrap().edges,
            Some(ratatui::widgets::Borders::TOP | ratatui::widgets::Borders::BOTTOM)
        );
    }

    #[test]
    fn full_border_shorthand_keeps_all_edges_under_compose() {
        // Regression guard: a full `border: rounded` shorthand (edges=ALL)
        // composed with a per-edge class stays ALL (never narrows).
        let sheet = Stylesheet::parse(
            r#"
            .full { border: rounded; }
            .bot  { border-bottom: single; }
            "#,
        )
        .unwrap();
        let node = OwnedNode::new("Div").with_classes(["full", "bot"]);
        let computed = sheet.compute(&node, None);
        let border = computed.style.border.expect("border present");
        assert_eq!(border.edges, Some(ratatui::widgets::Borders::ALL));
    }

    #[test]
    fn strict_mode_accepts_per_edge_properties() {
        // The new property names must be recognized in strict mode.
        Stylesheet::parse_strict(".x { border-bottom: rounded; }")
            .expect("border-bottom is a known property in strict mode");
    }

    // -------------------------------------------------------------------------
    // New: location tracking
    // -------------------------------------------------------------------------

    #[test]
    fn located_color_error() {
        // Line 3 holds the bad color.
        let css = "Button {\n    color: red;\n    background: #zzz;\n}\n";
        let err = Stylesheet::parse(css).unwrap_err();
        let loc = err.loc.expect("error has a location");
        assert_eq!(loc.line, 3, "line should point at the bad color's line");
        // sanity: it is indeed an invalid color error.
        assert!(matches!(err.kind, CssErrorKind::InvalidColor(_)));
    }

    #[test]
    fn comment_positions_preserved() {
        // A multi-line comment must not shift the line of the error below it.
        let css = "/* a\n   multi-line\n   comment */\nButton {\n    color: #nope;\n}\n";
        // cleaned must be the same length as the source.
        let cleaned = strip_comments(css);
        assert_eq!(cleaned.len(), css.len(), "strip_comments is length-preserving");
        // The bad color is on line 5 of the original.
        let err = Stylesheet::parse(css).unwrap_err();
        let loc = err.loc.expect("error has a location");
        assert_eq!(loc.line, 5);
        assert!(matches!(err.kind, CssErrorKind::InvalidColor(_)));
    }

    #[test]
    fn strip_comments_keeps_length_and_newlines() {
        let css = "a { color: red; /* x\ny */ color: blue; }";
        let cleaned = strip_comments(css);
        assert_eq!(cleaned.len(), css.len());
        assert_eq!(cleaned.matches('\n').count(), css.matches('\n').count());
        // the comment body is now spaces (plus the preserved newline).
        assert!(!cleaned.contains("/*"));
        assert!(!cleaned.contains("*/"));
    }

    // -------------------------------------------------------------------------
    // New: strict mode
    // -------------------------------------------------------------------------

    #[test]
    fn strict_unknown_property() {
        let err = Stylesheet::parse_strict("Foo { colr: red; }").unwrap_err();
        assert!(matches!(err.kind, CssErrorKind::UnknownProperty(ref p) if p == "colr"));
        // loc points at the property (line 1, somewhere on that line).
        let loc = err.loc.expect("unknown property has a location");
        assert_eq!(loc.line, 1);
    }

    #[test]
    fn strict_known_property_ok() {
        Stylesheet::parse_strict("Foo { color: red; }").expect("known property parses in strict mode");
    }

    #[test]
    fn strict_undefined_var() {
        let err = Stylesheet::parse_strict("Foo { color: var(--nope); }").unwrap_err();
        assert!(matches!(err.kind, CssErrorKind::UndefinedVariable(ref n) if n == "nope"));
    }

    #[test]
    fn strict_defined_var_ok() {
        Stylesheet::parse_strict(":root{--x:red;}\nFoo{color:var(--x);}").expect("defined var is fine");
    }

    #[test]
    fn strict_var_with_fallback_ok() {
        // A fallback makes even an undefined var acceptable in strict mode.
        Stylesheet::parse_strict("Foo { color: var(--nope, #fff); }")
            .expect("var with fallback does not error in strict mode");
    }

    #[test]
    fn lenient_parse_still_ignores_unknown() {
        // The default `parse` must NOT error on an unknown property.
        Stylesheet::parse("Foo { colr: red; }").expect("lenient parse ignores unknown property");
        // …nor on an undefined var.
        Stylesheet::parse("Foo { color: var(--nope); }").expect("lenient parse keeps undefined var");
    }

    // -------------------------------------------------------------------------
    // Length var() tokens (:root parsing) and strict coverage
    // -------------------------------------------------------------------------

    #[test]
    fn root_parses_length_token() {
        let sheet = Stylesheet::parse(":root{--w:22;--c:#fff;}").unwrap();
        assert_eq!(sheet.tokens().get_length("w"), Some(&crate::box_model::Length::Cells(22)));
        assert_eq!(
            sheet.tokens().get_color("c"),
            Some(&Color::literal(RColor::Rgb(0xff, 0xff, 0xff)))
        );
    }

    #[test]
    fn root_parses_length_percent_token() {
        let sheet = Stylesheet::parse(":root{--half:50%}").unwrap();
        assert_eq!(sheet.tokens().get_length("half"), Some(&crate::box_model::Length::Percent(50)));
    }

    #[test]
    fn root_rejects_garbage_token_value() {
        // Neither a valid color nor a valid length → error.
        assert!(Stylesheet::parse(":root{--x: banana;}").is_err());
    }

    #[test]
    fn strict_undefined_length_var() {
        let err = Stylesheet::parse_strict(".x{width:var(--nope)}").unwrap_err();
        assert!(matches!(err.kind, CssErrorKind::UndefinedVariable(ref n) if n == "nope"));
    }

    #[test]
    fn strict_defined_length_var_ok() {
        Stylesheet::parse_strict(":root{--w:10}.x{width:var(--w)}")
            .expect("defined length var is fine in strict mode");
    }
}
