//! Box-model value types: padding, margin, border, and sizing lengths.
//!
//! These are *descriptors*; they are projected onto ratatui primitives
//! (`Padding`, `Borders`/`BorderType`, `Constraint`) by methods in `style.rs`.

use ratatui::{
    layout::Constraint,
    widgets::{BorderType, Borders, Padding},
};

use crate::color::Color;
use crate::error::{CssError, Result};

// ---------------------------------------------------------------------------
// Padding / margin
// ---------------------------------------------------------------------------

/// One value per edge, in terminal cells (top, right, bottom, left).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BoxEdges {
    pub top: u16,
    pub right: u16,
    pub bottom: u16,
    pub left: u16,
}

impl BoxEdges {
    pub const fn uniform(v: u16) -> Self {
        Self { top: v, right: v, bottom: v, left: v }
    }

    pub const fn zero() -> Self {
        Self { top: 0, right: 0, bottom: 0, left: 0 }
    }

    /// Parse a CSS shorthand: `1`, `1 2`, `1 2 3`, or `1 2 3 4`.
    pub fn parse(shorthand: &str) -> Result<Self> {
        let parts: Vec<&str> = shorthand.split_whitespace().collect();
        let nums: Vec<u16> = parts
            .iter()
            .map(|p| {
                p.trim_end_matches("px").parse::<u16>().map_err(|_| CssError::invalid_length(shorthand))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(match nums.len() {
            0 => Self::zero(),
            1 => Self::uniform(nums[0]),
            2 => Self { top: nums[0], bottom: nums[0], left: nums[1], right: nums[1] },
            3 => Self { top: nums[0], left: nums[1], right: nums[1], bottom: nums[2] },
            n => Self {
                top: nums[0],
                right: nums[1],
                bottom: nums[2 % n],
                left: nums[3 % n],
            },
        })
    }

    /// Project onto a ratatui `Padding` (used for `Block::padding`).
    pub fn to_padding(self) -> Padding {
        Padding::new(self.left, self.right, self.top, self.bottom)
    }

    /// Shrink a `Rect` outward by these edges (for `margin`).
    pub fn shrink(self, area: ratatui::layout::Rect) -> ratatui::layout::Rect {
        let x = area.x.saturating_add(self.left);
        let y = area.y.saturating_add(self.top);
        let width = area
            .width
            .saturating_sub(self.left.saturating_add(self.right));
        let height = area
            .height
            .saturating_sub(self.top.saturating_add(self.bottom));
        ratatui::layout::Rect::new(x, y, width, height)
    }
}

// ---------------------------------------------------------------------------
// Border
// ---------------------------------------------------------------------------

/// Border drawing style. Width is implicit in the terminal (always 1 cell).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BorderStyle {
    /// No border.
    #[default]
    None,
    /// A plain single-line border.
    Single,
    /// A rounded single-line border (`border-radius`).
    Rounded,
    /// A double-line border.
    Double,
    /// A thick single-line border.
    Thick,
}

impl BorderStyle {
    pub fn to_border_type(self) -> Option<BorderType> {
        match self {
            Self::None => None,
            Self::Single => Some(BorderType::Plain),
            Self::Rounded => Some(BorderType::Rounded),
            Self::Double => Some(BorderType::Double),
            Self::Thick => Some(BorderType::Thick),
        }
    }
}

/// A full border declaration: style + optional color + optional per-edge set.
///
/// The `edges` field is the per-edge control point:
/// - `None` (the default) means "not explicitly declared". For backwards
///   compatibility a spec with a non-`None` style but `edges == None` still
///   draws **all four** edges (the legacy `.rounded` behavior) — see
///   [`BorderSpec::borders`].
/// - `Some(set)` selects exactly which edges (`Borders::TOP`, `LEFT`, etc.) are
///   drawn. This is set by the `border-top`/`border-right`/… declarations and by
///   the full `border` shorthand (which forces `Some(Borders::ALL)`).
#[derive(Debug, Clone, PartialEq)]
pub struct BorderSpec {
    pub style: BorderStyle,
    pub color: Option<Color>,
    pub edges: Option<Borders>,
}

impl Default for BorderSpec {
    fn default() -> Self {
        Self { style: BorderStyle::None, color: None, edges: None }
    }
}

impl BorderSpec {
    /// Render an edges set as a human-readable CSS-ish keyword string:
    /// `all`, `none`, `top`, `top|bottom`, `left|right`, etc. Edges are emitted
    /// in a stable order (top, right, bottom, left) joined by `|`.
    pub fn edges_to_keyword(edges: Borders) -> &'static str {
        // Order matters for readability: top, right, bottom, left.
        if edges == Borders::ALL {
            return "all";
        }
        if edges == Borders::NONE {
            return "none";
        }
        // Stable, pipe-joined. Leaks a 'static per distinct combination, but
        // there are only 16 combinations of 4 bits.
        let mut parts: Vec<&'static str> = Vec::new();
        if edges.contains(Borders::TOP) {
            parts.push("top");
        }
        if edges.contains(Borders::RIGHT) {
            parts.push("right");
        }
        if edges.contains(Borders::BOTTOM) {
            parts.push("bottom");
        }
        if edges.contains(Borders::LEFT) {
            parts.push("left");
        }
        match parts.len() {
            0 => "none",
            _ => Box::leak(parts.join("|").into_boxed_str()),
        }
    }

