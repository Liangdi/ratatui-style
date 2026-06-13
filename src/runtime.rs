//! Runtime-overridable stylesheets.
//!
//! [`RuntimeStyle`] layers a compile-time embedded stylesheet (tagged
//! [`Origin::Theme`]) with an optional CSS file loaded from the filesystem at
//! runtime (tagged [`Origin::User`]). Because `Theme < User` in the cascade
//! ordering, runtime rules override embedded rules at equal specificity — no
//! special merge logic required.

use std::path::Path;

use crate::cascade::ComputedStyle;
use crate::error::{CssError, Result};
use crate::node::StyledNode;
use crate::stylesheet::{Origin, Stylesheet};

/// A stylesheet layered from a compile-time base plus an optional runtime
/// override.
///
/// Construct with [`RuntimeStyle::new`] around an embedded stylesheet produced
/// by the [`css!`](crate::css) macro, then optionally call
/// [`RuntimeStyle::load_override`] to apply a user-supplied CSS file. The merged
/// sheet is computed once when the override is (re)loaded, so
/// [`RuntimeStyle::compute`] stays allocation-free.
pub struct RuntimeStyle {
    /// The compile-time embedded stylesheet (Origin::Theme).
    embedded: &'static Stylesheet,
    /// The runtime override (Origin::User), if one is loaded.
    runtime: Option<Stylesheet>,
    /// The always-ready merged sheet: `embedded` cloned, optionally extended
    /// with `runtime`. Owned so that [`Self::compute`] is zero-copy.
    sheet: Stylesheet,
}

impl RuntimeStyle {
    /// Wrap an embedded stylesheet with no runtime override.
    pub fn new(embedded: &'static Stylesheet) -> Self {
        Self {
            embedded,
            runtime: None,
            sheet: embedded.clone(),
        }
    }

    /// Load (or reload) a runtime CSS override from `path`.
    ///
    /// If the file exists it is parsed and merged onto the embedded stylesheet;
    /// its rules carry [`Origin::User`] and override the embedded
    /// [`Origin::Theme`] rules at equal specificity. If the file does **not**
    /// exist, this is not an error — the embedded stylesheet is used as-is and
    /// any previously loaded override is cleared. Other I/O or parse failures
    /// are returned as [`CssError`].
    pub fn load_override(&mut self, path: &Path) -> Result<()> {
        match std::fs::read_to_string(path) {
            Ok(css) => {
                let runtime = Stylesheet::parse_with_origin(&css, Origin::User)?;
                // Rebuild the merged sheet from a clean clone of the embedded
                // base, then layer the runtime override on top.
                let mut sheet = self.embedded.clone();
                sheet.extend(&runtime);
                self.runtime = Some(runtime);
                self.sheet = sheet;
                Ok(())
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                self.runtime = None;
                self.sheet = self.embedded.clone();
                Ok(())
            }
            Err(e) => Err(CssError::Io(format!(
                "cannot read runtime CSS {}: {e}",
                path.display()
            ))),
        }
    }

    /// Compute the resolved style for `node`, optionally inheriting from
    /// `parent`. Delegates to the pre-merged sheet, so this is allocation-free.
    pub fn compute(&self, node: &dyn StyledNode, parent: Option<&ComputedStyle>) -> ComputedStyle {
        self.sheet.compute(node, parent)
    }

    /// The embedded (compile-time) stylesheet.
    pub fn embedded(&self) -> &Stylesheet {
        self.embedded
    }

    /// The runtime override stylesheet, if one is loaded.
    pub fn runtime(&self) -> Option<&Stylesheet> {
        self.runtime.as_ref()
    }

    /// Whether a runtime override is currently active.
    pub fn has_override(&self) -> bool {
        self.runtime.is_some()
    }
}
