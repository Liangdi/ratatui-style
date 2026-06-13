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

use crate::box_model::{BorderStyle, BorderSpec, BoxEdges, IntoBorderSpec, IntoBoxEdges, Length};
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
        let l = lower.split_whitespace().any(|t| t == "line-through" || t == "strikethrough");
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

    // — box model → Block / Rect —
    pub padding: Option<BoxEdges>,
    pub margin: Option<BoxEdges>,
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
        spec.style = style;
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
        s
    }

    /// Build a ratatui `Block` from the box-model fields.
    ///
    /// `border` sets borders/type/border-color; `padding` sets inner padding;
    /// `background` sets the block's fill style.
    pub fn to_block(&self) -> Block<'_> {
        let mut block = Block::default();
        if let Some(b) = &self.border {
            block = block.borders(b.borders()).border_type(b.border_type());
            if let Some(c) = b.color.as_ref().and_then(Self::paint) {
                block = block.border_style(RStyle::default().fg(c));
            }
        }
        if let Some(pad) = self.padding {
            block = block.padding(pad.to_padding());
        }
        if let Some(c) = self.background.as_ref().and_then(Self::paint) {
            block = block.style(RStyle::default().bg(c));
        }
        block
    }

    /// Shrink `area` by the `margin` edges, if any.
    pub fn apply_margin(&self, area: Rect) -> Rect {
        match self.margin {
            Some(e) => e.shrink(area),
            None => area,
        }
    }

    /// `(width, height)` constraints, if either is declared.
    pub fn constraints(&self) -> Option<(Constraint, Constraint)> {
        if self.width.is_none() && self.height.is_none() {
            return None;
        }
        let w = self.width.as_ref().map(|l| l.to_constraint()).unwrap_or(Constraint::Min(0));
        let h = self.height.as_ref().map(|l| l.to_constraint()).unwrap_or(Constraint::Min(0));
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
    use super::{Align, BorderStyle, CssStyle, FontStyle, TextDecoration, Weight};
    use serde::de::Error as DeError;
    use serde::ser::Error as SerError;
    use serde::{de::DeserializeOwned, Deserialize, Deserializer, Serialize, Serializer};
    use serde_json::{Map, Value};

    // --- keyword enums ---

    impl<'de> Deserialize<'de> for Align {
        fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            Align::parse(&String::deserialize(d)?).map_err(DeError::custom)
        }
    }
    impl Serialize for Align {
        fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
            s.serialize_str(self.as_str())
        }
    }

    impl<'de> Deserialize<'de> for FontStyle {
        fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            FontStyle::parse(&String::deserialize(d)?).map_err(DeError::custom)
        }
    }
    impl Serialize for FontStyle {
        fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
            s.serialize_str(self.as_str())
        }
    }

    impl<'de> Deserialize<'de> for Weight {
        fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            match Value::deserialize(d)? {
                // JSON may carry a bare number (≥600 → bold); everything else
                // funnels through the shared string parser.
                Value::Number(n) => Ok(if n.as_i64().map(|i| i >= 600).unwrap_or(false) {
                    Weight::Bold
                } else {
                    Weight::Normal
                }),
                Value::String(s) => Weight::parse(&s).map_err(DeError::custom),
                other => Err(DeError::custom(format!("invalid font-weight: {other}"))),
            }
        }
    }
    impl Serialize for Weight {
        fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
            s.serialize_str(self.as_str())
        }
    }

    impl<'de> Deserialize<'de> for TextDecoration {
        fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            TextDecoration::parse(&String::deserialize(d)?).map_err(DeError::custom)
        }
    }
    impl Serialize for TextDecoration {
        fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
            s.serialize_str(self.as_str())
        }
    }

    // --- CssStyle as a property map ---

    fn parse_opt<T: DeserializeOwned>(v: Value) -> Result<Option<T>, serde_json::Error> {
        if v.is_null() {
            return Ok(None);
        }
        serde_json::from_value(v).map(Some)
    }

    impl<'de> Deserialize<'de> for CssStyle {
        fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            let map = Map::<String, Value>::deserialize(d)?;
            let mut s = CssStyle::default();
            for (key, val) in map {
                let res: Result<(), &str> = match key.to_ascii_lowercase().as_str() {
                    "color" => parse_opt(val).map(|v| s.color = v).map_err(|_| "color"),
                    "background" | "background-color" => {
                        parse_opt(val).map(|v| s.background = v).map_err(|_| "background")
                    }
                    "font-weight" => parse_opt(val).map(|v| s.weight = v).map_err(|_| "font-weight"),
                    "font-style" => parse_opt(val).map(|v| s.font_style = v).map_err(|_| "font-style"),
                    "text-decoration" => {
                        parse_opt(val).map(|v| s.decoration = v).map_err(|_| "text-decoration")
                    }
                    "underline-color" => {
                        parse_opt(val).map(|v| s.underline_color = v).map_err(|_| "underline-color")
                    }
                    "padding" => parse_opt(val).map(|v| s.padding = v).map_err(|_| "padding"),
                    "margin" => parse_opt(val).map(|v| s.margin = v).map_err(|_| "margin"),
                    "border" => parse_opt(val).map(|v| s.border = v).map_err(|_| "border"),
                    "border-style" => parse_opt::<BorderStyle>(val)
                        .map(|v| s.border_mut().style = v.unwrap_or_default())
                        .map_err(|_| "border-style"),
                    "border-color" => parse_opt(val)
                        .map(|v| s.border_mut().color = v)
                        .map_err(|_| "border-color"),
                    "text-align" => parse_opt(val).map(|v| s.text_align = v).map_err(|_| "text-align"),
                    "width" => parse_opt(val).map(|v| s.width = v).map_err(|_| "width"),
                    "height" => parse_opt(val).map(|v| s.height = v).map_err(|_| "height"),
                    _ => continue, // unknown property → ignored (forward-compat)
                };
                res.map_err(|name| DeError::custom(format!("invalid {name} value")))?;
            }
            Ok(s)
        }
    }

    impl Serialize for CssStyle {
        fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
            let mut map = Map::new();
            macro_rules! put {
                ($key:expr, $field:expr) => {
                    if let Some(v) = $field {
                        map.insert($key.into(), serde_json::to_value(v).map_err(SerError::custom)?);
                    }
                };
            }
            put!("color", self.color.as_ref());
            put!("background-color", self.background.as_ref());
            put!("font-weight", self.weight);
            put!("font-style", self.font_style);
            put!("text-decoration", self.decoration);
            put!("underline-color", self.underline_color.as_ref());
            put!("padding", self.padding);
            put!("margin", self.margin);
            put!("border", self.border.as_ref());
            put!("text-align", self.text_align);
            put!("width", self.width.as_ref());
            put!("height", self.height.as_ref());
            map.serialize(s)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Color as RC;

    #[test]
    fn to_style_maps_decoration() {
        let s = CssStyle::new().color(RC::Red).background(RC::Blue).bold().italic();
        let rs = s.to_style();
        assert_eq!(rs.fg, Some(RC::Red));
        assert_eq!(rs.bg, Some(RC::Blue));
        assert!(rs.add_modifier.contains(Modifier::BOLD));
        assert!(rs.add_modifier.contains(Modifier::ITALIC));
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
        assert_eq!(border.style, BorderStyle::Rounded); // survived
        assert_eq!(border.color, Some(Color::literal(RC::Blue))); // applied

        // A later full shorthand still wins on the sub-fields it sets.
        let c = CssStyle::new().border("single red");
        a.overlay(&c);
        let border = a.border.as_ref().expect("border present");
        assert_eq!(border.style, BorderStyle::Single);
        assert_eq!(border.color, Some(Color::literal(RC::Red)));
    }

    #[test]
    fn padding_typed_uniform() {
        let s = CssStyle::new().padding(1u16);
        assert_eq!(s.padding, Some(BoxEdges::uniform(1)));
    }

    #[test]
    fn padding_typed_pair() {
        let s = CssStyle::new().padding((0u16, 2u16));
        let e = s.padding.expect("padding");
        assert_eq!((e.top, e.right, e.bottom, e.left), (0, 2, 0, 2));
    }

    #[test]
    fn padding_typed_quad() {
        let s = CssStyle::new().padding((1u16, 2u16, 3u16, 4u16));
        let e = s.padding.expect("padding");
        assert_eq!((e.top, e.right, e.bottom, e.left), (1, 2, 3, 4));
    }

    #[test]
    fn padding_string_still_works() {
        assert_eq!(CssStyle::new().padding("0 2").padding, CssStyle::new().padding((0u16, 2u16)).padding);
    }

    #[test]
    fn border_typed_style_only() {
        let s = CssStyle::new().border(BorderStyle::Rounded);
        let b = s.border.expect("border");
        assert_eq!(b.style, BorderStyle::Rounded);
        assert_eq!(b.color, None);
    }

    #[test]
    fn border_typed_with_color() {
        let s = CssStyle::new().border((BorderStyle::Double, "#ff0000"));
        let b = s.border.expect("border");
        assert_eq!(b.style, BorderStyle::Double);
        assert_eq!(b.color, Some(Color::literal(RC::Rgb(255, 0, 0))));
    }

    #[test]
    fn border_string_still_works() {
        let typed = CssStyle::new().border(BorderStyle::Single).border;
        let from_str = CssStyle::new().border("single").border;
        assert_eq!(typed.map(|b| (b.style, b.color)), from_str.map(|b| (b.style, b.color)));
    }

    #[test]
    #[cfg(feature = "serde")]
    fn serde_border_style_and_color_compose() {        // Two atomic border declarations deserialize into one merged spec —
        // the same Tailwind idiom the cascade exercises, but via the serde
        // path (which now funnels through `border_mut`).
        let json = r##"{ "border-style": "rounded", "border-color": "#334155" }"##;
        let s: CssStyle = serde_json::from_str(json).unwrap();
        let border = s.border.expect("border present");
        assert_eq!(border.style, BorderStyle::Rounded);
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
            style: BorderStyle::Rounded,
            color: None,
            edges: Some(ratatui::widgets::Borders::BOTTOM),
        });
        let json = serde_json::to_string(&original).unwrap();
        assert!(json.contains("\"edges\":\"bottom\""), "edges serialized as keyword: {json}");
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
}