    /// Parse an edges keyword string (the inverse of [`Self::edges_to_keyword`])
    /// into a `Borders` set. Accepts `all`, `none`, any of the single edges
    /// (`top`/`right`/`bottom`/`left`), and `x`/`y` convenience aliases, plus
    /// `|`-separated combinations. Whitespace is tolerated.
    pub fn parse_edges(s: &str) -> Option<Borders> {
        let lower = s.trim().to_ascii_lowercase();
        if lower.is_empty() {
            return None;
        }
        let mut acc = Borders::NONE;
        for part in lower.split('|') {
            let part = part.trim();
            acc |= match part {
                "all" => Borders::ALL,
                "none" => Borders::NONE,
                "top" => Borders::TOP,
                "right" => Borders::RIGHT,
                "bottom" => Borders::BOTTOM,
                "left" => Borders::LEFT,
                "x" => Borders::LEFT | Borders::RIGHT,
                "y" => Borders::TOP | Borders::BOTTOM,
                _ => return None,
            };
        }
        Some(acc)
    }

    /// The ratatui `Borders` set this spec draws.
    ///
    /// A `BorderStyle::None` style draws nothing. Otherwise the explicit
    /// `edges` set is used, defaulting to `Borders::ALL` when `edges` is `None`
    /// (the legacy "style set without a per-edge declaration draws all four
    /// sides" semantics, kept so existing `.rounded { border-style: rounded }`
    /// rules keep drawing a full box).
    pub fn borders(&self) -> Borders {
        if self.style == BorderStyle::None {
            Borders::NONE
        } else {
            self.edges.unwrap_or(Borders::ALL)
        }
    }

    pub fn border_type(&self) -> BorderType {
        self.style.to_border_type().unwrap_or(BorderType::Plain)
    }

    /// Parse a CSS shorthand: `none` / `single` / `rounded` / `double` / `thick`,
    /// optionally with a width (`1px`) and a color (`rounded #f00`).
    pub fn parse_shorthand(s: &str) -> Result<Self> {
        let mut style = BorderStyle::None;
        let mut color_tokens: Vec<&str> = Vec::new();
        for tok in s.split_whitespace() {
            if tok.ends_with("px") {
                // width — present-but-ignored (terminal borders are always 1 cell).
                continue;
            }
            if let Some(parsed) = BorderStyle::parse_keyword(tok) {
                style = parsed;
            } else {
                color_tokens.push(tok);
            }
        }
        let color = if color_tokens.is_empty() {
            None
        } else {
            Some(Color::parse(&color_tokens.join(" "))?)
        };
        // The full `border` shorthand declares a *complete* border: edges are
        // set to ALL so that, e.g., `border: rounded` draws all four sides.
        // (Per-edge declarations like `border-bottom` set a subset instead.)
        Ok(Self { style, color, edges: Some(Borders::ALL) })
    }

