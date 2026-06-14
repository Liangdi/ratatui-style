//! Framework-agnostic element view: the `StyledNode` trait and supporting
//! types. The cascade engine knows nothing about a2ui, ratatui widgets, or any
//! particular framework — it only knows a [`StyledNode`].

/// A set of pseudo-class flags for one element.
///
/// Maps directly to CSS pseudo-classes: `:focus`, `:hover`, `:disabled`,
/// `:checked`, `:active`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct State {
    pub focus: bool,
    pub hover: bool,
    pub disabled: bool,
    pub checked: bool,
    pub active: bool,
}

impl State {
    pub const fn empty() -> Self {
        Self {
            focus: false,
            hover: false,
            disabled: false,
            checked: false,
            active: false,
        }
    }
    pub const fn focus() -> Self {
        Self {
            focus: true,
            ..Self::empty()
        }
    }
    pub const fn disabled() -> Self {
        Self {
            disabled: true,
            ..Self::empty()
        }
    }
}

/// Where an element sits among its siblings. Used by future `:nth-child`
/// matching (P3); returned by [`StyledNode::position`] for forward-compat.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Position {
    /// 0-based index among siblings.
    pub index: usize,
    /// Total number of siblings (including self).
    pub sibling_count: usize,
    /// The parent element's type name, if any.
    pub parent_type: Option<String>,
}

impl Position {
    pub fn new(index: usize, sibling_count: usize) -> Self {
        Self {
            index,
            sibling_count,
            parent_type: None,
        }
    }
}

/// A borrowable view over an element's class list.
///
/// Two representations back the same query surface:
/// - [`Classes::from_slice`] borrows a `&'a [&'a str]` directly — used by
///   [`NodeRef`], it is **zero-allocation** (a compile-time guarantee when the
///   source is `&'static str`).
/// - [`Classes::from_vec`] owns a `Vec<&'a str>` — used by [`OwnedNode`], it
///   costs one `Vec` allocation per call (acceptable; the hot path uses
///   `NodeRef`).
///
/// Use [`as_slice`](Self::as_slice) for iteration (`&[&str]`) rather than
/// `iter()` so both representations unify behind a single concrete return
/// type.
pub struct Classes<'a> {
    repr: Repr<'a>,
}

enum Repr<'a> {
    Slice(&'a [&'a str]),
    Owned(Vec<&'a str>),
}

impl<'a> Classes<'a> {
    /// Zero-allocation view over a borrowed slice. The `NodeRef` path uses
    /// this — when `slice` is `&'static [&'static str]` no heap allocation
    /// occurs at any point.
    pub fn from_slice(slice: &'a [&'a str]) -> Self {
        Self {
            repr: Repr::Slice(slice),
        }
    }

    /// Owning view built from an existing `Vec<&str>`. Used by [`OwnedNode`]
    /// which stores `String`s and must materialize `&str` borrows per call.
    pub fn from_vec(v: Vec<&'a str>) -> Self {
        Self {
            repr: Repr::Owned(v),
        }
    }

    /// Unified read-only access to the underlying class names, regardless of
    /// representation. Prefer this over an `iter()` — both reprs return the
    /// same concrete `&[&str]`.
    pub fn as_slice(&self) -> &[&'a str] {
        match &self.repr {
            Repr::Slice(s) => s,
            Repr::Owned(v) => v,
        }
    }

    /// Whether `name` is present (case-sensitive).
    pub fn contains(&self, name: &str) -> bool {
        self.as_slice().contains(&name)
    }

    pub fn is_empty(&self) -> bool {
        self.as_slice().is_empty()
    }

    pub fn len(&self) -> usize {
        self.as_slice().len()
    }
}

/// The minimal contract the cascade needs to match selectors against an
/// element.
///
/// Implement this on your framework's node type (e.g. a2ui's `ComponentModel`,
/// or a plain app-state struct in a vanilla ratatui app).
///
/// For the draw-loop hot path prefer [`NodeRef`] (zero-allocation). [`OwnedNode`]
/// remains available for convenience where owned `String`/`Vec<String>` storage
/// is preferable.
pub trait StyledNode {
    /// Element type name — matches a CSS type selector (e.g. `"Button"`).
    fn type_name(&self) -> &str;

    /// Element id — matches a CSS `#id` selector.
    fn id(&self) -> Option<&str>;

    /// Class names — match CSS `.class` selectors.
    ///
    /// Returns a [`Classes<'_>`] borrow view rather than an allocating
    /// `Vec<&str>`. [`NodeRef`] makes this zero-allocation; [`OwnedNode`]
    /// pays one `Vec` allocation (it is not the hot path). The cascade hoists
    /// this call out of the per-rule loop so the cost is paid at most once per
    /// node regardless.
    fn classes(&self) -> Classes<'_>;

    /// Pseudo-class state — matches `:focus` / `:disabled` / etc.
    fn state(&self) -> State;

    /// Sibling position — for future `:nth-child` support.
    ///
    /// This is **optional**: `:nth-child` matching is P3 and not yet wired into
    /// the cascade, so `compute` does not consult it. The default returns an
    /// empty [`Position`]. Override it only when you need `:nth-child` data at
    /// some future point — until then, leaving the default avoids forcing every
    /// node type to materialize sibling info.
    fn position(&self) -> Position {
        Position::default()
    }
}

