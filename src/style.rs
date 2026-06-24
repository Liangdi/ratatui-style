//! `CssStyle` — a CSS declaration block projected onto ratatui primitives.
//!
//! Every field is optional; `None` means "not declared", which is the cascade's
//! "do not override" signal (and mirrors ratatui's own `Option`-based `Style`).
//!
//! Projections:
//! - decoration fields → [`ratatui::style::Style`] via [`CssStyle::to_style`]
//! - box-model fields  → [`ratatui::widgets::Block`] / `Rect` shrink
//! - sizing fields     → [`ratatui::layout::Constraint`] / [`ratatui::layout::Alignment`]

use ratatui::{
    layout::{Alignment, Constraint, Rect},
    style::{Modifier, Style as RStyle},
    widgets::Block,
};

use crate::box_model::{
    BorderSpec, BorderStyle, BorderStyleValue, BoxEdgesValue, IntoBorderSpec, IntoBoxEdges, Length,
};
use crate::color::Color;
use crate::error::{CssError, Result};

// ---------------------------------------------------------------------------
// Property enums
// ---------------------------------------------------------------------------

/// `font-weight`.
///
/// **Terminal limitation**: ratatui (and terminals themselves) only carry a
/// single bold modifier bit (`Modifier::BOLD`), so there is no real 100–900
/// weight gradient. [`Weight::parse`] collapses any numeric weight to one of
/// two values: `≥ 600` → [`Bold`](Self::Bold), `< 600` →
/// [`Normal`](Self::Normal). Thus `500` is indistinguishable from `normal`,
/// and `600`–`900` are all equivalent to `bold`. This is a property of the
/// rendering target, not a parser shortcoming.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Weight {
    #[default]
    Normal,
    Bold,
}

impl Weight {
    /// Parse a `font-weight` value: `bold`/`bolder`, `normal`/`lighter`, or a
    /// numeric weight (≥600 → bold). Shared by the text parser and serde.
    pub fn parse(s: &str) -> Result<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "bold" | "bolder" => Ok(Self::Bold),
            "normal" | "lighter" | "" => Ok(Self::Normal),
            other => other
                .parse::<u32>()
                .map(|n| if n >= 600 { Self::Bold } else { Self::Normal })
                .map_err(|_| CssError::invalid_length(format!("font-weight: {s}"))),
        }
    }

    /// The CSS keyword for this weight.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Bold => "bold",
        }
    }
}

/// `opacity` — the terminal's coarse approximation of CSS alpha.
///
/// **Terminal limitation**: there is no real alpha channel, so CSS `opacity`
/// collapses to a single [`Modifier::DIM`] bit. [`Opacity::parse`] maps any
/// value below fully opaque to [`Dim`](Self::Dim): `0.5`, `50%`, and `0` all
/// dim the cell; only `1` / `100%` / `normal` stay bright. This mirrors how
/// [`Weight`] collapses the 100–900 gradient to a single bold bit — a property
/// of the rendering target, not a parser shortcoming.
///
/// Unlike `color`/`font-weight`/etc., `opacity` is **not inherited**, so it is
/// intentionally omitted from [`CssStyle::inherit_from`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Opacity {
    /// Fully opaque — no modifier (`opacity: 1`, `100%`, or `normal`).
    #[default]
    Full,
    /// Dimmed (`opacity < 1`, e.g. `0.5`, `50%`, `0`).
    Dim,
}

impl Opacity {
    /// Parse an `opacity` value: `normal`, or a number / percentage. Any value
    /// below fully opaque (`< 1` or `< 100%`) becomes [`Dim`](Self::Dim);
    /// `1`/`100%`/`normal` stay [`Full`](Self::Full). Shared by the text parser
    /// and serde.
    pub fn parse(s: &str) -> Result<Self> {
        let s = s.trim().to_ascii_lowercase();
        if s.is_empty() || s == "normal" {
            return Ok(Self::Full);
        }
        let (num_str, percent) = match s.strip_suffix('%') {
            Some(rest) => (rest, true),
            None => (s.as_str(), false),
        };
        let v: f32 = num_str
            .parse()
            .map_err(|_| CssError::invalid_length(format!("opacity: {s}")))?;
        let alpha = if percent { v / 100.0 } else { v };
        Ok(if alpha >= 1.0 { Self::Full } else { Self::Dim })
    }

    /// `true` when this opacity dims the cell.
    pub const fn is_dim(self) -> bool {
        matches!(self, Self::Dim)
    }

    /// The CSS representation — `1` for full, `0.5` as a representative dimmed
    /// value. Any `< 1` collapses to [`Dim`](Self::Dim), so the exact figure is
    /// lost in a round trip; `0.5` is a conventional mid point, and [`parse`]
    /// accepts it (or any other value) back.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Full => "1",
            Self::Dim => "0.5",
        }
    }
}

impl From<f64> for Opacity {
    /// `>= 1.0` → [`Full`](Self::Full); otherwise [`Dim`](Self::Dim).
    fn from(alpha: f64) -> Self {
        if alpha >= 1.0 {
            Self::Full
        } else {
            Self::Dim
        }
    }
}

impl From<f32> for Opacity {
    fn from(alpha: f32) -> Self {
        Opacity::from(alpha as f64)
    }
}

impl From<i32> for Opacity {
    /// Lets `.opacity(1)` / `.opacity(0)` work directly (integer literals).
    fn from(alpha: i32) -> Self {
        if alpha >= 1 {
            Self::Full
        } else {
            Self::Dim
        }
    }
}

/// `font-style`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FontStyle {
    #[default]
    Normal,
    Italic,
}

