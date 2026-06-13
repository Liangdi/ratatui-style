//! Error type for the CSS engine.
//!
//! Hand-rolled (no `thiserror`) to keep the dependency surface minimal for an
//! ecosystem crate.
//!
//! Every error carries an optional [`Loc`] (line:column, 1-based) pointing into
//! the source text that produced it. Errors constructed during stylesheet
//! parsing are tagged with a precise location; errors from value parsers that
//! are called outside a parse context (e.g. ad-hoc `Color::parse`) leave
//! `loc = None` and can be annotated with [`CssError::at`] by the caller.

use std::fmt;

/// A 1-based `line:column` position in a CSS source document.
///
/// `(0, 0)` (the default) represents an unknown location, e.g. for errors that
/// were constructed without parse context. Compare with
/// [`Loc::UNKNOWN`](Loc::UNKNOWN) or check [`Loc::is_unknown`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Loc {
    pub line: u32,
    pub column: u32,
}

impl Loc {
    /// Sentinel for an unknown location: `(0, 0)`.
    pub const UNKNOWN: Loc = Loc { line: 0, column: 0 };

    /// Construct a known location.
    pub const fn new(line: u32, column: u32) -> Self {
        Self { line, column }
    }

    /// `true` when this is the [`UNKNOWN`](Loc::UNKNOWN) sentinel.
    pub fn is_unknown(self) -> bool {
        self.line == 0 && self.column == 0
    }
}

impl fmt::Display for Loc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.line, self.column)
    }
}

/// The kind of [`CssError`], independent of where it occurred.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CssErrorKind {
    /// A color value could not be parsed (e.g. `"#zzz"`).
    InvalidColor(String),
    /// A selector could not be parsed (e.g. `"..::"`).
    InvalidSelector(String),
    /// A length/sizing value could not be parsed (e.g. `"width: banana"`).
    InvalidLength(String),
    /// A `var(--name)` referenced a variable that is not defined in any token table.
    UndefinedVariable(String),
    /// A `var()` reference chain is too deep or cyclic.
    CircularVariable(String),
    /// (Strict mode only.) A declaration used a property the engine does not know.
    UnknownProperty(String),
    /// An I/O error occurred (e.g. reading a stylesheet from disk).
    Io(String),
}

impl fmt::Display for CssErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidColor(v) => write!(f, "invalid color: {v}"),
            Self::InvalidSelector(v) => write!(f, "invalid selector: {v}"),
            Self::InvalidLength(v) => write!(f, "invalid length: {v}"),
            Self::UndefinedVariable(v) => write!(f, "undefined variable: {v}"),
            Self::CircularVariable(v) => write!(f, "circular variable reference: {v}"),
            Self::UnknownProperty(v) => write!(f, "unknown property: {v}"),
            Self::Io(v) => write!(f, "io error: {v}"),
        }
    }
}

/// All errors produced by `ratatui-style`.
///
/// A `CssError` is a [`kind`](CssError::kind) plus an optional [`Loc`]. Use the
/// `invalid_color` / `unknown_property` / … constructors for a `loc = None`
/// error, then chain `.at(line, col)` (or `.with_loc(loc)`) to attach a
/// position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CssError {
    pub kind: CssErrorKind,
    pub loc: Option<Loc>,
}

impl CssError {
    // --- constructors (loc = None) ----------------------------------------

    /// Construct from an explicit kind with no location.
    pub fn from_kind(kind: CssErrorKind) -> Self {
        Self { kind, loc: None }
    }

    pub fn invalid_color(msg: impl Into<String>) -> Self {
        Self::from_kind(CssErrorKind::InvalidColor(msg.into()))
    }
    pub fn invalid_selector(msg: impl Into<String>) -> Self {
        Self::from_kind(CssErrorKind::InvalidSelector(msg.into()))
    }
    pub fn invalid_length(msg: impl Into<String>) -> Self {
        Self::from_kind(CssErrorKind::InvalidLength(msg.into()))
    }
    pub fn undefined_variable(msg: impl Into<String>) -> Self {
        Self::from_kind(CssErrorKind::UndefinedVariable(msg.into()))
    }
    pub fn circular_variable(msg: impl Into<String>) -> Self {
        Self::from_kind(CssErrorKind::CircularVariable(msg.into()))
    }
    /// (Strict mode only.) An unknown CSS property was used.
    pub fn unknown_property(msg: impl Into<String>) -> Self {
        Self::from_kind(CssErrorKind::UnknownProperty(msg.into()))
    }
    pub fn io(msg: impl Into<String>) -> Self {
        Self::from_kind(CssErrorKind::Io(msg.into()))
    }

    // --- location builder -------------------------------------------------

    /// Attach a 1-based `line:column` to this error, returning it for chaining.
    pub fn at(mut self, line: u32, column: u32) -> Self {
        self.loc = Some(Loc::new(line, column));
        self
    }

    /// Attach an explicit [`Loc`].
    pub fn with_loc(mut self, loc: Loc) -> Self {
        self.loc = Some(loc);
        self
    }

    /// A reference to the [`CssErrorKind`] this wraps.
    pub fn kind(&self) -> &CssErrorKind {
        &self.kind
    }
}

impl fmt::Display for CssError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.loc {
            Some(loc) if !loc.is_unknown() => write!(f, "{} at line {}:{}", self.kind, loc.line, loc.column),
            _ => write!(f, "{}", self.kind),
        }
    }
}

impl std::error::Error for CssError {}

pub type Result<T> = std::result::Result<T, CssError>;