/// A convenience node that owns its data (`String` / `Vec<String>`).
///
/// Handy for tests, one-off queries, and places where you want to build a node
/// from runtime-owned strings. It is **not** allocation-free: each
/// [`OwnedNode::new`] allocates a `String`, and [`StyledNode::classes`]
/// allocates one `Vec` per call. For the per-frame draw loop, use [`NodeRef`].
#[derive(Debug, Clone)]
pub struct OwnedNode {
    pub type_name: String,
    pub id: Option<String>,
    pub classes: Vec<String>,
    pub state: State,
    pub position: Position,
}

impl OwnedNode {
    pub fn new(type_name: impl Into<String>) -> Self {
        Self {
            type_name: type_name.into(),
            id: None,
            classes: Vec::new(),
            state: State::empty(),
            position: Position::default(),
        }
    }
    pub fn with_id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }
    pub fn with_classes(mut self, classes: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.classes = classes.into_iter().map(Into::into).collect();
        self
    }
    pub fn with_state(mut self, state: State) -> Self {
        self.state = state;
        self
    }
    /// Set the sibling position. Mirrors [`NodeRef::position`].
    pub fn with_position(mut self, position: Position) -> Self {
        self.position = position;
        self
    }
}

impl StyledNode for OwnedNode {
    fn type_name(&self) -> &str {
        &self.type_name
    }
    fn id(&self) -> Option<&str> {
        self.id.as_deref()
    }
    fn classes(&self) -> Classes<'_> {
        // One Vec allocation per call — acceptable for OwnedNode (not the hot
        // path). The draw loop uses NodeRef to avoid even this.
        Classes::from_vec(self.classes.iter().map(String::as_str).collect())
    }
    fn state(&self) -> State {
        self.state
    }
    fn position(&self) -> Position {
        self.position.clone()
    }
}

/// A borrowed node: `type_name`, `id`, and `classes` are all `&'a str` /
/// `&'a [&'a str]` borrows, so construction is **zero-allocation** (a
/// compile-time guarantee when the source data is `&'static`).
///
/// Use this in the draw loop:
///
/// ```rust,ignore
/// let node = NodeRef::new("Button").classes(&["primary"]).state(State::focus());
/// let computed = sheet.compute(&node, None);
/// ```
///
/// Builder methods mirror [`OwnedNode`]'s (`with_*`) for easy migration, plus
/// short `classes`/`state` setters.
pub struct NodeRef<'a> {
    type_name: &'a str,
    id: Option<&'a str>,
    classes: &'a [&'a str],
    state: State,
    position: Position,
}

impl<'a> NodeRef<'a> {
    /// Borrow `type_name`. Zero-allocation.
    pub fn new(type_name: &'a str) -> Self {
        Self {
            type_name,
            id: None,
            classes: &[],
            state: State::empty(),
            position: Position::default(),
        }
    }

    /// Set the id (borrowed). Zero-allocation.
    pub fn id(mut self, id: &'a str) -> Self {
        self.id = Some(id);
        self
    }
    /// Alias for [`id`](Self::id), matching [`OwnedNode::with_id`].
    pub fn with_id(self, id: &'a str) -> Self {
        self.id(id)
    }

    /// Set the class list (borrowed slice). Zero-allocation.
    pub fn classes(mut self, classes: &'a [&'a str]) -> Self {
        self.classes = classes;
        self
    }
    /// Alias for [`classes`](Self::classes), matching [`OwnedNode::with_classes`].
    pub fn with_classes(self, classes: &'a [&'a str]) -> Self {
        self.classes(classes)
    }

    /// Set the pseudo-class state. Zero-allocation.
    pub fn state(mut self, state: State) -> Self {
        self.state = state;
        self
    }
    /// Alias for [`state`](Self::state), matching [`OwnedNode::with_state`].
    pub fn with_state(self, state: State) -> Self {
        self.state(state)
    }

    /// Set the sibling position. Rarely needed in the draw loop.
    pub fn position(mut self, position: Position) -> Self {
        self.position = position;
        self
    }
}

impl<'a> StyledNode for NodeRef<'a> {
    fn type_name(&self) -> &str {
        self.type_name
    }
    fn id(&self) -> Option<&str> {
        self.id
    }
    fn classes(&self) -> Classes<'_> {
        // Zero-allocation: borrow the slice directly.
        Classes::from_slice(self.classes)
    }
    fn state(&self) -> State {
        self.state
    }
    fn position(&self) -> Position {
        self.position.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classes_slice_contains() {
        let c = Classes::from_slice(&["a", "b"]);
        assert!(c.contains("b"));
        assert!(!c.contains("c"));
        assert_eq!(c.len(), 2);
        assert!(!c.is_empty());
        assert_eq!(c.as_slice(), &["a", "b"]);
    }

    #[test]
    fn classes_owned_contains() {
        let c = Classes::from_vec(vec!["x", "y"]);
        assert!(c.contains("x"));
        assert!(!c.contains("z"));
        assert_eq!(c.len(), 2);
    }

    #[test]
    fn classes_empty() {
        let c = Classes::from_slice(&[]);
        assert!(c.is_empty());
        assert_eq!(c.len(), 0);
    }
}
