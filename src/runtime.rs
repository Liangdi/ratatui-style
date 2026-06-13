//! Runtime-overridable stylesheets.
//!
//! [`RuntimeStyle`] layers a base stylesheet (tagged [`Origin::Theme`]) with an
//! optional CSS file loaded from the filesystem at runtime (tagged
//! [`Origin::User`]). Because `Theme < User` in the cascade ordering, runtime
//! rules override base rules at equal specificity — no special merge logic
//! required.
//!
//! The base can come from two sources:
//! - a **compile-time `&'static`** stylesheet produced by the
//!   [`css!`](crate::css) macro ([`RuntimeStyle::new`]); or
//! - an **owned, runtime-parsed** stylesheet wrapped in an `Arc`
//!   ([`RuntimeStyle::from_owned`]), so purely runtime-driven theme loading
//!   never needs to `Box::leak`.
//!
//! For live theming, [`RuntimeStyle::reload_if_changed`] watches a CSS file's
//! mtime and re-parses it only when it changes — cheap to call every app tick.

use std::path::Path;
use std::sync::Arc;

use crate::cascade::{ComputedStyle, ComputeScratch};
use crate::error::{CssError, Result};
use crate::node::StyledNode;
use crate::stylesheet::{Origin, Stylesheet};

/// Where the base (non-overridden) stylesheet of a [`RuntimeStyle`] comes from.
///
/// [`Static`](Self::Static) is the zero-cost path used by the
/// [`css!`](crate::css) macro (a `&'static Stylesheet`). [`Owned`](Self::Owned)
/// lets callers supply a runtime-parsed stylesheet via an `Arc`, so themes loaded
/// from disk/config never need to leak memory.
enum Base {
    /// A compile-time embedded stylesheet (e.g. produced by the `css!` macro).
    Static(&'static Stylesheet),
    /// A runtime-parsed, refcounted stylesheet.
    Owned(Arc<Stylesheet>),
}

/// A stylesheet layered from a base plus an optional runtime override.
///
/// Construct the base via either:
/// - [`RuntimeStyle::new`] — wrap a compile-time `&'static Stylesheet`
///   (typically from the [`css!`](crate::css) macro), or
/// - [`RuntimeStyle::from_owned`] — wrap a runtime-parsed `Arc<Stylesheet>`
///   (e.g. `RuntimeStyle::from_owned(Arc::new(Stylesheet::parse(&css)?))`),
///   which avoids leaking memory for themes loaded purely at runtime.
///
/// Then optionally call [`RuntimeStyle::load_override`] (one-shot) or
/// [`RuntimeStyle::reload_if_changed`] (mtime-based, tick-friendly) to apply a
/// user-supplied CSS file. The merged sheet is recomputed only when the override
/// changes, so [`RuntimeStyle::compute`] stays allocation-free.
pub struct RuntimeStyle {
    /// The base stylesheet (Origin::Theme), either static or owned.
    base: Base,
    /// The runtime override (Origin::User), if one is loaded.
    runtime: Option<Stylesheet>,
    /// The always-ready merged sheet: base cloned, optionally extended with
    /// `runtime`. Owned so that [`Self::compute`] is zero-copy.
    sheet: Stylesheet,
    /// The mtime recorded for the override `path` the last time it was loaded.
    /// Used by [`Self::reload_if_changed`] to skip unchanged files.
    last_mtime: Option<std::time::SystemTime>,
}

impl RuntimeStyle {
    /// Wrap a compile-time `&'static` embedded stylesheet with no runtime
    /// override. This is the path used by the [`css!`](crate::css) macro and is
    /// zero-cost (no allocation, no refcount).
    pub fn new(embedded: &'static Stylesheet) -> Self {
        Self {
            base: Base::Static(embedded),
            runtime: None,
            sheet: embedded.clone(),
            last_mtime: None,
        }
    }

    /// Wrap a runtime-parsed, owned stylesheet with no runtime override.
    ///
    /// For apps that load their theme purely at runtime (from disk, config,
    /// network, …) there is no compile-time `&'static` to borrow. This
    /// constructor takes an `Arc<Stylesheet>` so the caller never needs to
    /// `Box::leak`:
    ///
    /// ```no_run
    /// # use std::sync::Arc;
    /// # use ratatui_style::{RuntimeStyle, Stylesheet};
    /// let css = "Button { color: red; }";
    /// let style = RuntimeStyle::from_owned(Arc::new(Stylesheet::parse(css).unwrap()));
    /// ```
    pub fn from_owned(embedded: Arc<Stylesheet>) -> Self {
        // Initialize the merged sheet from a clone of the base.
        let sheet = embedded.as_ref().clone();
        Self {
            base: Base::Owned(embedded),
            runtime: None,
            last_mtime: None,
            sheet,
        }
    }