    /// Merge another spec's *declared* sub-fields into this one in place.
    ///
    /// - `style` and `color` follow the existing sentinel rule (a non-`None`
    ///   style or `Some` color overrides; see below).
    /// - `edges` **accumulates** by OR when the other spec declares any: this
    ///   lets `.border-top` and `.border-bottom` compose into a top+bottom set
    ///   rather than one clobbering the other, mirroring how `.rounded` +
    ///   `.border-slate-700` compose on style/color.
    ///
    /// A sub-field counts as declared when its style is not
    /// [`BorderStyle::None`] (the default, reused as a "not declared"
    /// sentinel) or its color is `Some`. This is the per-declaration step of
    /// the cascade that lets two atomic rules — e.g. `.rounded` (style only)
    /// and `.border-slate-700` (color only) — compose into one border instead
    /// of one clobbering the other.
    pub fn merge(&mut self, other: &BorderSpec) {
        if other.style != BorderStyle::None {
            self.style = other.style;
        }
        if other.color.is_some() {
            self.color = other.color.clone();
        }
        // Per-edge declarations accumulate: `border-top` + `border-bottom`
        // → TOP | BOTTOM. A spec that never declares edges (the legacy
        // `border-style`/`border-color` path) leaves `self.edges` untouched.
        if let Some(oe) = other.edges {
            self.edges = Some(self.edges.unwrap_or(Borders::NONE) | oe);
        }
    }
}

impl BorderStyle {
    /// Parse a single keyword, case-insensitive.
    pub fn parse_keyword(s: &str) -> Option<Self> {
        Some(match s.to_ascii_lowercase().as_str() {
            "none" | "hidden" => Self::None,
            "single" | "solid" | "plain" => Self::Single,
            "rounded" => Self::Rounded,
            "double" => Self::Double,
            "thick" => Self::Thick,
            _ => return None,
        })
    }

    pub fn as_keyword(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Single => "single",
            Self::Rounded => "rounded",
            Self::Double => "double",
            Self::Thick => "thick",
        }
    }
}

// ---------------------------------------------------------------------------
// Length / sizing
// ---------------------------------------------------------------------------

/// A one-dimensional size, mapped to a ratatui `Constraint`.
///
/// Not `Copy`: the [`Length::Var`] variant carries a heap-allocated name, so a
/// `Length` must be `.clone()`-d when duplicated (which is rare outside the
/// cascade, where `var()` references have already been resolved away).
#[derive(Debug, Clone, PartialEq)]
pub enum Length {
    /// `auto` — let the layout engine decide (becomes `Min(0)`).
    Auto,
    /// A fixed cell count (`10`, `10px`).
    Cells(u16),
    /// A percentage of the available space (`50%`).
    Percent(u16),
    /// `min(n)` — at least `n` cells, grow if room.
    Min(u16),
    /// `max(n)` — at most `n` cells.
    Max(u16),
    /// A `var(--name)` reference, resolved against the token table during the
    /// cascade. A `Length::Var` should never survive into `to_constraint` — if
    /// one does (e.g. an unresolved variable in lenient mode), it degrades to
    /// `Min(0)` (same as [`Length::Auto`]) rather than panicking.
    ///
    /// Fallback (`var(--x, 10)`) is not yet supported: if a fallback is present
    /// it is currently ignored and only the name is captured.
    Var { name: String },
}

