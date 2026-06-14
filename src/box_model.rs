//! Box-model value types: padding, margin, border, and sizing lengths.
//!
//! These are *descriptors*; they are projected onto ratatui primitives
//! (`Padding`, `Borders`/`BorderType`, `Constraint`) by methods in `style.rs`.

use ratatui::{
    layout::Constraint,
    widgets::{BorderType, Borders, Padding},
};

use crate::color::{split_top_comma, Color};
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
        Self {
            top: v,
            right: v,
            bottom: v,
            left: v,
        }
    }

    pub const fn zero() -> Self {
        Self {
            top: 0,
            right: 0,
            bottom: 0,
            left: 0,
        }
    }

    /// Parse a CSS shorthand: `1`, `1 2`, `1 2 3`, or `1 2 3 4`.
    pub fn parse(shorthand: &str) -> Result<Self> {
        let parts: Vec<&str> = shorthand.split_whitespace().collect();
        let nums: Vec<u16> = parts
            .iter()
            .map(|p| {
                p.trim_end_matches("px")
                    .parse::<u16>()
                    .map_err(|_| CssError::invalid_length(shorthand))
            })
            .collect::<Result<Vec<_>>>()?;
        match nums.len() {
            0 => Ok(Self::zero()),
            1 => Ok(Self::uniform(nums[0])),
            2 => Ok(Self {
                top: nums[0],
                bottom: nums[0],
                left: nums[1],
                right: nums[1],
            }),
            3 => Ok(Self {
                top: nums[0],
                left: nums[1],
                right: nums[1],
                bottom: nums[2],
            }),
            // 4 values (the CSS shorthand maximum): top, right, bottom, left.
            4 => Ok(Self {
                top: nums[0],
                right: nums[1],
                bottom: nums[2],
                left: nums[3],
            }),
            // CSS shorthand allows at most 4 values.
            _ => Err(CssError::invalid_length(format!(
                "box shorthand allows at most 4 values, got {}: {shorthand}",
                nums.len()
            ))),
        }
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
    pub style: BorderStyleValue,
    pub color: Option<Color>,
    pub edges: Option<Borders>,
}

impl Default for BorderSpec {
    fn default() -> Self {
        Self {
            style: BorderStyleValue::Fixed(BorderStyle::None),
            color: None,
            edges: None,
        }
    }
}

impl BorderSpec {
    /// Render an edges set as a human-readable CSS-ish keyword string:
    /// `all`, `none`, `top`, `top|bottom`, `left|right`, etc. Edges are emitted
    /// in a stable order (top, right, bottom, left) joined by `|`.
    pub fn edges_to_keyword(edges: Borders) -> &'static str {
        // Borders is a 4-bit bitset (TOP=1, RIGHT=2, BOTTOM=4, LEFT=8), so
        // there are exactly 16 distinct combinations. Pre-compute every one
        // into a fixed table of `&'static str` literals — no allocation, no
        // `Box::leak`. Index by the raw bits (0..=15). Edges in each entry are
        // emitted in reading order: top, right, bottom, left.
        const EDGES_KW: [&str; 16] = [
            "none",              // 0000
            "top",               // 0001  TOP
            "right",             // 0010  RIGHT
            "top|right",         // 0011  TOP | RIGHT
            "bottom",            // 0100  BOTTOM
            "top|bottom",        // 0101  TOP | BOTTOM
            "right|bottom",      // 0110  RIGHT | BOTTOM
            "top|right|bottom",  // 0111  TOP | RIGHT | BOTTOM
            "left",              // 1000  LEFT
            "top|left",          // 1001  TOP | LEFT
            "right|left",        // 1010  RIGHT | LEFT
            "top|right|left",    // 1011  TOP | RIGHT | LEFT
            "bottom|left",       // 1100  BOTTOM | LEFT
            "top|bottom|left",   // 1101  TOP | BOTTOM | LEFT
            "right|bottom|left", // 1110  RIGHT | BOTTOM | LEFT
            "all",               // 1111  ALL
        ];
        let bits = edges.bits() as usize;
        if bits >= EDGES_KW.len() {
            // Defensive: a malformed bitset outside 0..=15. "none" is the
            // closest sensible keyword (no edges declared).
            return "none";
        }
        EDGES_KW[bits]
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
    ///
    /// A `BorderStyleValue::Var` that has not yet been resolved counts as
    /// "not None" (the edges are kept) — after cascade resolution no `Var`
    /// should survive, but this keeps the pre-resolution `borders()` sensible.
    pub fn borders(&self) -> Borders {
        if matches!(self.style.as_fixed(), Some(BorderStyle::None)) {
            Borders::NONE
        } else {
            self.edges.unwrap_or(Borders::ALL)
        }
    }

    pub fn border_type(&self) -> BorderType {
        self.style
            .as_fixed()
            .and_then(|s| s.to_border_type())
            .unwrap_or(BorderType::Plain)
    }