impl FontStyle {
    /// Parse a `font-style` value. Shared by the text parser and serde.
    pub fn parse(s: &str) -> Result<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "italic" | "oblique" => Ok(Self::Italic),
            "normal" | "" => Ok(Self::Normal),
            other => Err(CssError::invalid_selector(format!("font-style: {other}"))),
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Italic => "italic",
        }
    }
}

/// `text-decoration`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextDecoration {
    #[default]
    None,
    Underline,
    LineThrough,
    UnderlineLineThrough,
}

impl TextDecoration {
    fn modifiers(self) -> Option<Modifier> {
        match self {
            Self::None => None,
            Self::Underline => Some(Modifier::UNDERLINED),
            Self::LineThrough => Some(Modifier::CROSSED_OUT),
            Self::UnderlineLineThrough => Some(Modifier::UNDERLINED.union(Modifier::CROSSED_OUT)),
        }
    }

    /// Parse `text-decoration`: any whitespace-separated combo of `underline`
    /// and `line-through`/`strikethrough`. Never fails. Shared by the text
    /// parser and serde.
    pub fn parse(s: &str) -> Result<Self> {
        let lower = s.trim().to_ascii_lowercase();
        let u = lower.split_whitespace().any(|t| t == "underline");
        let l = lower
            .split_whitespace()
            .any(|t| t == "line-through" || t == "strikethrough");
        Ok(match (u, l) {
            (false, false) => Self::None,
            (true, false) => Self::Underline,
            (false, true) => Self::LineThrough,
            (true, true) => Self::UnderlineLineThrough,
        })
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Underline => "underline",
            Self::LineThrough => "line-through",
            Self::UnderlineLineThrough => "underline line-through",
        }
    }
}

/// `text-align`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Align {
    #[default]
    Left,
    Center,
    Right,
}

impl Align {
    pub fn to_alignment(self) -> Alignment {
        match self {
            Self::Left => Alignment::Left,
            Self::Center => Alignment::Center,
            Self::Right => Alignment::Right,
        }
    }

    /// Parse a `text-align` value. Shared by the text parser and serde.
    pub fn parse(s: &str) -> Result<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "left" | "justify" => Ok(Self::Left),
            "center" => Ok(Self::Center),
            "right" => Ok(Self::Right),
            other => Err(CssError::invalid_selector(format!("text-align: {other}"))),
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Center => "center",
            Self::Right => "right",
        }
    }
}

// ---------------------------------------------------------------------------
// CssStyle
// ---------------------------------------------------------------------------

/// A CSS declaration block.
///
/// Construct via builder, deserialize (with the `serde` feature), or receive as
/// a [`crate::cascade::ComputedStyle`] after cascade resolution.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CssStyle {
    // — decoration → Style —
    pub color: Option<Color>,
    pub background: Option<Color>,
    pub weight: Option<Weight>,
    pub font_style: Option<FontStyle>,
    pub decoration: Option<TextDecoration>,
    pub underline_color: Option<Color>,
    pub opacity: Option<Opacity>,

    // — box model → Block / Rect —
    pub padding: Option<BoxEdgesValue>,
    pub margin: Option<BoxEdgesValue>,
    pub border: Option<BorderSpec>,

    // — sizing → Constraint / Alignment —
    pub text_align: Option<Align>,
    pub width: Option<Length>,
    pub height: Option<Length>,
}

impl CssStyle {
    /// An empty declaration block (no declarations).
    pub fn new() -> Self {
        Self::default()
    }

    // --- builders ----------------------------------------------------------

    pub fn color(mut self, c: impl Into<Color>) -> Self {
        self.color = Some(c.into());
        self
    }
    pub fn background(mut self, c: impl Into<Color>) -> Self {
        self.background = Some(c.into());
        self
    }
    pub fn bold(mut self) -> Self {
        self.weight = Some(Weight::Bold);
        self
    }
    pub fn italic(mut self) -> Self {
        self.font_style = Some(FontStyle::Italic);
        self
    }
    /// Set `opacity`. A value below fully opaque (`< 1`, e.g. `0.5`) dims the
    /// cell via `Modifier::DIM`; `1`/`100%`/`normal` does not. Accepts an
    /// [`Opacity`] or any number (via `Into`): `.opacity(0.5)`, `.opacity(1)`.
    pub fn opacity(mut self, opacity: impl Into<Opacity>) -> Self {
        self.opacity = Some(opacity.into());
        self
    }
    pub fn underline(mut self) -> Self {
        self.decoration = Some(TextDecoration::Underline);
        self
    }

    /// Box-model builders. Accept typed input (zero panic) or a CSS shorthand
    /// string. Typed forms: `.padding(1)`, `.padding((0, 2))`,
    /// `.padding((1, 2, 3, 4))`, `.border(BorderStyle::Rounded)`,
    /// `.border((BorderStyle::Rounded, "#00d4ff"))`. The string shorthand
    /// (`.padding("1 2")`, `.border("rounded #f00")`) is kept for literal
    /// convenience but **panics** on a malformed value — only use it for
    /// compile-time-known literals. For data-driven input, deserialize instead.
    pub fn padding(mut self, edges: impl IntoBoxEdges) -> Self {
        self.padding = Some(edges.into_edges());
        self
    }
    pub fn margin(mut self, edges: impl IntoBoxEdges) -> Self {
        self.margin = Some(edges.into_edges());
        self
    }
    pub fn border(mut self, spec: impl IntoBorderSpec) -> Self {
        self.border = Some(spec.into_spec());
        self
    }