impl Length {
    pub fn parse(s: &str) -> Result<Self> {
        let s = s.trim();
        // var(--name) — recognized first, before any numeric/keyword logic.
        // A trailing fallback (e.g. `var(--x, 10)`) is tolerated by taking only
        // the name part; the fallback itself is not yet honored.
        if let Some(inner) = s
            .strip_prefix("var(")
            .or_else(|| s.strip_prefix("VAR("))
            .or_else(|| s.strip_prefix("Var("))
        {
            let inner = inner.strip_suffix(')').unwrap_or(inner);
            let name_part = inner.split(',').next().unwrap_or(inner);
            let name = name_part.trim().trim_start_matches('-').trim().to_string();
            if name.is_empty() {
                return Err(CssError::invalid_length(format!("var(): empty name in {s}")));
            }
            return Ok(Self::Var { name });
        }
        if s.eq_ignore_ascii_case("auto") || s.is_empty() {
            return Ok(Self::Auto);
        }
        if let Some(rest) = s.strip_prefix("min(").and_then(|r| r.strip_suffix(')')) {
            return Ok(Self::Min(parse_cells(rest)?));
        }
        if let Some(rest) = s.strip_prefix("max(").and_then(|r| r.strip_suffix(')')) {
            return Ok(Self::Max(parse_cells(rest)?));
        }
        if let Some(rest) = s.strip_suffix('%') {
            return Ok(Self::Percent(rest.parse().map_err(|_| CssError::invalid_length(s))?));
        }
        Ok(Self::Cells(parse_cells(s)?))
    }

    pub fn to_constraint(&self) -> Constraint {
        match self {
            Self::Auto => Constraint::Min(0),
            Self::Cells(n) => Constraint::Length(*n),
            Self::Percent(p) => Constraint::Percentage(*p),
            Self::Min(n) => Constraint::Min(*n),
            Self::Max(n) => Constraint::Max(*n),
            // Should have been resolved during the cascade; degrade like Auto.
            Self::Var { .. } => Constraint::Min(0),
        }
    }
}

fn parse_cells(s: &str) -> Result<u16> {
    s.trim_end_matches("px")
        .trim()
        .parse::<u16>()
        .map_err(|_| CssError::invalid_length(s))
}

// ---------------------------------------------------------------------------
// Conversion traits — typed (infallible) or string-shorthand input for the
// `CssStyle::padding` / `margin` / `border` builders.
// ---------------------------------------------------------------------------

/// Input accepted by [`crate::style::CssStyle::padding`] /
/// [`crate::style::CssStyle::margin`]: a typed value (zero panic) or a CSS
/// shorthand string (panics on a malformed literal).
///
/// - `u16` → uniform edges on all four sides.
/// - `(u16, u16)` → CSS two-value shorthand: `top = bottom = a`, `left = right = b`.
/// - `(u16, u16, u16, u16)` → `(top, right, bottom, left)`.
/// - `&str` → CSS shorthand (`"1"`, `"1 2"`, `"1 2 3"`, `"1 2 3 4"`); a bad
///   literal **panics**. Only use the string form for compile-time-known
///   literals — pass a `u16` or tuple for infallible construction.
pub trait IntoBoxEdges {
    fn into_edges(self) -> BoxEdges;
}

impl IntoBoxEdges for u16 {
    fn into_edges(self) -> BoxEdges {
        BoxEdges::uniform(self)
    }
}

impl IntoBoxEdges for (u16, u16) {
    fn into_edges(self) -> BoxEdges {
        let (a, b) = self;
        BoxEdges { top: a, bottom: a, left: b, right: b }
    }
}

impl IntoBoxEdges for (u16, u16, u16, u16) {
    fn into_edges(self) -> BoxEdges {
        let (top, right, bottom, left) = self;
        BoxEdges { top, right, bottom, left }
    }
}