    /// Parse a CSS shorthand: `none` / `single` / `rounded` / `double` / `thick`
    /// (or a `var(--name)` reference), optionally with a width (`1px`) and a
    /// color (`rounded #f00`, `var(--bs) #f00`).
    ///
    /// A token starting with `var(` (case-insensitive) becomes a
    /// [`BorderStyleValue::Var`]; to support `var(--name, fallback)` with a
    /// fallback that may contain spaces, a `var(` token is re-joined with the
    /// remaining tokens up to the matching `)`. Otherwise the token is parsed
    /// as a [`BorderStyle`] keyword, and anything that is neither a keyword nor
    /// a `px` width becomes the color.
    pub fn parse_shorthand(s: &str) -> Result<Self> {
        let mut style: BorderStyleValue = BorderStyleValue::Fixed(BorderStyle::None);
        let mut color_tokens: Vec<&str> = Vec::new();
        let lowered = s.to_ascii_lowercase();
        let bytes = s.as_bytes();
        let mut i = 0;
        // Track consumed byte ranges so we can slice the original `s` for color
        // tokens that the whitespace split also visits.
        let mut consumed: Vec<(usize, usize)> = Vec::new();
        while i < bytes.len() {
            // Skip whitespace.
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if i >= bytes.len() {
                break;
            }
            let start = i;
            // Find the end of this token (next whitespace).
            while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            let tok = &s[start..i];
            // Case-insensitive `var(` prefix check on the lowered source.
            if lowered[start..i].starts_with("var(") {
                // Re-join tokens until the matching ')'. The `)` may be on a
                // later token, so consume forward.
                let mut joined = String::from(tok);
                // If this token doesn't end with ')', pull in more.
                while !joined.ends_with(')') && i < bytes.len() {
                    // Skip exactly one whitespace run and append it + next token.
                    let ws_start = i;
                    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                        i += 1;
                    }
                    if i >= bytes.len() {
                        break;
                    }
                    let t2_start = i;
                    while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
                        i += 1;
                    }
                    // Append the whitespace we skipped + the token.
                    joined.push_str(&s[ws_start..i]);
                    let _ = t2_start; // for clarity; the slice already covers it
                }
                // Ambiguity: a `var()` token could be either the style or the
                // color component of the shorthand. We treat it as a style ONLY
                // when no style has been set yet (neither a keyword nor a prior
                // var-style); once a style is set, a later `var()` is the color.
                // This makes both `var(--bs) #f00` (var as style) and
                // `rounded var(--rim)` (var as color) parse correctly.
                let style_already_set =
                    !matches!(style, BorderStyleValue::Fixed(BorderStyle::None));
                if style_already_set {
                    // Treat as a color token — push the FULL rejoined var(...)
                    // expression (which may span multiple whitespace-separated
                    // tokens) so a fallback like `var(--nope, #00ff00)` survives.
                    // `[start..i]` is a slice of the original `s` that covers the
                    // whole expression, so it satisfies the `&str` borrow.
                    color_tokens.push(&s[start..i]);
                } else {
                    consumed.push((start, i));
                    style = BorderStyleValue::parse(&joined)?;
                }
            } else if tok.ends_with("px") {
                // width — present-but-ignored (terminal borders are always 1 cell).
                consumed.push((start, start + tok.len()));
            } else if let Some(parsed) = BorderStyle::parse_keyword(tok) {
                consumed.push((start, start + tok.len()));
                style = BorderStyleValue::Fixed(parsed);
            } else {
                // Color token — handled in the second pass below.
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
        Ok(Self {
            style,
            color,
            edges: Some(Borders::ALL),
        })
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
    ///
    /// A `BorderStyleValue::Var` counts as declared (it may resolve to a
    /// non-`None` style), so it overrides any existing style.
    pub fn merge(&mut self, other: &BorderSpec) {
        let other_declares_style =
            !matches!(other.style, BorderStyleValue::Fixed(BorderStyle::None));
        if other_declares_style {
            self.style = other.style.clone();
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
// var()-carrying wrappers: BoxEdgesValue / BorderStyleValue
// ---------------------------------------------------------------------------

/// A `BoxEdges` value **or** a `var(--name)` reference, for `padding`/`margin`.
///
/// `BoxEdges` itself is `Copy` (4× `u16`) and cannot carry a heap `String`, so
/// the var name lives in this wrapper enum. The cascade
/// ([`crate::cascade`]) resolves `Var` against the token table; an unresolved
/// `Var` degrades to zero edges (mirroring how an unresolved color `Var`
/// degrades to `Reset`).
#[derive(Debug, Clone, PartialEq)]
pub enum BoxEdgesValue {
    /// A concrete set of edges.
    Edges(BoxEdges),
    /// A `var(--name[, fallback])` reference, resolved during the cascade.
    Var {
        name: String,
        fallback: Option<Box<BoxEdgesValue>>,
    },
}

impl BoxEdgesValue {
    /// Parse a CSS shorthand that may be a `var(--name)` reference.
    ///
    /// If `s` starts with `var(` (case-insensitive), the name and optional
    /// fallback are split on the first top-level comma (mirroring `Length::parse`
    /// / `Color::parse_var`); the fallback recurses via
    /// [`BoxEdgesValue::parse`]. Otherwise `s` is parsed as a concrete
    /// [`BoxEdges`] shorthand.
    pub fn parse(s: &str) -> Result<Self> {
        let s = s.trim();
        let lower = s.to_ascii_lowercase();
        if let Some(rest) = lower.strip_prefix("var(") {
            // `rest` still has the trailing ')'.
            let inner = rest.strip_suffix(')').unwrap_or(rest);
            let (name_part, fallback_part) = split_top_comma(inner);
            let name = name_part.trim().trim_start_matches('-').trim().to_string();
            if name.is_empty() {
                return Err(CssError::invalid_length(format!(
                    "var(): empty name in {s}"
                )));
            }
            let fallback = match fallback_part.trim() {
                "" => None,
                expr => Some(Box::new(Self::parse(expr)?)),
            };
            return Ok(Self::Var { name, fallback });
        }
        BoxEdges::parse(s).map(Self::Edges)
    }

    /// `true` if this is a `var()` reference that needs token resolution.
    pub fn is_var(&self) -> bool {
        matches!(self, Self::Var { .. })
    }

    /// Shortcut for `var(--name)` with no fallback.
    pub fn var(name: impl Into<String>) -> Self {
        Self::Var {
            name: name.into(),
            fallback: None,
        }
    }
}

impl From<BoxEdges> for BoxEdgesValue {
    fn from(e: BoxEdges) -> Self {
        Self::Edges(e)
    }
}

impl std::fmt::Display for BoxEdgesValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Edges(e) => {
                let e = *e;
                if e.top == e.right && e.right == e.bottom && e.bottom == e.left {
                    write!(f, "{}", e.top)
                } else {
                    write!(f, "{} {} {} {}", e.top, e.right, e.bottom, e.left)
                }
            }
            Self::Var { name, fallback } => match fallback {
                Some(fb) => write!(f, "var(--{name}, {fb})"),
                None => write!(f, "var(--{name})"),
            },
        }
    }
}

/// A `BorderStyle` value **or** a `var(--name)` reference, for `border-style`.
///
/// `BorderStyle` itself is `Copy` and cannot carry a heap `String`, so the var
/// name lives in this wrapper enum. The cascade resolves `Var` against the
/// token table; an unresolved `Var` degrades to `BorderStyle::None`.
#[derive(Debug, Clone, PartialEq)]
pub enum BorderStyleValue {
    /// A concrete border style.
    Fixed(BorderStyle),
    /// A `var(--name[, fallback])` reference, resolved during the cascade.
    Var {
        name: String,
        fallback: Option<Box<BorderStyleValue>>,
    },
}

impl BorderStyleValue {
    /// Parse a keyword or a `var(--name)` reference.
    ///
    /// If `s` starts with `var(` (case-insensitive), the name and optional
    /// fallback are split on the first top-level comma (mirroring
    /// [`BoxEdgesValue::parse`]); the fallback recurses via
    /// [`BorderStyleValue::parse`]. Otherwise `s` must be a valid
    /// [`BorderStyle`] keyword (error if not — mirroring the current
    /// `border-style` declaration behavior).
    pub fn parse(s: &str) -> Result<Self> {
        let s = s.trim();
        let lower = s.to_ascii_lowercase();
        if let Some(rest) = lower.strip_prefix("var(") {
            let inner = rest.strip_suffix(')').unwrap_or(rest);
            let (name_part, fallback_part) = split_top_comma(inner);
            let name = name_part.trim().trim_start_matches('-').trim().to_string();
            if name.is_empty() {
                return Err(CssError::invalid_length(format!(
                    "var(): empty name in {s}"
                )));
            }
            let fallback = match fallback_part.trim() {
                "" => None,
                expr => Some(Box::new(Self::parse(expr)?)),
            };
            return Ok(Self::Var { name, fallback });
        }
        match BorderStyle::parse_keyword(s) {
            Some(b) => Ok(Self::Fixed(b)),
            None => Err(CssError::invalid_length(format!("border-style: {s}"))),
        }
    }