    /// Set only the border style (CSS `border-style`), leaving any border color
    /// already on this block intact. This is what makes Tailwind-style utilities
    /// like `.rounded` and `.border-slate-700` compose into one declaration.
    pub fn border_style(mut self, style: BorderStyle) -> Self {
        let mut spec = self.border.unwrap_or_default();
        spec.style = BorderStyleValue::Fixed(style);
        self.border = Some(spec);
        self
    }

    /// Set only the border color (CSS `border-color`), leaving any border style
    /// already on this block intact.
    pub fn border_color(mut self, color: impl Into<Color>) -> Self {
        let mut spec = self.border.unwrap_or_default();
        spec.color = Some(color.into());
        self.border = Some(spec);
        self
    }

    /// Borrow the border spec mutably, defaulting it if absent.
    ///
    /// Shared spine for every "apply a `border-*` sub-declaration" path — the
    /// text parser ([`crate::stylesheet::apply_decl`]), the serde
    /// deserializer, and the cascade overlay all funnel through here instead
    /// of repeating `border.clone().unwrap_or_default() → set → Some(…)`.
    pub(crate) fn border_mut(&mut self) -> &mut crate::box_model::BorderSpec {
        self.border.get_or_insert_default()
    }

    // --- cascade -----------------------------------------------------------

    /// Overlay `other` onto `self`: every field that is `Some` in `other`
    /// replaces the corresponding field in `self`. `None` fields are left
    /// untouched. This is the per-declaration step of the cascade.
    pub fn overlay(&mut self, other: &CssStyle) {
        macro_rules! over {
            ($f:ident) => {
                if other.$f.is_some() {
                    self.$f = other.$f.clone();
                }
            };
        }
        over!(color);
        over!(background);
        over!(weight);
        over!(font_style);
        over!(decoration);
        over!(underline_color);
        over!(opacity);
        over!(padding);
        over!(margin);
        // `border` cascades at the sub-field level via [`BorderSpec::merge`]:
        // `.rounded` (style) and `.border-slate-700` (color) compose into one
        // spec instead of one clobbering the other. A `None` style is "not
        // declared" and does not override; an explicit `BorderStyle::None` is
        // preserved.
        if let Some(other_border) = &other.border {
            self.border_mut().merge(other_border);
        }
        over!(text_align);
        over!(width);
        over!(height);
    }

    /// Fill inheritable fields that are still `None` from `parent`.
    /// Inheritable: `color`, `weight`, `font_style`, `decoration`,
    /// `underline_color`, `text_align`.
    pub fn inherit_from(&mut self, parent: &CssStyle) {
        if self.color.is_none() {
            self.color = parent.color.clone();
        }
        if self.weight.is_none() {
            self.weight = parent.weight;
        }
        if self.font_style.is_none() {
            self.font_style = parent.font_style;
        }
        if self.decoration.is_none() {
            self.decoration = parent.decoration;
        }
        if self.underline_color.is_none() {
            self.underline_color = parent.underline_color.clone();
        }
        if self.text_align.is_none() {
            self.text_align = parent.text_align;
        }
    }

    // --- projections -------------------------------------------------------

    /// Resolve a [`Color`] for direct application: `Literal`/`Reset` map to a
    /// ratatui color, `Var`/`Inherit` map to `None` (caller leaves it unset).
    /// After cascade resolution no `Var`/`Inherit` should remain.
    fn paint(c: &Color) -> Option<ratatui::style::Color> {
        match c {
            Color::Literal(lc) => Some(*lc),
            Color::Reset => Some(ratatui::style::Color::Reset),
            Color::Var { .. } | Color::Inherit => None,
        }
    }

    /// Build a ratatui `Style` from the decoration fields.
    pub fn to_style(&self) -> RStyle {
        let mut s = RStyle::default();
        if let Some(c) = self.color.as_ref().and_then(Self::paint) {
            s = s.fg(c);
        }
        if let Some(c) = self.background.as_ref().and_then(Self::paint) {
            s = s.bg(c);
        }
        if let Some(c) = self.underline_color.as_ref().and_then(Self::paint) {
            s = s.underline_color(c);
        }
        if self.weight == Some(Weight::Bold) {
            s = s.add_modifier(Modifier::BOLD);
        }
        if self.font_style == Some(FontStyle::Italic) {
            s = s.add_modifier(Modifier::ITALIC);
        }
        if let Some(m) = self.decoration.and_then(|d| d.modifiers()) {
            s = s.add_modifier(m);
        }
        if self.opacity == Some(Opacity::Dim) {
            s = s.add_modifier(Modifier::DIM);
        }
        s
    }

