//! # ratatui-style
//!
//! A CSS cascade engine for ratatui — selectors, specificity, inheritance,
//! pseudo-states, and data-driven styling. Produces native ratatui `Style` /
//! `Block` / `Constraint` values; it is never a parallel rendering stack.
//!
//! See [`design.md`](https://github.com/Liangdi/a2ui/blob/master/crates/ratatui-style/design.md)
//! for the full RFC.
//!
//! # Quick start (L1 — stylesheet + class)
//!
//! ```
//! use ratatui_style::{CssStyle, Origin, OwnedNode, Stylesheet};
//!
//! let mut sheet = Stylesheet::new();
//! sheet.add(
//!     "Button.primary",
//!     CssStyle::new().color("#fff").background("blue").bold(),
//!     Origin::User,
//! )
//! .unwrap();
//!
//! let node = OwnedNode::new("Button").with_classes(["primary"]);
//! let computed = sheet.compute(&node, None);
//! let _ratatui_style = computed.to_style();
//! ```

pub mod box_model;
pub mod cascade;
pub mod color;
pub mod css_macro;
pub mod error;
pub mod node;
pub mod runtime;
pub mod selector;
pub mod style;
pub mod stylesheet;
pub mod token;

#[cfg(feature = "themekit")]
pub mod themekit;

// The `scss!` macro calls into `grass`; re-export it at the crate root so the
// macro resolves in downstream crates without them depending on `grass`.
#[cfg(feature = "scss")]
pub mod scss_macro;

#[cfg(feature = "scss")]
pub use grass;

// Re-exports — the primary public surface.
pub use box_model::{BorderSpec, BorderStyle, BoxEdges, IntoBorderSpec, IntoBoxEdges, Length};
pub use cascade::{render_computed, CascadeContext, ComputedStyle, ComputeScratch};
pub use color::Color;
pub use error::{CssError, CssErrorKind, Loc, Result};
pub use node::{Classes, NodeRef, OwnedNode, Position, State, StyledNode};
pub use runtime::RuntimeStyle;
pub use selector::{PseudoClass, Selector};
pub use style::{Align, CssStyle, FontStyle, TextDecoration, Weight};
pub use stylesheet::{apply_decl, Origin, RuleEntry, Stylesheet};
pub use token::ThemeTokens;

/// Convenience re-exports — `use ratatui_style::prelude::*;` to pull in the
/// common public surface in one line.
///
/// This is purely an ergonomic entry point: every item here is also re-exported
/// at the crate root, so the prelude adds nothing new — it just gathers the
/// types/traits/functions/macros a downstream app most often needs into one
/// glob-importable list.
///
/// The `css!` macro is `#[macro_export]`-ed at the crate root and (as a
/// `macro_rules!`) cannot be reliably re-exported through a module glob, so it
/// is **not** included here. Import it directly:
///
/// ```rust,ignore
/// use ratatui_style::css;
/// ```
pub mod prelude {
    pub use crate::box_model::{BorderSpec, BorderStyle, BoxEdges, IntoBorderSpec, IntoBoxEdges, Length};
    pub use crate::cascade::{render_computed, CascadeContext, ComputedStyle, ComputeScratch};
    pub use crate::color::Color;
    pub use crate::error::{CssError, CssErrorKind, Loc, Result};
    pub use crate::node::{Classes, NodeRef, OwnedNode, Position, State, StyledNode};
    pub use crate::runtime::RuntimeStyle;
    pub use crate::selector::{PseudoClass, Selector};
    pub use crate::style::{Align, CssStyle, FontStyle, TextDecoration, Weight};
    pub use crate::stylesheet::{Origin, RuleEntry, Stylesheet};
    pub use crate::token::ThemeTokens;
}