    /// `true` if this is a `var()` reference that needs token resolution.
    pub fn is_var(&self) -> bool {
        matches!(self, Self::Var { .. })
    }

    /// Return the concrete [`BorderStyle`] if this is `Fixed`, else `None`
    /// (for an unresolved `Var`). Used by projections that need a concrete
    /// style; a `Var` degrades to `None` (treated as `BorderStyle::None`).
    pub fn as_fixed(&self) -> Option<BorderStyle> {
        match self {
            Self::Fixed(b) => Some(*b),
            Self::Var { .. } => None,
        }
    }

    /// Shortcut for `var(--name)` with no fallback.
    pub fn var(name: impl Into<String>) -> Self {
        Self::Var {
            name: name.into(),
            fallback: None,
        }
    }
}

impl From<BorderStyle> for BorderStyleValue {
    fn from(b: BorderStyle) -> Self {
        Self::Fixed(b)
    }
}

impl Default for BorderStyleValue {
    fn default() -> Self {
        Self::Fixed(BorderStyle::None)
    }
}

impl std::fmt::Display for BorderStyleValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Fixed(b) => f.write_str(b.as_keyword()),
            Self::Var { name, fallback } => match fallback {
                Some(fb) => write!(f, "var(--{name}, {fb})"),
                None => write!(f, "var(--{name})"),
            },
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
    Var {
        name: String,
        fallback: Option<Box<Length>>,
    },
}

impl Length {
    pub fn parse(s: &str) -> Result<Self> {
        let s = s.trim();
        // var(--name) — recognized first, before any numeric/keyword logic.
        // A fallback (e.g. `var(--x, 10)`) is split on the FIRST top-level comma
        // (honoring nested parens) and parsed as a Length — mirroring how
        // `Color::parse_var` handles the color var() fallback.
        if let Some(inner) = s
            .strip_prefix("var(")
            .or_else(|| s.strip_prefix("VAR("))
            .or_else(|| s.strip_prefix("Var("))
        {
            let inner = inner.strip_suffix(')').unwrap_or(inner);
            let (name_part, fallback_part) = split_top_comma(inner);
            let name = name_part.trim().trim_start_matches('-').trim().to_string();
            if name.is_empty() {
                return Err(CssError::invalid_length(format!(
                    "var(): empty name in {s}"
                )));
            }
            let fallback = match fallback_part.trim() {
                "" => None,
                expr => Some(Box::new(Self::parse(expr)?)),
            };
            return Ok(Self::Var { name, fallback });
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
            return Ok(Self::Percent(
                rest.parse().map_err(|_| CssError::invalid_length(s))?,
            ));
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
            // Should have been resolved during the cascade; if it reaches here,
            // prefer a fallback's constraint, else degrade like Auto (Min(0)).
            Self::Var {
                fallback: Some(fb), ..
            } => fb.to_constraint(),
            Self::Var { fallback: None, .. } => Constraint::Min(0),
        }
    }
}

fn parse_cells(s: &str) -> Result<u16> {
    s.trim_end_matches("px")
        .trim()
        .parse::<u16>()
        .map_err(|_| CssError::invalid_length(s))
}