impl IntoBoxEdges for &str {
    fn into_edges(self) -> BoxEdges {
        BoxEdges::parse(self)
            .expect("invalid padding/margin shorthand — pass a u16 or tuple for infallible construction")
    }
}

impl IntoBoxEdges for BoxEdges {
    fn into_edges(self) -> BoxEdges {
        self
    }
}

/// Input accepted by [`crate::style::CssStyle::border`]: a typed value (zero
/// panic) or a CSS shorthand string (panics on a malformed literal).
///
/// - [`BorderStyle`] → spec with that style and no color.
/// - `(BorderStyle, C) where C: Into<Color>` → spec with that style and color;
///   e.g. `(BorderStyle::Rounded, "#00d4ff")` or
///   `(BorderStyle::Rounded, RColor::Cyan)`.
/// - `&str` → CSS shorthand (`"rounded"`, `"rounded #f00"`, …); a bad literal
///   **panics**. Only use the string form for compile-time-known literals —
///   pass a `BorderStyle` or `(BorderStyle, color)` for infallible construction.
pub trait IntoBorderSpec {
    fn into_spec(self) -> BorderSpec;
}

impl IntoBorderSpec for BorderStyle {
    fn into_spec(self) -> BorderSpec {
        // edges: None → borders() falls back to ALL (legacy behavior).
        BorderSpec { style: self, color: None, edges: None }
    }
}

impl<C: Into<Color>> IntoBorderSpec for (BorderStyle, C) {
    fn into_spec(self) -> BorderSpec {
        let (style, color) = self;
        BorderSpec { style, color: Some(color.into()), edges: None }
    }
}

impl IntoBorderSpec for &str {
    fn into_spec(self) -> BorderSpec {
        BorderSpec::parse_shorthand(self)
            .expect("invalid border shorthand — pass a BorderStyle / (BorderStyle, color) for infallible construction")
    }
}