    /// Build a ratatui `Block` from the box-model fields.
    ///
    /// `border` sets borders/type/border-color; `padding` sets inner padding;
    /// `background` sets the block's fill style.
    ///
    /// A `BoxEdgesValue::Var` padding that survived the cascade degrades to no
    /// padding (mirroring how an unresolved color `Var` is dropped). Likewise a
    /// `BorderStyleValue::Var` border style degrades to `BorderStyle::None`.
    pub fn to_block(&self) -> Block<'_> {
        let mut block = Block::default();
        if let Some(b) = &self.border {
            block = block.borders(b.borders()).border_type(b.border_type());
            if let Some(c) = b.color.as_ref().and_then(Self::paint) {
                block = block.border_style(RStyle::default().fg(c));
            }
        }
        if let Some(BoxEdgesValue::Edges(pad)) = self.padding {
            block = block.padding(pad.to_padding());
        }
        if let Some(c) = self.background.as_ref().and_then(Self::paint) {
            block = block.style(RStyle::default().bg(c));
        }
        block
    }

    /// Shrink `area` by the `margin` edges, if any.
    ///
    /// An unresolved `BoxEdgesValue::Var` margin degrades to zero (the area is
    /// returned unchanged), mirroring `to_block`'s padding degradation.
    pub fn apply_margin(&self, area: Rect) -> Rect {
        match self.margin {
            Some(BoxEdgesValue::Edges(e)) => e.shrink(area),
            _ => area,
        }
    }

    /// `(width, height)` constraints, if either is declared.
    pub fn constraints(&self) -> Option<(Constraint, Constraint)> {
        if self.width.is_none() && self.height.is_none() {
            return None;
        }
        let w = self
            .width
            .as_ref()
            .map(|l| l.to_constraint())
            .unwrap_or(Constraint::Min(0));
        let h = self
            .height
            .as_ref()
            .map(|l| l.to_constraint())
            .unwrap_or(Constraint::Min(0));
        Some((w, h))
    }

    /// The resolved text alignment, if declared.
    pub fn alignment(&self) -> Option<Alignment> {
        self.text_align.map(|a| a.to_alignment())
    }

    /// `true` if no declarations are set.
    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }
}

// Convenience: literals coerce into `Color`.
impl From<ratatui::style::Color> for Color {
    fn from(c: ratatui::style::Color) -> Self {
        Color::Literal(c)
    }
}

// ---------------------------------------------------------------------------
// Optional serde — deserialize a CssStyle from a property map keyed by CSS
// property names; serialize back to the same shape.
// ---------------------------------------------------------------------------

#[cfg(feature = "serde")]
mod serde_impl {
    use super::{Align, BorderStyleValue, CssStyle, FontStyle, Opacity, TextDecoration, Weight};
    use crate::color::Color;
    use serde::de::{self, MapAccess, Visitor};
    use serde::ser::SerializeMap;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::fmt;

    // --- keyword enums (format-agnostic str visitors) ---