/// Render a [`Length`] to its CSS string form (used by serde Serialize).
#[cfg(feature = "serde")]
fn length_to_css(length: &Length) -> String {
    match length {
        Length::Auto => "auto".to_string(),
        Length::Cells(n) => format!("{n}px"),
        Length::Percent(p) => format!("{p}%"),
        Length::Min(n) => format!("min({n})"),
        Length::Max(n) => format!("max({n})"),
        Length::Var {
            name,
            fallback: None,
        } => format!("var(--{name})"),
        Length::Var {
            name,
            fallback: Some(fb),
        } => {
            format!("var(--{name}, {})", length_to_css(fb))
        }
    }
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
/// - [`BoxEdges`] / [`BoxEdgesValue`] → passed through (identity). A
///   `BoxEdgesValue::Var` can be passed to declare a `var()` padding/margin
///   programmatically.
pub trait IntoBoxEdges {
    fn into_edges(self) -> BoxEdgesValue;
}

impl IntoBoxEdges for u16 {
    fn into_edges(self) -> BoxEdgesValue {
        BoxEdgesValue::Edges(BoxEdges::uniform(self))
    }
}

impl IntoBoxEdges for (u16, u16) {
    fn into_edges(self) -> BoxEdgesValue {
        let (a, b) = self;
        BoxEdgesValue::Edges(BoxEdges {
            top: a,
            bottom: a,
            left: b,
            right: b,
        })
    }
}

impl IntoBoxEdges for (u16, u16, u16, u16) {
    fn into_edges(self) -> BoxEdgesValue {
        let (top, right, bottom, left) = self;
        BoxEdgesValue::Edges(BoxEdges {
            top,
            right,
            bottom,
            left,
        })
    }
}

impl IntoBoxEdges for &str {
    fn into_edges(self) -> BoxEdgesValue {
        BoxEdgesValue::parse(self).expect(
            "invalid padding/margin shorthand — pass a u16 or tuple for infallible construction",
        )
    }
}

impl IntoBoxEdges for BoxEdges {
    fn into_edges(self) -> BoxEdgesValue {
        BoxEdgesValue::Edges(self)
    }
}

impl IntoBoxEdges for BoxEdgesValue {
    fn into_edges(self) -> BoxEdgesValue {
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
        BorderSpec {
            style: BorderStyleValue::Fixed(self),
            color: None,
            edges: None,
        }
    }
}

impl<C: Into<Color>> IntoBorderSpec for (BorderStyle, C) {
    fn into_spec(self) -> BorderSpec {
        let (style, color) = self;
        BorderSpec {
            style: BorderStyleValue::Fixed(style),
            color: Some(color.into()),
            edges: None,
        }
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
        let mut a = BorderSpec {
            style: BorderStyleValue::Fixed(BorderStyle::Rounded),
            color: None,
            edges: None,
        };
        let b = BorderSpec {
            style: BorderStyleValue::Fixed(BorderStyle::None),
            color: Some(Color::literal(RC::Blue)),
            edges: None,
        };
        a.merge(&b);
        assert_eq!(a.style, BorderStyleValue::Fixed(BorderStyle::Rounded)); // survived
        assert_eq!(a.color, Some(Color::literal(RC::Blue))); // applied

        // An all-default other (style=None, no color) declares nothing → merge
        // leaves the existing spec untouched.
        let mut c = BorderSpec {
            style: BorderStyleValue::Fixed(BorderStyle::Double),
            color: None,
            edges: None,
        };
        c.merge(&BorderSpec::default());
        assert_eq!(c.style, BorderStyleValue::Fixed(BorderStyle::Double));
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
    fn edges_shorthand_rejects_more_than_four() {
        // CSS shorthand allows at most 4 values; 5 must be an error.
        assert!(BoxEdges::parse("1 2 3 4 5").is_err());
        assert!(BoxEdges::parse("1 2 3 4 5 6").is_err());
        // 4 is still fine.
        assert!(BoxEdges::parse("1 2 3 4").is_ok());
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
            Length::Var {
                name: "w".into(),
                fallback: None
            }
        );
        // Numeric/percent still parse as before.
        assert_eq!(Length::parse("10").unwrap(), Length::Cells(10));
        assert_eq!(Length::parse("50%").unwrap(), Length::Percent(50));
        // A fallback is now captured and parsed as a Length.
        assert_eq!(
            Length::parse("var(--w, 10)").unwrap(),
            Length::Var {
                name: "w".into(),
                fallback: Some(Box::new(Length::Cells(10)))
            }
        );
        // A percent fallback parses to Percent.
        assert_eq!(
            Length::parse("var(--w, 50%)").unwrap(),
            Length::Var {
                name: "w".into(),
                fallback: Some(Box::new(Length::Percent(50)))
            }
        );
        // Empty name is an error.
        assert!(Length::parse("var(--)").is_err());
    }

    #[test]
    fn length_var_degrades_to_min_zero() {
        // A Var without a fallback that reaches to_constraint degrades like Auto.
        assert_eq!(
            Length::Var {
                name: "x".into(),
                fallback: None
            }
            .to_constraint(),
            Constraint::Min(0)
        );
        // A Var WITH a fallback uses the fallback's constraint.
        assert_eq!(
            Length::Var {
                name: "x".into(),
                fallback: Some(Box::new(Length::Cells(7)))
            }
            .to_constraint(),
            Constraint::Length(7)
        );
    }

    #[test]
    fn into_box_edges_uniform() {
        let e: BoxEdgesValue = 1u16.into_edges();
        assert_eq!(e, BoxEdgesValue::Edges(BoxEdges::uniform(1)));
    }

    #[test]
    fn into_box_edges_pair() {
        let e: BoxEdgesValue = (0u16, 2u16).into_edges();
        match e {
            BoxEdgesValue::Edges(e) => {
                assert_eq!((e.top, e.right, e.bottom, e.left), (0, 2, 0, 2));
            }
            other => panic!("expected Edges, got {other:?}"),
        }
    }

    #[test]
    fn into_box_edges_quad() {
        let e: BoxEdgesValue = (1u16, 2u16, 3u16, 4u16).into_edges();
        match e {
            BoxEdgesValue::Edges(e) => {
                assert_eq!((e.top, e.right, e.bottom, e.left), (1, 2, 3, 4));
            }
            other => panic!("expected Edges, got {other:?}"),
        }
    }

    #[test]
    fn into_box_edges_string_matches_pair() {
        let typed = (0u16, 2u16).into_edges();
        let from_str: BoxEdgesValue = "0 2".into_edges();
        assert_eq!(typed, from_str);
    }

    #[test]
    fn into_border_spec_style_only() {
        let spec = BorderStyle::Rounded.into_spec();
        assert_eq!(spec.style, BorderStyleValue::Fixed(BorderStyle::Rounded));
        assert_eq!(spec.color, None);
    }

    #[test]
    fn into_border_spec_with_color() {
        use ratatui::style::Color as RC;
        let spec = (BorderStyle::Double, "#ff0000").into_spec();
        assert_eq!(spec.style, BorderStyleValue::Fixed(BorderStyle::Double));
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
        assert_eq!(spec.style, BorderStyleValue::Fixed(BorderStyle::Rounded));
        assert_eq!(spec.edges, Some(Borders::ALL));
        assert_eq!(spec.borders(), Borders::ALL);
    }

    #[test]
    fn border_style_only_legacy_all() {
        // A spec built the legacy way (style set, edges == None) still draws
        // all four edges — this is the regression-protected `.rounded` path.
        let spec = BorderSpec {
            style: BorderStyleValue::Fixed(BorderStyle::Rounded),
            color: None,
            edges: None,
        };
        assert_eq!(spec.borders(), Borders::ALL);
    }

    #[test]
    fn border_none_style_draws_nothing_even_with_edges() {
        // A None style short-circuits to NONE regardless of edges.
        let spec = BorderSpec {
            style: BorderStyleValue::Fixed(BorderStyle::None),
            color: None,
            edges: Some(Borders::BOTTOM),
        };
        assert_eq!(spec.borders(), Borders::NONE);
    }

    #[test]
    fn per_edge_merge_accumulates() {
        // `.border-top` + `.border-bottom` compose into TOP | BOTTOM via merge,
        // mirroring how `.rounded` + `.border-color` compose on style/color.
        let mut a = BorderSpec {
            style: BorderStyleValue::Fixed(BorderStyle::Rounded),
            color: None,
            edges: Some(Borders::TOP),
        };
        let b = BorderSpec {
            style: BorderStyleValue::Fixed(BorderStyle::None),
            color: None,
            edges: Some(Borders::BOTTOM),
        };
        a.merge(&b);
        assert_eq!(a.style, BorderStyleValue::Fixed(BorderStyle::Rounded)); // survived
        assert_eq!(a.edges, Some(Borders::TOP | Borders::BOTTOM));
        assert_eq!(a.borders(), Borders::TOP | Borders::BOTTOM);
    }

    #[test]
    fn per_edge_merge_legacy_none_edges_not_touched() {
        // A legacy spec (edges == None) merged into a per-edge spec must NOT
        // clobber the accumulated edges — merge only ORs when other declares.
        let mut a = BorderSpec {
            style: BorderStyleValue::Fixed(BorderStyle::Rounded),
            color: None,
            edges: Some(Borders::TOP),
        };
        let legacy = BorderSpec {
            style: BorderStyleValue::Fixed(BorderStyle::None),
            color: None,
            edges: None,
        };
        a.merge(&legacy);
        assert_eq!(a.edges, Some(Borders::TOP)); // unchanged
    }

    #[test]
    fn per_edge_full_shorthand_then_edge_widens() {
        // A full `border: rounded` (edges=ALL) followed by a `border-bottom`
        // declaration: merge ORs ALL | BOTTOM == ALL (no narrowing). And a full
        // shorthand after edges keeps ALL.
        let mut a = BorderSpec {
            style: BorderStyleValue::Fixed(BorderStyle::Rounded),
            color: None,
            edges: Some(Borders::ALL),
        };
        let b = BorderSpec {
            style: BorderStyleValue::Fixed(BorderStyle::None),
            color: None,
            edges: Some(Borders::BOTTOM),
        };
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
            assert_eq!(
                BorderSpec::parse_edges(keyword),
                Some(edges),
                "parse {keyword}"
            );
            assert_eq!(
                BorderSpec::edges_to_keyword(edges),
                keyword,
                "emit {keyword}"
            );
        }
        // The reverse order parses back to the same set.
        assert_eq!(
            BorderSpec::parse_edges("left|right"),
            Some(Borders::LEFT | Borders::RIGHT)
        );
        // x / y convenience aliases parse but emit as right|left / top|bottom.
        assert_eq!(
            BorderSpec::parse_edges("x"),
            Some(Borders::LEFT | Borders::RIGHT)
        );
        assert_eq!(
            BorderSpec::parse_edges("y"),
            Some(Borders::TOP | Borders::BOTTOM)
        );
    }

    #[test]
    fn edges_to_keyword_is_leak_free_and_covers_all_16() {
        // The function returns &'static str literals from a fixed 16-entry
        // table (no Box::leak). Verify every possible 4-bit Borders set emits
        // the right keyword (reading order: top, right, bottom, left) and that
        // each keyword round-trips back through parse_edges.
        let combos: [(Borders, &str); 16] = [
            (Borders::NONE, "none"),
            (Borders::TOP, "top"),
            (Borders::RIGHT, "right"),
            (Borders::TOP | Borders::RIGHT, "top|right"),
            (Borders::BOTTOM, "bottom"),
            (Borders::TOP | Borders::BOTTOM, "top|bottom"),
            (Borders::RIGHT | Borders::BOTTOM, "right|bottom"),
            (
                Borders::TOP | Borders::RIGHT | Borders::BOTTOM,
                "top|right|bottom",
            ),
            (Borders::LEFT, "left"),
            (Borders::TOP | Borders::LEFT, "top|left"),
            (Borders::RIGHT | Borders::LEFT, "right|left"),
            (
                Borders::TOP | Borders::RIGHT | Borders::LEFT,
                "top|right|left",
            ),
            (Borders::BOTTOM | Borders::LEFT, "bottom|left"),
            (
                Borders::TOP | Borders::BOTTOM | Borders::LEFT,
                "top|bottom|left",
            ),
            (
                Borders::RIGHT | Borders::BOTTOM | Borders::LEFT,
                "right|bottom|left",
            ),
            (Borders::ALL, "all"),
        ];
        for (edges, expected) in combos {
            let kw = BorderSpec::edges_to_keyword(edges);
            assert_eq!(kw, expected, "bits {:#06b}", edges.bits());
            // The keyword must round-trip back to the same set.
            assert_eq!(BorderSpec::parse_edges(kw), Some(edges), "roundtrip {kw}");
        }
    }

    // -----------------------------------------------------------------
    // BoxEdgesValue / BorderStyleValue (var() wrappers)
    // -----------------------------------------------------------------

    #[test]
    fn box_edges_value_parse_var_no_fallback() {
        assert_eq!(
            BoxEdgesValue::parse("var(--pad)").unwrap(),
            BoxEdgesValue::Var {
                name: "pad".into(),
                fallback: None,
            }
        );
        // Case-insensitive VAR( prefix.
        assert_eq!(
            BoxEdgesValue::parse("VAR(--pad)").unwrap(),
            BoxEdgesValue::Var {
                name: "pad".into(),
                fallback: None,
            }
        );
    }

    #[test]
    fn box_edges_value_parse_concrete() {
        let e = BoxEdgesValue::parse("1 2").unwrap();
        match e {
            BoxEdgesValue::Edges(e) => {
                assert_eq!((e.top, e.right, e.bottom, e.left), (1, 2, 1, 2));
            }
            other => panic!("expected Edges, got {other:?}"),
        }
    }

    #[test]
    fn box_edges_value_parse_var_with_fallback() {
        let v = BoxEdgesValue::parse("var(--pad, 1)").unwrap();
        match v {
            BoxEdgesValue::Var {
                name,
                fallback: Some(fb),
            } => {
                assert_eq!(name, "pad");
                assert_eq!(*fb, BoxEdgesValue::Edges(BoxEdges::uniform(1)));
            }
            other => panic!("expected Var with fallback, got {other:?}"),
        }
        // 4-value fallback parses as a full BoxEdges.
        let v = BoxEdgesValue::parse("var(--pad, 1 2 3 4)").unwrap();
        match v {
            BoxEdgesValue::Var { fallback: Some(fb), .. } => match *fb {
                BoxEdgesValue::Edges(e) => {
                    assert_eq!((e.top, e.right, e.bottom, e.left), (1, 2, 3, 4));
                }
                other => panic!("expected Edges fallback, got {other:?}"),
            },
            other => panic!("expected Var, got {other:?}"),
        }
    }

    #[test]
    fn box_edges_value_empty_name_errors() {
        assert!(BoxEdgesValue::parse("var(--)").is_err());
    }

    #[test]
    fn box_edges_value_display_roundtrip() {
        // Concrete uniform → single number.
        let v = BoxEdgesValue::Edges(BoxEdges::uniform(3));
        assert_eq!(v.to_string(), "3");
        // Concrete multi → "1 2 3 4".
        let v = BoxEdgesValue::Edges(BoxEdges {
            top: 1,
            right: 2,
            bottom: 3,
            left: 4,
        });
        assert_eq!(v.to_string(), "1 2 3 4");
        // Var no fallback.
        let v = BoxEdgesValue::var("pad");
        assert_eq!(v.to_string(), "var(--pad)");
        // Var with fallback.
        let v = BoxEdgesValue::Var {
            name: "pad".into(),
            fallback: Some(Box::new(BoxEdgesValue::Edges(BoxEdges::uniform(1)))),
        };
        assert_eq!(v.to_string(), "var(--pad, 1)");
    }

    #[test]
    fn border_style_value_parse_var() {
        assert_eq!(
            BorderStyleValue::parse("var(--bs)").unwrap(),
            BorderStyleValue::Var {
                name: "bs".into(),
                fallback: None,
            }
        );
    }

    #[test]
    fn border_style_value_parse_keyword() {
        assert_eq!(
            BorderStyleValue::parse("rounded").unwrap(),
            BorderStyleValue::Fixed(BorderStyle::Rounded)
        );
        assert_eq!(
            BorderStyleValue::parse("none").unwrap(),
            BorderStyleValue::Fixed(BorderStyle::None)
        );
    }

    #[test]
    fn border_style_value_garbage_errors() {
        assert!(BorderStyleValue::parse("banana").is_err());
    }

    #[test]
    fn border_style_value_parse_var_with_fallback() {
        let v = BorderStyleValue::parse("var(--bs, rounded)").unwrap();
        match v {
            BorderStyleValue::Var {
                name,
                fallback: Some(fb),
            } => {
                assert_eq!(name, "bs");
                assert_eq!(*fb, BorderStyleValue::Fixed(BorderStyle::Rounded));
            }
            other => panic!("expected Var with fallback, got {other:?}"),
        }
    }

    #[test]
    fn border_style_value_display_roundtrip() {
        assert_eq!(
            BorderStyleValue::Fixed(BorderStyle::Rounded).to_string(),
            "rounded"
        );
        assert_eq!(BorderStyleValue::var("bs").to_string(), "var(--bs)");
    }

    #[test]
    fn border_shorthand_accepts_var_style_component() {
        // `border: var(--bs)` — the style component is a var.
        let spec = BorderSpec::parse_shorthand("var(--bs)").unwrap();
        assert_eq!(spec.style, BorderStyleValue::var("bs"));
        assert_eq!(spec.edges, Some(Borders::ALL));
        // `border: var(--bs) #f00` — var style + a literal color.
        let spec = BorderSpec::parse_shorthand("var(--bs) #f00").unwrap();
        assert_eq!(spec.style, BorderStyleValue::var("bs"));
        use ratatui::style::Color as RC;
        assert_eq!(spec.color, Some(Color::literal(RC::Rgb(0xff, 0, 0))));
    }

    #[test]
    fn border_shorthand_var_with_fallback_in_style() {
        // A var with a fallback that is itself a keyword: "var(--bs, rounded)".
        let spec = BorderSpec::parse_shorthand("var(--bs, rounded) #f00").unwrap();
        assert_eq!(
            spec.style,
            BorderStyleValue::Var {
                name: "bs".into(),
                fallback: Some(Box::new(BorderStyleValue::Fixed(BorderStyle::Rounded))),
            }
        );
    }

    #[cfg(feature = "serde")]
    #[test]
    fn box_edges_value_serde_roundtrip() {
        let v = BoxEdgesValue::Edges(BoxEdges::uniform(2));
        let json = serde_json::to_string(&v).unwrap();
        let back: BoxEdgesValue = serde_json::from_str(&json).unwrap();
        assert_eq!(back, v);
        // Var form.
        let v = BoxEdgesValue::var("pad");
        let json = serde_json::to_string(&v).unwrap();
        assert!(json.contains("var(--pad)"), "serialize var: {json}");
        let back: BoxEdgesValue = serde_json::from_str(&json).unwrap();
        assert_eq!(back, v);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn border_style_value_serde_roundtrip() {
        let v = BorderStyleValue::Fixed(BorderStyle::Rounded);
        let json = serde_json::to_string(&v).unwrap();
        let back: BorderStyleValue = serde_json::from_str(&json).unwrap();
        assert_eq!(back, v);
        let v = BorderStyleValue::var("bs");
        let json = serde_json::to_string(&v).unwrap();
        assert!(json.contains("var(--bs)"), "serialize var: {json}");
        let back: BorderStyleValue = serde_json::from_str(&json).unwrap();
        assert_eq!(back, v);
    }
}

// ---------------------------------------------------------------------------
// Optional serde
// ---------------------------------------------------------------------------

#[cfg(feature = "serde")]
mod serde_impl {
    use super::{
        length_to_css, BorderSpec, BorderStyle, BorderStyleValue, BoxEdges, BoxEdgesValue, Length,
    };
    use crate::color::Color;
    use ratatui::widgets::Borders;
    use serde::{
        de::{self, MapAccess, Visitor},
        Deserialize, Deserializer, Serialize, Serializer,
    };
    use std::fmt;

    // -------------------------------------------------------------------------
    // BoxEdges — a bare integer (uniform) OR a CSS shorthand string. Driven
    // via `deserialize_any` so the same impl works for JSON, TOML, and YAML
    // without ever materializing a `serde_json::Value`.
    // -------------------------------------------------------------------------

    impl<'de> Deserialize<'de> for BoxEdges {
        fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            struct BoxEdgesVisitor;

            impl<'de> Visitor<'de> for BoxEdgesVisitor {
                type Value = BoxEdges;

                fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                    f.write_str("a CSS box shorthand (number or string)")
                }

                fn visit_i64<E: de::Error>(self, v: i64) -> Result<BoxEdges, E> {
                    Ok(BoxEdges::uniform(v.max(0) as u16))
                }
                fn visit_u64<E: de::Error>(self, v: u64) -> Result<BoxEdges, E> {
                    Ok(BoxEdges::uniform(v as u16))
                }
                fn visit_f64<E: de::Error>(self, v: f64) -> Result<BoxEdges, E> {
                    Ok(BoxEdges::uniform(v.max(0.0) as u16))
                }
                fn visit_str<E: de::Error>(self, v: &str) -> Result<BoxEdges, E> {
                    BoxEdges::parse(v).map_err(E::custom)
                }
                fn visit_string<E: de::Error>(self, v: String) -> Result<BoxEdges, E> {
                    BoxEdges::parse(&v).map_err(E::custom)
                }
            }

            d.deserialize_any(BoxEdgesVisitor)
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

    // -------------------------------------------------------------------------
    // Length — same pattern as BoxEdges: number → Cells, string → parse.
    // -------------------------------------------------------------------------

    impl<'de> Deserialize<'de> for Length {
        fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            struct LengthVisitor;

            impl<'de> Visitor<'de> for LengthVisitor {
                type Value = Length;

                fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                    f.write_str("a CSS length (number or string)")
                }

                fn visit_i64<E: de::Error>(self, v: i64) -> Result<Length, E> {
                    Ok(Length::Cells(v.max(0) as u16))
                }
                fn visit_u64<E: de::Error>(self, v: u64) -> Result<Length, E> {
                    Ok(Length::Cells(v as u16))
                }
                fn visit_f64<E: de::Error>(self, v: f64) -> Result<Length, E> {
                    Ok(Length::Cells(v.max(0.0) as u16))
                }
                fn visit_str<E: de::Error>(self, v: &str) -> Result<Length, E> {
                    Length::parse(v).map_err(E::custom)
                }
                fn visit_string<E: de::Error>(self, v: String) -> Result<Length, E> {
                    Length::parse(&v).map_err(E::custom)
                }
            }

            d.deserialize_any(LengthVisitor)
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
                Length::Var {
                    name,
                    fallback: None,
                } => s.serialize_str(&format!("var(--{name})")),
                Length::Var {
                    name,
                    fallback: Some(fb),
                } => s.serialize_str(&format!("var(--{name}, {})", length_to_css(fb))),
            }
        }
    }

    // -------------------------------------------------------------------------
    // BorderStyle — a keyword string. Format-agnostic str visitor.
    // -------------------------------------------------------------------------

    impl<'de> Deserialize<'de> for BorderStyle {
        fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            struct BorderStyleVisitor;

            impl<'de> Visitor<'de> for BorderStyleVisitor {
                type Value = BorderStyle;

                fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                    f.write_str("a border style keyword")
                }

                fn visit_str<E: de::Error>(self, v: &str) -> Result<BorderStyle, E> {
                    BorderStyle::parse_keyword(v)
                        .ok_or_else(|| E::custom(format!("invalid border style: {v}")))
                }

                fn visit_string<E: de::Error>(self, v: String) -> Result<BorderStyle, E> {
                    BorderStyle::parse_keyword(&v)
                        .ok_or_else(|| E::custom(format!("invalid border style: {v}")))
                }
            }

            d.deserialize_str(BorderStyleVisitor)
        }
    }
    impl Serialize for BorderStyle {
        fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
            s.serialize_str(self.as_keyword())
        }
    }

    // -------------------------------------------------------------------------
    // BoxEdgesValue — a number/shorthand string (→ Edges) OR a `var(...)` /
    // `"1 2"` string. Mirrors BoxEdges' deserialize_any so an integer uniform
    // value stays an integer in JSON/TOML/YAML, while a multi-value or var
    // form is a string.
    // -------------------------------------------------------------------------

    impl<'de> Deserialize<'de> for BoxEdgesValue {
        fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            struct BoxEdgesValueVisitor;

            impl<'de> Visitor<'de> for BoxEdgesValueVisitor {
                type Value = BoxEdgesValue;

                fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                    f.write_str("a CSS box shorthand or var() string (number or string)")
                }

                fn visit_i64<E: de::Error>(self, v: i64) -> Result<BoxEdgesValue, E> {
                    Ok(BoxEdgesValue::Edges(BoxEdges::uniform(v.max(0) as u16)))
                }
                fn visit_u64<E: de::Error>(self, v: u64) -> Result<BoxEdgesValue, E> {
                    Ok(BoxEdgesValue::Edges(BoxEdges::uniform(v as u16)))
                }
                fn visit_f64<E: de::Error>(self, v: f64) -> Result<BoxEdgesValue, E> {
                    Ok(BoxEdgesValue::Edges(BoxEdges::uniform(v.max(0.0) as u16)))
                }
                fn visit_str<E: de::Error>(self, v: &str) -> Result<BoxEdgesValue, E> {
                    BoxEdgesValue::parse(v).map_err(E::custom)
                }
                fn visit_string<E: de::Error>(self, v: String) -> Result<BoxEdgesValue, E> {
                    BoxEdgesValue::parse(&v).map_err(E::custom)
                }
            }

            d.deserialize_any(BoxEdgesValueVisitor)
        }
    }
    impl Serialize for BoxEdgesValue {
        fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
            match self {
                BoxEdgesValue::Edges(e) => {
                    if e.top == e.right && e.right == e.bottom && e.bottom == e.left {
                        s.serialize_u64(e.top as u64)
                    } else {
                        s.serialize_str(&format!(
                            "{} {} {} {}",
                            e.top, e.right, e.bottom, e.left
                        ))
                    }
                }
                BoxEdgesValue::Var { .. } => s.serialize_str(&self.to_string()),
            }
        }
    }

    // -------------------------------------------------------------------------
    // BorderStyleValue — a keyword string OR a `var(...)` string.
    // -------------------------------------------------------------------------

    impl<'de> Deserialize<'de> for BorderStyleValue {
        fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            struct BorderStyleValueVisitor;

            impl<'de> Visitor<'de> for BorderStyleValueVisitor {
                type Value = BorderStyleValue;

                fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                    f.write_str("a border style keyword or var() string")
                }

                fn visit_str<E: de::Error>(self, v: &str) -> Result<BorderStyleValue, E> {
                    BorderStyleValue::parse(v).map_err(E::custom)
                }
                fn visit_string<E: de::Error>(self, v: String) -> Result<BorderStyleValue, E> {
                    BorderStyleValue::parse(&v).map_err(E::custom)
                }
            }

            d.deserialize_str(BorderStyleValueVisitor)
        }
    }
    impl Serialize for BorderStyleValue {
        fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
            s.serialize_str(&self.to_string())
        }
    }

    // -------------------------------------------------------------------------
    // EdgesInput — accepts either an edges keyword string ("top", "all",
    // "top|left", …) or a raw bit integer. Format-agnostic via deserialize_any.
    // Used by the BorderSpec map branch for the `edges` field.
    // -------------------------------------------------------------------------

    enum EdgesInput {
        None,
        Some(Borders),
    }

    impl<'de> Deserialize<'de> for EdgesInput {
        fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            struct EdgesVisitor;

            impl<'de> Visitor<'de> for EdgesVisitor {
                type Value = EdgesInput;

                fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                    f.write_str("an edges keyword string or a bit integer")
                }

                fn visit_unit<E: de::Error>(self) -> Result<EdgesInput, E> {
                    Ok(EdgesInput::None)
                }
                fn visit_none<E: de::Error>(self) -> Result<EdgesInput, E> {
                    Ok(EdgesInput::None)
                }
                fn visit_i64<E: de::Error>(self, v: i64) -> Result<EdgesInput, E> {
                    let bits = v as u8;
                    Ok(EdgesInput::Some(
                        Borders::from_bits(bits).unwrap_or(Borders::NONE),
                    ))
                }
                fn visit_u64<E: de::Error>(self, v: u64) -> Result<EdgesInput, E> {
                    let bits = v as u8;
                    Ok(EdgesInput::Some(
                        Borders::from_bits(bits).unwrap_or(Borders::NONE),
                    ))
                }
                fn visit_f64<E: de::Error>(self, v: f64) -> Result<EdgesInput, E> {
                    let bits = v as u8;
                    Ok(EdgesInput::Some(
                        Borders::from_bits(bits).unwrap_or(Borders::NONE),
                    ))
                }
                fn visit_str<E: de::Error>(self, v: &str) -> Result<EdgesInput, E> {
                    BorderSpec::parse_edges(v)
                        .map(EdgesInput::Some)
                        .ok_or_else(|| E::custom(format!("invalid edges: {v}")))
                }
                fn visit_string<E: de::Error>(self, v: String) -> Result<EdgesInput, E> {
                    BorderSpec::parse_edges(&v)
                        .map(EdgesInput::Some)
                        .ok_or_else(|| E::custom(format!("invalid edges: {v}")))
                }
            }

            d.deserialize_any(EdgesVisitor)
        }
    }

    // -------------------------------------------------------------------------
    // ColorInput — Color or null. The map branch reads `color` as this, so a
    // JSON null / YAML nil stays None without forcing Color to accept null.
    // -------------------------------------------------------------------------

    enum ColorInput {
        None,
        Some(Color),
    }

    impl<'de> Deserialize<'de> for ColorInput {
        fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            struct ColorInputVisitor;

            impl<'de> Visitor<'de> for ColorInputVisitor {
                type Value = ColorInput;

                fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                    f.write_str("a CSS color string or null")
                }

                fn visit_unit<E: de::Error>(self) -> Result<ColorInput, E> {
                    Ok(ColorInput::None)
                }
                fn visit_none<E: de::Error>(self) -> Result<ColorInput, E> {
                    Ok(ColorInput::None)
                }
                fn visit_str<E: de::Error>(self, v: &str) -> Result<ColorInput, E> {
                    Color::parse(v).map(ColorInput::Some).map_err(E::custom)
                }
                fn visit_string<E: de::Error>(self, v: String) -> Result<ColorInput, E> {
                    Color::parse(&v).map(ColorInput::Some).map_err(E::custom)
                }
            }

            d.deserialize_any(ColorInputVisitor)
        }
    }

    // -------------------------------------------------------------------------
    // BorderSpec — a shorthand string OR a `{style, color, edges}` map. The
    // map branch drives a MapAccess walk and deserializes each value directly
    // into its typed leaf — no serde_json::Value is materialized, so the same
    // code path serves JSON, TOML tables, and YAML mappings.
    // -------------------------------------------------------------------------

    impl<'de> Deserialize<'de> for BorderSpec {
        fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            struct BorderSpecVisitor;

            impl<'de> Visitor<'de> for BorderSpecVisitor {
                type Value = BorderSpec;

                fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                    f.write_str("a border shorthand string or a border object")
                }

                fn visit_str<E: de::Error>(self, v: &str) -> Result<BorderSpec, E> {
                    BorderSpec::parse_shorthand(v).map_err(E::custom)
                }

                fn visit_string<E: de::Error>(self, v: String) -> Result<BorderSpec, E> {
                    BorderSpec::parse_shorthand(&v).map_err(E::custom)
                }

                fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<BorderSpec, A::Error> {
                    let mut style: Option<BorderStyleValue> = None;
                    let mut color: Option<Color> = None;
                    let mut edges: Option<Borders> = None;
                    while let Some(key) = map.next_key::<String>()? {
                        match key.as_str() {
                            "style" => {
                                style = Some(map.next_value()?);
                            }
                            "color" => match map.next_value::<ColorInput>()? {
                                ColorInput::Some(c) => color = Some(c),
                                ColorInput::None => {}
                            },
                            "edges" => match map.next_value::<EdgesInput>()? {
                                EdgesInput::Some(e) => edges = Some(e),
                                EdgesInput::None => {}
                            },
                            // Unknown keys: forward-compat, read & discard the value.
                            _ => {
                                let _: de::IgnoredAny = map.next_value()?;
                            }
                        }
                    }
                    Ok(BorderSpec {
                        style: style.unwrap_or_default(),
                        color,
                        edges,
                    })
                }
            }

            d.deserialize_any(BorderSpecVisitor)
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