impl IntoBorderSpec for BorderSpec {
    fn into_spec(self) -> BorderSpec {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn border_spec_merge_keeps_declared_subfields() {
        use ratatui::style::Color as RC;
        // `.rounded` (style only) + `.border-blue` (color only) compose into
        // one spec rather than one clobbering the other.
        let mut a = BorderSpec { style: BorderStyle::Rounded, color: None, edges: None };
        let b = BorderSpec { style: BorderStyle::None, color: Some(Color::literal(RC::Blue)), edges: None };
        a.merge(&b);
        assert_eq!(a.style, BorderStyle::Rounded); // survived
        assert_eq!(a.color, Some(Color::literal(RC::Blue))); // applied

        // An all-default other (style=None, no color) declares nothing → merge
        // leaves the existing spec untouched.
        let mut c = BorderSpec { style: BorderStyle::Double, color: None, edges: None };
        c.merge(&BorderSpec::default());
        assert_eq!(c.style, BorderStyle::Double);
    }

    #[test]
    fn edges_shorthand() {
        assert_eq!(BoxEdges::parse("1").unwrap(), BoxEdges::uniform(1));
        let e = BoxEdges::parse("1 2").unwrap();
        assert_eq!((e.top, e.right, e.bottom, e.left), (1, 2, 1, 2));
        let e = BoxEdges::parse("1 2 3 4").unwrap();
        assert_eq!((e.top, e.right, e.bottom, e.left), (1, 2, 3, 4));
    }

    #[test]
    fn edges_shrink() {
        let area = ratatui::layout::Rect::new(0, 0, 10, 10);
        let inner = BoxEdges::uniform(1).shrink(area);
        assert_eq!((inner.x, inner.y, inner.width, inner.height), (1, 1, 8, 8));
    }

    #[test]
    fn length_parse() {
        assert_eq!(Length::parse("auto").unwrap(), Length::Auto);
        assert_eq!(Length::parse("10px").unwrap(), Length::Cells(10));
        assert_eq!(Length::parse("50%").unwrap(), Length::Percent(50));
        assert_eq!(Length::parse("min(3)").unwrap(), Length::Min(3));
    }

    #[test]
    fn length_var_parse() {
        assert_eq!(
            Length::parse("var(--w)").unwrap(),
            Length::Var { name: "w".into() }
        );
        // Numeric/percent still parse as before.
        assert_eq!(Length::parse("10").unwrap(), Length::Cells(10));
        assert_eq!(Length::parse("50%").unwrap(), Length::Percent(50));
        // A trailing fallback is tolerated: only the name is captured.
        assert_eq!(
            Length::parse("var(--w, 10)").unwrap(),
            Length::Var { name: "w".into() }
        );
        // Empty name is an error.
        assert!(Length::parse("var(--)").is_err());
    }

    #[test]
    fn length_var_degrades_to_min_zero() {
        // A Var that somehow reaches to_constraint degrades like Auto.
        assert_eq!(Length::Var { name: "x".into() }.to_constraint(), Constraint::Min(0));
    }

    #[test]
    fn into_box_edges_uniform() {
        let e: BoxEdges = 1u16.into_edges();
        assert_eq!(e, BoxEdges::uniform(1));
    }

    #[test]
    fn into_box_edges_pair() {
        let e: BoxEdges = (0u16, 2u16).into_edges();
        assert_eq!((e.top, e.right, e.bottom, e.left), (0, 2, 0, 2));
    }

    #[test]
    fn into_box_edges_quad() {
        let e: BoxEdges = (1u16, 2u16, 3u16, 4u16).into_edges();
        assert_eq!((e.top, e.right, e.bottom, e.left), (1, 2, 3, 4));
    }

    #[test]
    fn into_box_edges_string_matches_pair() {
        let typed = (0u16, 2u16).into_edges();
        let from_str: BoxEdges = "0 2".into_edges();
        assert_eq!(typed, from_str);
    }

    #[test]
    fn into_border_spec_style_only() {
        let spec = BorderStyle::Rounded.into_spec();
        assert_eq!(spec.style, BorderStyle::Rounded);
        assert_eq!(spec.color, None);
    }

    #[test]
    fn into_border_spec_with_color() {
        use ratatui::style::Color as RC;
        let spec = (BorderStyle::Double, "#ff0000").into_spec();
        assert_eq!(spec.style, BorderStyle::Double);
        assert_eq!(spec.color, Some(Color::literal(RC::Rgb(255, 0, 0))));
    }

    #[test]
    fn into_border_spec_string_matches() {
        let typed = BorderStyle::Single.into_spec();
        let from_str: BorderSpec = "single".into_spec();
        assert_eq!(typed.style, from_str.style);
        assert_eq!(typed.color, from_str.color);
    }

    // -----------------------------------------------------------------
    // Per-edge border
    // -----------------------------------------------------------------

    #[test]
    fn border_full_shorthand_all_edges() {
        // The full `border` shorthand declares edges == ALL.
        let spec = BorderSpec::parse_shorthand("rounded").unwrap();
        assert_eq!(spec.style, BorderStyle::Rounded);
        assert_eq!(spec.edges, Some(Borders::ALL));
        assert_eq!(spec.borders(), Borders::ALL);
    }

    #[test]
    fn border_style_only_legacy_all() {
        // A spec built the legacy way (style set, edges == None) still draws
        // all four edges — this is the regression-protected `.rounded` path.
        let spec = BorderSpec { style: BorderStyle::Rounded, color: None, edges: None };
        assert_eq!(spec.borders(), Borders::ALL);
    }

    #[test]
    fn border_none_style_draws_nothing_even_with_edges() {
        // A None style short-circuits to NONE regardless of edges.
        let spec = BorderSpec { style: BorderStyle::None, color: None, edges: Some(Borders::BOTTOM) };
        assert_eq!(spec.borders(), Borders::NONE);
    }

    #[test]
    fn per_edge_merge_accumulates() {
        // `.border-top` + `.border-bottom` compose into TOP | BOTTOM via merge,
        // mirroring how `.rounded` + `.border-color` compose on style/color.
        let mut a = BorderSpec { style: BorderStyle::Rounded, color: None, edges: Some(Borders::TOP) };
        let b = BorderSpec { style: BorderStyle::None, color: None, edges: Some(Borders::BOTTOM) };
        a.merge(&b);
        assert_eq!(a.style, BorderStyle::Rounded); // survived
        assert_eq!(a.edges, Some(Borders::TOP | Borders::BOTTOM));
        assert_eq!(a.borders(), Borders::TOP | Borders::BOTTOM);
    }

    #[test]
    fn per_edge_merge_legacy_none_edges_not_touched() {
        // A legacy spec (edges == None) merged into a per-edge spec must NOT
        // clobber the accumulated edges — merge only ORs when other declares.
        let mut a = BorderSpec { style: BorderStyle::Rounded, color: None, edges: Some(Borders::TOP) };
        let legacy = BorderSpec { style: BorderStyle::None, color: None, edges: None };
        a.merge(&legacy);
        assert_eq!(a.edges, Some(Borders::TOP)); // unchanged
    }

    #[test]
    fn per_edge_full_shorthand_then_edge_widens() {
        // A full `border: rounded` (edges=ALL) followed by a `border-bottom`
        // declaration: merge ORs ALL | BOTTOM == ALL (no narrowing). And a full
        // shorthand after edges keeps ALL.
        let mut a = BorderSpec { style: BorderStyle::Rounded, color: None, edges: Some(Borders::ALL) };
        let b = BorderSpec { style: BorderStyle::None, color: None, edges: Some(Borders::BOTTOM) };
        a.merge(&b);
        assert_eq!(a.edges, Some(Borders::ALL));
    }

    #[test]
    fn edges_keyword_roundtrip() {
        // edges_to_keyword emits in a fixed reading order (top, right, bottom,
        // left); parse_edges accepts that same order AND the reverse, so the
        // round-trip pairs below match the emit order exactly.
        for (keyword, edges) in [
            ("all", Borders::ALL),
            ("none", Borders::NONE),
            ("top", Borders::TOP),
            ("bottom", Borders::BOTTOM),
            ("top|bottom", Borders::TOP | Borders::BOTTOM),
            ("right|left", Borders::LEFT | Borders::RIGHT),
        ] {
            assert_eq!(BorderSpec::parse_edges(keyword), Some(edges), "parse {keyword}");
            assert_eq!(BorderSpec::edges_to_keyword(edges), keyword, "emit {keyword}");
        }
        // The reverse order parses back to the same set.
        assert_eq!(
            BorderSpec::parse_edges("left|right"),
            Some(Borders::LEFT | Borders::RIGHT)
        );
        // x / y convenience aliases parse but emit as right|left / top|bottom.
        assert_eq!(BorderSpec::parse_edges("x"), Some(Borders::LEFT | Borders::RIGHT));
        assert_eq!(BorderSpec::parse_edges("y"), Some(Borders::TOP | Borders::BOTTOM));
    }
}

// ---------------------------------------------------------------------------
// Optional serde
// ---------------------------------------------------------------------------

#[cfg(feature = "serde")]
mod serde_impl {
    use super::{BorderStyle, BorderSpec, BoxEdges, Length};
    use crate::color::Color;
    use ratatui::widgets::Borders;
    use serde::{de::Error, Deserialize, Deserializer, Serialize, Serializer};
    use serde_json::Value;