    impl<'de> Deserialize<'de> for Align {
        fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            struct AlignVisitor;
            impl<'de> Visitor<'de> for AlignVisitor {
                type Value = Align;
                fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                    f.write_str("a text-align keyword")
                }
                fn visit_str<E: de::Error>(self, v: &str) -> Result<Align, E> {
                    Align::parse(v).map_err(E::custom)
                }
                fn visit_string<E: de::Error>(self, v: String) -> Result<Align, E> {
                    Align::parse(&v).map_err(E::custom)
                }
            }
            d.deserialize_str(AlignVisitor)
        }
    }
    impl Serialize for Align {
        fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
            s.serialize_str(self.as_str())
        }
    }

    impl<'de> Deserialize<'de> for FontStyle {
        fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            struct FontStyleVisitor;
            impl<'de> Visitor<'de> for FontStyleVisitor {
                type Value = FontStyle;
                fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                    f.write_str("a font-style keyword")
                }
                fn visit_str<E: de::Error>(self, v: &str) -> Result<FontStyle, E> {
                    FontStyle::parse(v).map_err(E::custom)
                }
                fn visit_string<E: de::Error>(self, v: String) -> Result<FontStyle, E> {
                    FontStyle::parse(&v).map_err(E::custom)
                }
            }
            d.deserialize_str(FontStyleVisitor)
        }
    }
    impl Serialize for FontStyle {
        fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
            s.serialize_str(self.as_str())
        }
    }

    impl<'de> Deserialize<'de> for TextDecoration {
        fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            struct TextDecorationVisitor;
            impl<'de> Visitor<'de> for TextDecorationVisitor {
                type Value = TextDecoration;
                fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                    f.write_str("a text-decoration keyword")
                }
                fn visit_str<E: de::Error>(self, v: &str) -> Result<TextDecoration, E> {
                    TextDecoration::parse(v).map_err(E::custom)
                }
                fn visit_string<E: de::Error>(self, v: String) -> Result<TextDecoration, E> {
                    TextDecoration::parse(&v).map_err(E::custom)
                }
            }
            d.deserialize_str(TextDecorationVisitor)
        }
    }
    impl Serialize for TextDecoration {
        fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
            s.serialize_str(self.as_str())
        }
    }

    impl<'de> Deserialize<'de> for Weight {
        fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            // A font-weight may be a keyword ("bold"/"normal") or a numeric
            // weight (≥600 → bold). `deserialize_any` lets the same impl accept
            // a TOML/YAML/JSON integer or string with no intermediate Value.
            struct WeightVisitor;
            impl<'de> Visitor<'de> for WeightVisitor {
                type Value = Weight;
                fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                    f.write_str("a font-weight keyword or number")
                }
                fn visit_i64<E: de::Error>(self, v: i64) -> Result<Weight, E> {
                    Ok(if v >= 600 {
                        Weight::Bold
                    } else {
                        Weight::Normal
                    })
                }
                fn visit_u64<E: de::Error>(self, v: u64) -> Result<Weight, E> {
                    Ok(if v >= 600 {
                        Weight::Bold
                    } else {
                        Weight::Normal
                    })
                }
                fn visit_f64<E: de::Error>(self, v: f64) -> Result<Weight, E> {
                    Ok(if v >= 600.0 {
                        Weight::Bold
                    } else {
                        Weight::Normal
                    })
                }
                fn visit_str<E: de::Error>(self, v: &str) -> Result<Weight, E> {
                    Weight::parse(v).map_err(E::custom)
                }
                fn visit_string<E: de::Error>(self, v: String) -> Result<Weight, E> {
                    Weight::parse(&v).map_err(E::custom)
                }
            }
            d.deserialize_any(WeightVisitor)
        }
    }
    impl Serialize for Weight {
        fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
            s.serialize_str(self.as_str())
        }
    }

    impl<'de> Deserialize<'de> for Opacity {
        fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            // An opacity may be a keyword ("normal"), a percentage ("50%"), or a
            // number (0.5). `deserialize_any` accepts a TOML/YAML/JSON number or
            // string with no intermediate Value.
            struct OpacityVisitor;
            impl<'de> Visitor<'de> for OpacityVisitor {
                type Value = Opacity;
                fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                    f.write_str("an opacity keyword, number, or percentage string")
                }
                fn visit_i64<E: de::Error>(self, v: i64) -> Result<Opacity, E> {
                    Ok(if v >= 1 { Opacity::Full } else { Opacity::Dim })
                }
                fn visit_u64<E: de::Error>(self, v: u64) -> Result<Opacity, E> {
                    Ok(if v >= 1 { Opacity::Full } else { Opacity::Dim })
                }
                fn visit_f64<E: de::Error>(self, v: f64) -> Result<Opacity, E> {
                    Ok(if v >= 1.0 { Opacity::Full } else { Opacity::Dim })
                }
                fn visit_str<E: de::Error>(self, v: &str) -> Result<Opacity, E> {
                    Opacity::parse(v).map_err(E::custom)
                }
                fn visit_string<E: de::Error>(self, v: String) -> Result<Opacity, E> {
                    Opacity::parse(&v).map_err(E::custom)
                }
            }
            d.deserialize_any(OpacityVisitor)
        }
    }
    impl Serialize for Opacity {
        fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
            s.serialize_str(self.as_str())
        }
    }

    // -------------------------------------------------------------------------
    // CssStyle — deserialize a property map keyed by CSS property names.
    //
    // Format-agnostic: a `Visitor` whose `visit_map` walks entries and
    // dispatches each key to the typed leaf's own Deserialize via
    // `next_value::<T>()`. No `serde_json::Value` is materialized, so the same
    // path serves JSON objects, TOML tables, and YAML mappings. Null values
    // are handled by deserializing each field as `Option<T>` (serde's Option
    // visitor maps null/nil/unit to None and anything else to Some).
    // -------------------------------------------------------------------------

    impl<'de> Deserialize<'de> for CssStyle {
        fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            struct CssStyleVisitor;

            impl<'de> Visitor<'de> for CssStyleVisitor {
                type Value = CssStyle;

                fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                    f.write_str("a CSS style declaration map")
                }

                fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<CssStyle, A::Error> {
                    let mut s = CssStyle::default();
                    while let Some(key) = map.next_key::<String>()? {
                        match key.to_ascii_lowercase().as_str() {
                            "color" => {
                                s.color = map.next_value()?;
                            }
                            "background" | "background-color" => {
                                s.background = map.next_value()?;
                            }
                            "font-weight" => {
                                s.weight = map.next_value()?;
                            }
                            "font-style" => {
                                s.font_style = map.next_value()?;
                            }
                            "text-decoration" => {
                                s.decoration = map.next_value()?;
                            }
                            "underline-color" => {
                                s.underline_color = map.next_value()?;
                            }
                            "opacity" => {
                                s.opacity = map.next_value()?;
                            }
                            "padding" => {
                                s.padding = map.next_value()?;
                            }
                            "margin" => {
                                s.margin = map.next_value()?;
                            }
                            "border" => {
                                s.border = map.next_value()?;
                            }
                            "border-style" => {
                                let v: Option<BorderStyleValue> = map.next_value()?;
                                s.border_mut().style = v.unwrap_or_default();
                            }
                            "border-color" => {
                                let v: Option<Color> = map.next_value()?;
                                if let Some(c) = v {
                                    s.border_mut().color = Some(c);
                                }
                            }
                            "text-align" => {
                                s.text_align = map.next_value()?;
                            }
                            "width" => {
                                s.width = map.next_value()?;
                            }
                            "height" => {
                                s.height = map.next_value()?;
                            }
                            // Unknown property → read & discard (forward-compat).
                            _ => {
                                let _: de::IgnoredAny = map.next_value()?;
                            }
                        }
                    }
                    Ok(s)
                }
            }

            d.deserialize_map(CssStyleVisitor)
        }
    }

    impl Serialize for CssStyle {
        fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
            // Count present fields so the serializer can pre-size the map.
            let count = [
                self.color.is_some(),
                self.background.is_some(),
                self.weight.is_some(),
                self.font_style.is_some(),
                self.decoration.is_some(),
                self.underline_color.is_some(),
                self.opacity.is_some(),
                self.padding.is_some(),
                self.margin.is_some(),
                self.border.is_some(),
                self.text_align.is_some(),
                self.width.is_some(),
                self.height.is_some(),
            ]
            .iter()
            .filter(|&&b| b)
            .count();

            let mut map = s.serialize_map(Some(count))?;
            if let Some(v) = self.color.as_ref() {
                map.serialize_entry("color", v)?;
            }
            if let Some(v) = self.background.as_ref() {
                map.serialize_entry("background-color", v)?;
            }
            if let Some(v) = self.weight {
                map.serialize_entry("font-weight", &v)?;
            }
            if let Some(v) = self.font_style {
                map.serialize_entry("font-style", &v)?;
            }
            if let Some(v) = self.decoration {
                map.serialize_entry("text-decoration", &v)?;
            }
            if let Some(v) = self.underline_color.as_ref() {
                map.serialize_entry("underline-color", v)?;
            }
            if let Some(v) = self.opacity {
                map.serialize_entry("opacity", &v)?;
            }
            if let Some(v) = self.padding.as_ref() {
                map.serialize_entry("padding", v)?;
            }
            if let Some(v) = self.margin.as_ref() {
                map.serialize_entry("margin", v)?;
            }
            if let Some(v) = self.border.as_ref() {
                map.serialize_entry("border", v)?;
            }
            if let Some(v) = self.text_align {
                map.serialize_entry("text-align", &v)?;
            }
            if let Some(v) = self.width.as_ref() {
                map.serialize_entry("width", v)?;
            }
            if let Some(v) = self.height.as_ref() {
                map.serialize_entry("height", v)?;
            }
            map.end()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::box_model::{BorderStyle, BorderStyleValue, BoxEdges, BoxEdgesValue};
    use ratatui::style::Color as RC;

    #[test]
    fn to_style_maps_decoration() {
        let s = CssStyle::new()
            .color(RC::Red)
            .background(RC::Blue)
            .bold()
            .italic();
        let rs = s.to_style();
        assert_eq!(rs.fg, Some(RC::Red));
        assert_eq!(rs.bg, Some(RC::Blue));
        assert!(rs.add_modifier.contains(Modifier::BOLD));
        assert!(rs.add_modifier.contains(Modifier::ITALIC));
    }

    #[test]
    fn opacity_parse_collapses_to_dim_bit() {
        // Fully opaque (and the keyword) stay Full; anything below 1 dims.
        assert_eq!(Opacity::parse("1").unwrap(), Opacity::Full);
        assert_eq!(Opacity::parse("100%").unwrap(), Opacity::Full);
        assert_eq!(Opacity::parse("normal").unwrap(), Opacity::Full);
        assert_eq!(Opacity::parse("0.5").unwrap(), Opacity::Dim);
        assert_eq!(Opacity::parse("50%").unwrap(), Opacity::Dim);
        assert_eq!(Opacity::parse("0").unwrap(), Opacity::Dim);
        assert!(Opacity::parse("not-a-number").is_err());
    }

    #[test]
    fn opacity_builder_maps_to_dim_modifier() {
        let dim = CssStyle::new().opacity(0.5).to_style();
        assert!(dim.add_modifier.contains(Modifier::DIM));

        let full = CssStyle::new().opacity(1).to_style();
        assert!(!full.add_modifier.contains(Modifier::DIM));

        // Unset opacity never adds DIM.
        assert!(!CssStyle::new().to_style().add_modifier.contains(Modifier::DIM));
    }

    #[test]
    fn opacity_overlay_overrides() {
        let mut a = CssStyle::new().opacity(1); // Full
        a.overlay(&CssStyle::new().opacity(0.5)); // higher priority: Dim
        assert_eq!(a.opacity, Some(Opacity::Dim));
    }

    #[test]
    fn opacity_is_not_inherited() {
        // opacity does NOT inherit — a child with no opacity stays unset even
        // when the parent dims.
        let parent = CssStyle::new().opacity(0.5);
        let mut child = CssStyle::new().bold();
        child.inherit_from(&parent);
        assert_eq!(child.opacity, None);
    }

    #[test]
    fn overlay_only_sets_some() {
        let mut a = CssStyle::new().color(RC::Red).bold();
        let b = CssStyle::new().color(RC::Blue); // higher priority: color only
        a.overlay(&b);
        assert_eq!(a.color, Some(Color::literal(RC::Blue)));
        assert_eq!(a.weight, Some(Weight::Bold)); // survives
    }

    #[test]
    fn inherit_from_parent() {
        let parent = CssStyle::new().color(RC::Green).italic();
        let mut child = CssStyle::new().bold(); // no color
        child.inherit_from(&parent);
        assert_eq!(child.color, Some(Color::literal(RC::Green)));
        assert_eq!(child.weight, Some(Weight::Bold)); // own
        assert_eq!(child.font_style, Some(FontStyle::Italic)); // inherited
    }

    #[test]
    fn apply_margin_and_block() {
        let s = CssStyle::new();
        let area = Rect::new(0, 0, 10, 10);
        assert_eq!(s.apply_margin(area), area); // no margin
    }

    #[test]
    fn overlay_merges_border_subfields() {
        // `.rounded` declares only a style; `.border-slate-700` only a color.
        // The cascade must merge them rather than let one clobber the other.
        let mut a = CssStyle::new().border_style(BorderStyle::Rounded);
        let b = CssStyle::new().border_color(RC::Blue);
        a.overlay(&b);
        let border = a.border.as_ref().expect("border present");
        assert_eq!(border.style, BorderStyleValue::Fixed(BorderStyle::Rounded)); // survived
        assert_eq!(border.color, Some(Color::literal(RC::Blue))); // applied

        // A later full shorthand still wins on the sub-fields it sets.
        let c = CssStyle::new().border("single red");
        a.overlay(&c);
        let border = a.border.as_ref().expect("border present");
        assert_eq!(border.style, BorderStyleValue::Fixed(BorderStyle::Single));
        assert_eq!(border.color, Some(Color::literal(RC::Red)));
    }

    #[test]
    fn padding_typed_uniform() {
        let s = CssStyle::new().padding(1u16);
        assert_eq!(s.padding, Some(BoxEdgesValue::Edges(BoxEdges::uniform(1))));
    }

    #[test]
    fn padding_typed_pair() {
        let s = CssStyle::new().padding((0u16, 2u16));
        match s.padding.expect("padding") {
            BoxEdgesValue::Edges(e) => {
                assert_eq!((e.top, e.right, e.bottom, e.left), (0, 2, 0, 2));
            }
            other => panic!("expected Edges, got {other:?}"),
        }
    }

    #[test]
    fn padding_typed_quad() {
        let s = CssStyle::new().padding((1u16, 2u16, 3u16, 4u16));
        match s.padding.expect("padding") {
            BoxEdgesValue::Edges(e) => {
                assert_eq!((e.top, e.right, e.bottom, e.left), (1, 2, 3, 4));
            }
            other => panic!("expected Edges, got {other:?}"),
        }
    }

    #[test]
    fn padding_string_still_works() {
        assert_eq!(
            CssStyle::new().padding("0 2").padding,
            CssStyle::new().padding((0u16, 2u16)).padding
        );
    }

    #[test]
    fn border_typed_style_only() {
        let s = CssStyle::new().border(BorderStyle::Rounded);
        let b = s.border.expect("border");
        assert_eq!(b.style, BorderStyleValue::Fixed(BorderStyle::Rounded));
        assert_eq!(b.color, None);
    }

    #[test]
    fn border_typed_with_color() {
        let s = CssStyle::new().border((BorderStyle::Double, "#ff0000"));
        let b = s.border.expect("border");
        assert_eq!(b.style, BorderStyleValue::Fixed(BorderStyle::Double));
        assert_eq!(b.color, Some(Color::literal(RC::Rgb(255, 0, 0))));
    }

    #[test]
    fn border_string_still_works() {
        let typed = CssStyle::new().border(BorderStyle::Single).border;
        let from_str = CssStyle::new().border("single").border;
        assert_eq!(
            typed.map(|b| (b.style.clone(), b.color)),
            from_str.map(|b| (b.style.clone(), b.color))
        );
    }

    #[test]
    #[cfg(feature = "serde")]
    fn serde_border_style_and_color_compose() {
        // Two atomic border declarations deserialize into one merged spec —
        // the same Tailwind idiom the cascade exercises, but via the serde
        // path (which now funnels through `border_mut`).
        let json = r##"{ "border-style": "rounded", "border-color": "#334155" }"##;
        let s: CssStyle = serde_json::from_str(json).unwrap();
        let border = s.border.expect("border present");
        assert_eq!(border.style, BorderStyleValue::Fixed(BorderStyle::Rounded));
        assert_eq!(
            border.color,
            Some(Color::literal(ratatui::style::Color::Rgb(0x33, 0x41, 0x55)))
        );
        // The legacy serde path sets no edges → None (draws ALL via fallback).
        assert_eq!(border.edges, None);
    }

    #[test]
    #[cfg(feature = "serde")]
    fn serde_border_edges_roundtrip() {
        // An explicit edges set serializes to a readable keyword and round-trips
        // back through the object form.
        let original = CssStyle::new().border(crate::box_model::BorderSpec {
            style: BorderStyleValue::Fixed(BorderStyle::Rounded),
            color: None,
            edges: Some(ratatui::widgets::Borders::BOTTOM),
        });
        let json = serde_json::to_string(&original).unwrap();
        assert!(
            json.contains("\"edges\":\"bottom\""),
            "edges serialized as keyword: {json}"
        );
        let back: CssStyle = serde_json::from_str(&json).unwrap();
        let border = back.border.expect("border present");
        assert_eq!(border.edges, Some(ratatui::widgets::Borders::BOTTOM));
        assert_eq!(border.borders(), ratatui::widgets::Borders::BOTTOM);
    }

    #[test]
    #[cfg(feature = "serde")]
    fn serde_border_edges_absent_is_none() {
        // Backwards compatibility: an object without an `edges` key deserializes
        // to edges == None (legacy ALL-fallback).
        let json = r##"{ "style": "rounded", "color": "#f00" }"##;
        let spec: crate::box_model::BorderSpec = serde_json::from_str(json).unwrap();
        assert_eq!(spec.edges, None);
        assert_eq!(spec.borders(), ratatui::widgets::Borders::ALL);
    }

    #[test]
    fn to_block_resolved_edges_produce_padding() {
        let s = CssStyle::new().padding((1u16, 2u16, 3u16, 4u16));
        let block = s.to_block();
        // BoxEdges{top:1,right:2,bottom:3,left:4} shrinks a 10x10 area to
        // (x=4, y=1, width=4, height=6).
        let area = Rect::new(0, 0, 10, 10);
        assert_eq!(block.inner(area), Rect::new(4, 1, 4, 6));
    }

    #[test]
    fn to_block_unresolved_var_padding_is_noop() {
        // A BoxEdgesValue::Var that survived the cascade (shouldn't happen
        // post-resolution, but guarded) produces no padding.
        let mut s = CssStyle::new();
        s.padding = Some(BoxEdgesValue::var("pad"));
        let block = s.to_block();
        let area = Rect::new(0, 0, 10, 10);
        assert_eq!(block.inner(area), area);
    }

    #[test]
    fn apply_margin_unresolved_var_is_noop() {
        let mut s = CssStyle::new();
        s.margin = Some(BoxEdgesValue::var("m"));
        let area = Rect::new(0, 0, 10, 10);
        // An unresolved Var margin returns the area unchanged.
        assert_eq!(s.apply_margin(area), area);
    }

    #[test]
    fn overlay_merges_padding_margin_value_enums() {
        let mut a = CssStyle::new().padding(1u16);
        let b = CssStyle::new().padding(2u16).margin(3u16);
        a.overlay(&b);
        assert_eq!(
            a.padding,
            Some(BoxEdgesValue::Edges(BoxEdges::uniform(2)))
        );
        assert_eq!(
            a.margin,
            Some(BoxEdgesValue::Edges(BoxEdges::uniform(3)))
        );
    }

    #[test]
    #[cfg(feature = "serde")]
    fn serde_cssstyle_with_padding_margin_border_style() {
        // A fully-populated box-model CssStyle round-trips through serde,
        // including a var() border-style.
        let mut original = CssStyle::new()
            .padding((1u16, 2u16))
            .margin(3u16)
            .border(BorderStyle::Rounded);
        // Override the border style with a var (proves the value-enum survives).
        if let Some(spec) = original.border.as_mut() {
            spec.style = BorderStyleValue::var("bs");
        }
        let json = serde_json::to_string(&original).unwrap();
        let back: CssStyle = serde_json::from_str(&json).unwrap();
        assert_eq!(back, original, "serde round-trip mismatch\n{json}");
    }
}

