//! Compile-time SCSS embedding via the [`scss!`](crate::scss) macro
//! (requires the `scss` feature).
//!
//! The `.scss` source is `include_str!`-ed into the binary at build time;
//! [`grass`] compiles it to CSS on first access (lazily, cached in a
//! `LazyLock`). The resulting rules carry
//! [`Origin::Theme`](crate::Origin::Theme) — the same override semantics as
//! [`css!`](crate::css).
//!
//! `grass` is re-exported as [`crate::grass`], so the `scss!` macro resolves in
//! downstream crates without them adding `grass` as a direct dependency.
//!
//! ```rust,ignore
//! use std::sync::LazyLock;
//! use ratatui_style::{OwnedNode, scss, Stylesheet};
//!
//! static THEME: LazyLock<Stylesheet> = scss!("theme.scss");
//!
//! fn main() {
//!     let node = OwnedNode::new("Button").with_classes(["primary"]);
//!     let computed = THEME.compute(&node, None);   // auto-deref through LazyLock
//! }
//! ```

/// Embed a `.scss` file at compile time and lazily compile it to a
/// [`std::sync::LazyLock<Stylesheet>`](crate::Stylesheet).
///
/// Requires the `scss` feature. The path is resolved relative to the source
/// file where the macro is invoked (via `include_str!`). The SCSS source is
/// embedded at build time; `grass` compiles it to CSS on first access. Rules
/// are tagged [`Origin::Theme`](crate::Origin::Theme), making them overridable
/// by user-level rules at runtime (see [`RuntimeStyle`](crate::RuntimeStyle)).
///
/// Bind the result to a `static`, just like [`css!`](crate::css).
///
/// # Panics
///
/// Panics on first access if the SCSS fails to compile or the resulting CSS
/// fails to parse. Since the source is fixed at compile time, this indicates a
/// bug in the `.scss` file.
#[macro_export]
macro_rules! scss {
    ($path:literal) => {
        std::sync::LazyLock::new(|| {
            let css = $crate::grass::from_string(
                include_str!($path).to_owned(),
                &$crate::grass::Options::default(),
            )
            .expect(concat!("SCSS compile failed: ", $path));
            $crate::Stylesheet::parse_with_origin(&css, $crate::Origin::Theme)
                .expect(concat!("embedded CSS parse failed: ", $path))
        })
    };
}