    /// Returns the base [`Stylesheet`], regardless of whether it is static or
    /// owned.
    fn base(&self) -> &Stylesheet {
        match &self.base {
            Base::Static(s) => s,
            Base::Owned(s) => s,
        }
    }

    /// Load (or reload) a runtime CSS override from `path`.
    ///
    /// If the file exists it is parsed and merged onto the base stylesheet;
    /// its rules carry [`Origin::User`] and override the base [`Origin::Theme`]
    /// rules at equal specificity. If the file does **not** exist, this is not
    /// an error — the base stylesheet is used as-is and any previously loaded
    /// override is cleared. Other I/O or parse failures are returned as
    /// [`CssError`].
    ///
    /// This performs a full re-read and re-parse every call. For cheap,
    /// mtime-gated reloading in an app tick, see [`Self::reload_if_changed`].
    pub fn load_override(&mut self, path: &Path) -> Result<()> {
        match std::fs::read_to_string(path) {
            Ok(css) => {
                let runtime = Stylesheet::parse_with_origin(&css, Origin::User)?;
                // Rebuild the merged sheet from a clean clone of the base,
                // then layer the runtime override on top.
                let mut sheet = self.base().clone();
                sheet.extend(&runtime);
                self.runtime = Some(runtime);
                self.sheet = sheet;
                // Record the mtime so reload_if_changed can detect later edits.
                self.last_mtime = current_mtime(path);
                Ok(())
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                self.runtime = None;
                self.sheet = self.base().clone();
                self.last_mtime = None;
                Ok(())
            }
            Err(e) => Err(CssError::io(format!(
                "cannot read runtime CSS {}: {e}",
                path.display()
            ))),
        }
    }

    /// Reload the override at `path` only if its mtime changed since the last
    /// load; otherwise do nothing.
    ///
    /// Returns `true` when a reload actually happened (the file changed and was
    /// re-parsed), or when an existing override was cleared because the file
    /// disappeared (mirroring [`Self::load_override`]'s `NotFound` semantics).
    /// Returns `false` when nothing changed.
    ///
    /// Call this from an app's event-loop tick to get "edit the theme file →
    /// see it live" behavior without re-parsing every frame:
    ///
    /// ```no_run
    /// # use std::path::Path;
    /// # use std::sync::Arc;
    /// # use ratatui_style::{RuntimeStyle, Stylesheet};
    /// # let base = Arc::new(Stylesheet::parse("Root { color: red; }").unwrap());
    /// # let mut style = RuntimeStyle::from_owned(base);
    /// # let path = Path::new("/tmp/theme.css");
    /// // in your tick / poll loop:
    /// if style.reload_if_changed(path).unwrap() {
    ///     // theme was updated — the next compute() reflects the new rules
    /// }
    /// ```
    ///
    /// **Degradation policy:** if the filesystem cannot report a modification
    /// time for `path` (e.g. some network/FUSE mounts), this is treated as a
    /// change — the file is reloaded and `true` is returned — so updates are
    /// never silently dropped. `NotFound` still means "override removed".
    pub fn reload_if_changed(&mut self, path: &Path) -> Result<bool> {
        match std::fs::metadata(path) {
            // File exists: compare mtime, reload only if changed.
            Ok(meta) => {
                let mtime = meta.modified();
                match (mtime, self.last_mtime) {
                    (Ok(m), Some(prev)) if m == prev => {
                        // Unchanged — nothing to do.
                        Ok(false)
                    }
                    // Different, unknown, or first load → reload. (Unknown mtime
                    // degrades to "always reload" so we never miss an update.)
                    _ => {
                        self.load_override(path)?;
                        Ok(true)
                    }
                }
            }
            // File gone: clear override iff we had one (matches load_override).
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                if self.has_override() {
                    self.load_override(path)?;
                    Ok(true)
                } else {
                    Ok(false)
                }
            }
            Err(e) => Err(CssError::io(format!(
                "cannot stat runtime CSS {}: {e}",
                path.display()
            ))),
        }
    }

    /// Compute the resolved style for `node`, optionally inheriting from
    /// `parent`. Delegates to the pre-merged sheet, so this is allocation-free.
    pub fn compute(&self, node: &dyn StyledNode, parent: Option<&ComputedStyle>) -> ComputedStyle {
        self.sheet.compute(node, parent)
    }

    /// Compute using a caller-provided [`ComputeScratch`], reused across calls.
    ///
    /// Delegates to [`Stylesheet::compute_with`] on the pre-merged sheet. Use
    /// this in the draw loop alongside [`NodeRef`](crate::node::NodeRef) for a
    /// fully allocation-free per-frame path.
    pub fn compute_with(
        &self,
        node: &dyn StyledNode,
        parent: Option<&ComputedStyle>,
        scratch: &mut ComputeScratch,
    ) -> ComputedStyle {
        self.sheet.compute_with(node, parent, scratch)
    }