// ---------------------------------------------------------------------------
// Cross-format serde round-trips (JSON / TOML / YAML).
// The crate advertises (design.md §1/§2, Cargo.toml feature comment) that
// styles can come from JSON, TOML, or YAML. These tests verify that promise
// holds for a fully-populated CssStyle: serialize to each format's string,
// deserialize back, and assert equality with the original.
// ---------------------------------------------------------------------------

#[cfg(all(test, feature = "serde"))]
mod cross_format_tests {
    use super::*;
    use crate::box_model::{BorderSpec, BorderStyle, BorderStyleValue, Length};
    use ratatui::style::Color as RC;

    /// A CssStyle exercising every leaf type that has a custom Deserialize:
    /// `Color` (color/background), `BoxEdges` (padding), `BorderSpec` (border
    /// with an explicit edges set), `Length` (width), `Weight`, and `Align`.
    fn populated_style() -> CssStyle {
        let mut s = CssStyle::new()
            .color(RC::Red)
            .background(RC::Rgb(0x33, 0x41, 0x55))
            .bold()
            .padding((1u16, 2u16, 3u16, 4u16))
            .border(BorderSpec {
                style: BorderStyleValue::Fixed(BorderStyle::Rounded),
                color: Some(Color::literal(RC::Blue)),
                edges: Some(ratatui::widgets::Borders::BOTTOM),
            });
        // `width` and `text_align` are plain fields (no builder).
        s.width = Some(Length::Cells(10));
        s.text_align = Some(Align::Center);
        s
    }