    impl<'de> Deserialize<'de> for BoxEdges {
        fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            match Value::deserialize(d)? {
                Value::Number(n) => {
                    let v = n.as_u64().unwrap_or(0) as u16;
                    Ok(BoxEdges::uniform(v))
                }
                Value::String(s) => BoxEdges::parse(&s).map_err(D::Error::custom),
                other => Err(D::Error::custom(format!("invalid padding/margin: {other}"))),
            }
        }
    }
    impl Serialize for BoxEdges {
        fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
            if self.top == self.right && self.right == self.bottom && self.bottom == self.left {
                s.serialize_u64(self.top as u64)
            } else {
                s.serialize_str(&format!(
                    "{} {} {} {}",
                    self.top, self.right, self.bottom, self.left
                ))
            }
        }
    }

    impl<'de> Deserialize<'de> for Length {
        fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            match Value::deserialize(d)? {
                Value::Number(n) => Ok(Length::Cells(n.as_u64().unwrap_or(0) as u16)),
                Value::String(s) => Length::parse(&s).map_err(D::Error::custom),
                other => Err(D::Error::custom(format!("invalid length: {other}"))),
            }
        }
    }
    impl Serialize for Length {
        fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
            match self {
                Length::Auto => s.serialize_str("auto"),
                Length::Cells(n) => s.serialize_str(&format!("{n}px")),
                Length::Percent(p) => s.serialize_str(&format!("{p}%")),
                Length::Min(n) => s.serialize_str(&format!("min({n})")),
                Length::Max(n) => s.serialize_str(&format!("max({n})")),
                Length::Var { name } => s.serialize_str(&format!("var(--{name})")),
            }
        }
    }

    impl<'de> Deserialize<'de> for BorderStyle {
        fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            let s = String::deserialize(d)?;
            BorderStyle::parse_keyword(&s)
                .ok_or_else(|| D::Error::custom(format!("invalid border style: {s}")))
        }
    }
    impl Serialize for BorderStyle {
        fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
            s.serialize_str(self.as_keyword())
        }
    }

    impl<'de> Deserialize<'de> for BorderSpec {
        fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            match Value::deserialize(d)? {
                Value::String(s) => BorderSpec::parse_shorthand(&s).map_err(D::Error::custom),
                Value::Object(map) => {
                    let style = match map.get("style") {
                        Some(v) => serde_json::from_value::<BorderStyle>(v.clone())
                            .map_err(D::Error::custom)?,
                        None => BorderStyle::None,
                    };
                    let color = match map.get("color") {
                        Some(Value::Null) | None => None,
                        Some(v) => Some(serde_json::from_value::<Color>(v.clone()).map_err(D::Error::custom)?),
                    };
                    // `edges` is optional and backwards-compatible: absent or
                    // null → None (legacy ALL-fallback). Accept either a
                    // keyword string ("top", "all", "top|left", …) or a raw
                    // bit integer.
                    let edges = match map.get("edges") {
                        Some(Value::Null) | None => None,
                        Some(Value::String(s)) => {
                            Some(BorderSpec::parse_edges(s).ok_or_else(|| {
                                D::Error::custom(format!("invalid edges: {s}"))
                            })?)
                        }
                        Some(Value::Number(n)) => {
                            let bits = n.as_u64().unwrap_or(0) as u8;
                            Some(Borders::from_bits(bits).unwrap_or(Borders::NONE))
                        }
                        Some(other) => {
                            return Err(D::Error::custom(format!(
                                "invalid edges: {other}"
                            )))
                        }
                    };
                    Ok(BorderSpec { style, color, edges })
                }
                other => Err(D::Error::custom(format!("invalid border: {other}"))),
            }
        }
    }
    impl Serialize for BorderSpec {
        fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
            use serde::ser::SerializeStruct;
            let mut st = s.serialize_struct("BorderSpec", 3)?;
            st.serialize_field("style", &self.style)?;
            st.serialize_field("color", &self.color)?;
            // edges as a readable keyword string (None stays null).
            match self.edges {
                None => st.serialize_field("edges", &None::<&str>)?,
                Some(e) => st.serialize_field("edges", BorderSpec::edges_to_keyword(e))?,
            }
            st.end()
        }
    }
}