    /// The base (compile-time or owned) stylesheet.
    pub fn embedded(&self) -> &Stylesheet {
        self.base()
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

/// Read the modification time of `path`, returning `None` if unavailable.
fn current_mtime(path: &Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(path).ok().and_then(|m| m.modified().ok())
}

// Compile-time proof that RuntimeStyle stays Send + Sync: the `Arc<Stylesheet>`
// base is Send+Sync (Stylesheet is Send+Sync), and the other fields are too.
const _: () = {
    const fn _assert_send_sync<T: Send + Sync>() {}
    const _PROOF: () = _assert_send_sync::<RuntimeStyle>();
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::NodeRef;
    use std::sync::Arc;
    use std::thread;
    use std::time::Duration;

    /// A unique temp CSS path for one test file.
    fn temp_css(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rss-{}-{}.css",
            std::process::id(),
            name
        ))
    }

    #[test]
    fn owned_base_works() {
        let base = Arc::new(Stylesheet::parse("Button { color: red; }").unwrap());
        let style = RuntimeStyle::from_owned(base);

        let node = NodeRef::new("Button");
        let computed = style.compute(&node, None);
        // color: red resolves to a non-Reset Color::Literal. Assert the
        // resolved color is set (not Reset/default).
        let color = computed.style.color.expect("color should be set");
        assert!(
            matches!(color, crate::color::Color::Literal(_)),
            "owned base should set the button color to a literal, got {color:?}"
        );
    }

    #[test]
    fn owned_base_then_override() {
        let path = temp_css("owned_base_then_override");
        std::fs::write(&path, ".primary { background: blue; }").unwrap();

        let base = Arc::new(Stylesheet::parse("Button { color: red; }").unwrap());
        let mut style = RuntimeStyle::from_owned(base);
        style.load_override(&path).unwrap();
        assert!(style.has_override());

        // A `.primary` Button: background comes from the override (blue),
        // color comes from the base (red).
        let node = NodeRef::new("Button").classes(&["primary"]);
        let computed = style.compute(&node, None);
        let color = computed.style.color.expect("base color (red) should apply");
        assert!(
            matches!(color, crate::color::Color::Literal(_)),
            "base color should still apply, got {color:?}"
        );
        let bg = computed
            .style
            .background
            .expect("override background (blue) should apply");
        assert!(
            matches!(bg, crate::color::Color::Literal(_)),
            "override background should apply, got {bg:?}"
        );

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn reload_if_changed_no_change() {
        let path = temp_css("reload_no_change");
        std::fs::write(&path, "Button { color: red; }").unwrap();

        let base = Arc::new(Stylesheet::parse("Root {}").unwrap());
        let mut style = RuntimeStyle::from_owned(base);
        style.load_override(&path).unwrap();

        // Immediately re-check without any change: should be false.
        let reloaded = style.reload_if_changed(&path).unwrap();
        assert!(!reloaded, "no file change → should not reload");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn reload_if_changed_after_edit() {
        let path = temp_css("reload_after_edit");
        std::fs::write(&path, "Button { color: red; }").unwrap();

        let base = Arc::new(Stylesheet::parse("Root {}").unwrap());
        let mut style = RuntimeStyle::from_owned(base);
        style.load_override(&path).unwrap();

        let before = style
            .compute(&NodeRef::new("Button"), None)
            .style
            .color
            .expect("v1 sets color");

        // Sleep to guarantee an observable mtime delta, then rewrite the file.
        thread::sleep(Duration::from_millis(20));
        std::fs::write(&path, "Button { color: blue; }").unwrap();

        let reloaded = style.reload_if_changed(&path).unwrap();
        assert!(reloaded, "file changed → should reload");

        let after = style
            .compute(&NodeRef::new("Button"), None)
            .style
            .color
            .expect("v2 sets color");
        assert_ne!(
            before, after,
            "the reloaded value should differ from the original"
        );

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn reload_if_changed_file_removed() {
        let path = temp_css("reload_file_removed");
        std::fs::write(&path, "Button { color: red; }").unwrap();

        let base = Arc::new(Stylesheet::parse("Root {}").unwrap());
        let mut style = RuntimeStyle::from_owned(base);
        style.load_override(&path).unwrap();
        assert!(style.has_override());

        std::fs::remove_file(&path).unwrap();
        let reloaded = style.reload_if_changed(&path).unwrap();
        assert!(reloaded, "override file disappearing should clear the override");
        assert!(!style.has_override());
    }
}