    #[test]
    fn json_roundtrip() {
        let original = populated_style();
        let json = serde_json::to_string(&original).expect("serialize to JSON");
        let back: CssStyle = serde_json::from_str(&json).expect("deserialize from JSON");
        assert_eq!(back, original, "JSON round-trip mismatch\n{json}");
    }

    #[test]
    fn toml_roundtrip() {
        let original = populated_style();
        let s = toml::to_string(&original).expect("serialize to TOML");
        let back: CssStyle = toml::from_str(&s).expect("deserialize from TOML");
        assert_eq!(back, original, "TOML round-trip mismatch\n{s}");
    }

    #[test]
    fn yaml_roundtrip() {
        let original = populated_style();
        let s = serde_yaml::to_string(&original).expect("serialize to YAML");
        let back: CssStyle = serde_yaml::from_str(&s).expect("deserialize from YAML");
        assert_eq!(back, original, "YAML round-trip mismatch\n{s}");
    }

    /// Deserializing a hand-written TOML document (not just a re-serialized
    /// one) — this is the realistic "styles in a config file" path and the
    /// place where JSON-coupled Deserialize impls actually break.
    #[test]
    fn toml_from_literal_doc() {
        // r##"..."## — double-hash delimiter so the "#334155" hex value
        // inside doesn't terminate a single-hash raw string.
        let doc = r##"
color = "red"
background-color = "#334155"
font-weight = "bold"
padding = "1 2 3 4"
width = "10px"
text-align = "center"

[border]
style = "rounded"
color = "blue"
edges = "bottom"
"##;
        let parsed: CssStyle = toml::from_str(doc).expect("deserialize TOML doc");
        assert_eq!(parsed, populated_style(), "TOML doc mismatch");
    }

    #[test]
    fn yaml_from_literal_doc() {
        let doc = r##"
color: red
background-color: "#334155"
font-weight: bold
padding: "1 2 3 4"
width: "10px"
text-align: center
border:
  style: rounded
  color: blue
  edges: bottom
"##;
        let parsed: CssStyle = serde_yaml::from_str(doc).expect("deserialize YAML doc");
        assert_eq!(parsed, populated_style(), "YAML doc mismatch");
    }

    /// The realistic config-file case that exposes JSON-coupling: a TOML doc
    /// that uses a **bare integer** for `width` and `padding` (not the string
    /// forms `"10px"` / `"1 2 3 4"` that re-serialization always emits). The
    /// `Length`/`BoxEdges` Deserialize impls accept either a number or a
    /// string in JSON; this verifies they also do so through the TOML
    /// deserializer.
    #[test]
    fn toml_bare_integers_for_length_and_padding() {
        let doc = r#"
width = 10
padding = 4
"#;
        let parsed: CssStyle = toml::from_str(doc).expect("deserialize TOML doc");
        assert_eq!(parsed.width, Some(Length::Cells(10)));
        assert_eq!(
            parsed.padding,
            Some(crate::box_model::BoxEdgesValue::Edges(
                crate::box_model::BoxEdges::uniform(4)
            ))
        );
    }
}
