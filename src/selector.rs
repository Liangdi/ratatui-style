//! CSS selectors — the pragmatic subset: compound selectors of the form
//! `Type.class#id:pseudo…` (plus comma lists and the `*` universal).
//!
//! Structural pseudo-classes (`:nth-child`, `:first-child`, etc.) are supported
//! and match against the node's [`Position`](crate::node::Position).
//!
//! Descendant (`A B`) and child (`A > B`) combinators are supported; they only
//! match when evaluated through a [`CascadeContext`](crate::CascadeContext)
//! (which threads an ancestor-identity stack). The one-shot
//! [`Stylesheet::compute`](crate::Stylesheet::compute) path has no ancestor
//! information, so a selector with an [`Selector::ancestor`] chain simply does
//! not match there.

use crate::error::{CssError, Result};
use crate::node::{Classes, Position, State, StyledNode};

/// A single pseudo-class state flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PseudoClass {
    Focus,
    Hover,
    Disabled,
    Checked,
    Active,
}

impl PseudoClass {
    fn parse(s: &str) -> Option<Self> {
        Some(match s.to_ascii_lowercase().as_str() {
            "focus" => Self::Focus,
            "hover" => Self::Hover,
            "disabled" => Self::Disabled,
            "checked" => Self::Checked,
            "active" => Self::Active,
            _ => return None,
        })
    }
}

/// An `an + b` expression for `:nth-child` / `:nth-last-child`.
///
/// Matches a 1-based index `i` when there exists a non-negative integer `n`
/// with `i == a * n + b`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct NthExpr {
    pub a: i32,
    pub b: i32,
}

impl NthExpr {
    /// Does 1-based index `i` satisfy `an + b` (for some n >= 0)?
    pub fn matches(self, i: i32) -> bool {
        if self.a == 0 {
            i == self.b
        } else {
            let d = i - self.b;
            d % self.a == 0 && d / self.a >= 0
        }
    }

    /// Parse a CSS `An+B` expression (case-insensitive).
    ///
    /// Accepted forms: `odd` → (2, 1); `even` → (2, 0); a bare integer `N` →
    /// (0, N); `an` / `-an` / `2n` / `-3n` forms (a = coefficient, b = 0);
    /// `an+b` / `2n+1` / `-n+3` / `2n-1` / `n` forms. Optional leading sign and
    /// surrounding spaces are tolerated. Returns an [`CssError::invalid_selector`]
    /// on garbage.
    pub fn parse(s: &str) -> Result<Self> {
        parse_nth(s)
    }
}

/// Parse a CSS `An+B` expression.
///
/// See [`NthExpr::parse`] for the accepted forms.
fn parse_nth(input: &str) -> Result<NthExpr> {
    // Normalize: trim and lowercase the ASCII form. We keep the raw bytes for
    // numeric parsing but compare the lowered variant for `odd`/`even`/`n`.
    let raw = input.trim();
    if raw.is_empty() {
        return Err(CssError::invalid_selector("empty nth expression"));
    }
    let lower = raw.to_ascii_lowercase();

    if lower == "odd" {
        return Ok(NthExpr { a: 2, b: 1 });
    }
    if lower == "even" {
        return Ok(NthExpr { a: 2, b: 0 });
    }

    // Split on the coefficient marker `n`. If absent, this is a bare integer.
    // Accept a single `n` only.
    let Some(n_pos) = lower.find('n') else {
        // No `n` — must be a bare integer (optional sign + digits). Reject any
        // trailing junk like `3x`.
        return parse_bare_int(raw).map(|b| NthExpr { a: 0, b });
    };

    // Ensure `n` is the only alphabetic character — reject `nx`, `an+bx`, etc.
    let rest_after_n = &lower[n_pos + 1..];
    if rest_after_n.chars().any(|c| c.is_ascii_alphabetic()) {
        return Err(CssError::invalid_selector(format!(
            "invalid nth expression `{input}`"
        )));
    }

    // Coefficient is the substring before `n`.
    let coef_part = raw[..n_pos].trim();
    let a = match coef_part {
        "" | "+" => 1,
        "-" => -1,
        _ => parse_bare_int(coef_part)?,
    };

    // Constant is the substring after `n` — optional sign + integer. An empty
    // constant means `b == 0`. We tolerate interior whitespace but the optional
    // sign must be followed by digits.
    let const_part = raw[n_pos + 1..].trim();
    let b = if const_part.is_empty() {
        0
    } else {
        parse_bare_int(const_part)?
    };

    Ok(NthExpr { a, b })
}

/// Parse a signed decimal integer from `s`. Tolerates interior whitespace
/// between an optional sign and the digits (CSS `An+B` permits forms like
/// `+ 1` and `- 2`). Rejects anything that isn't an optional `+`/`-` (with
/// optional following spaces) and one or more ASCII digits.
fn parse_bare_int(s: &str) -> Result<i32> {
    let s = s.trim();
    let bytes = s.as_bytes();
    let mut i = 0;
    let sign = match bytes.first() {
        Some(b'+') => {
            i += 1;
            "+"
        }
        Some(b'-') => {
            i += 1;
            "-"
        }
        _ => "",
    };
    // Allow whitespace between sign and digits, e.g. `+ 1`.
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    if i >= bytes.len() || !bytes[i..].iter().all(|b| b.is_ascii_digit()) {
        return Err(CssError::invalid_selector(format!(
            "invalid integer `{s}` in nth expression"
        )));
    }
    let digits = &s[i..];
    let cleaned: String = sign.chars().chain(digits.chars()).collect();
    cleaned
        .parse::<i32>()
        .map_err(|_| CssError::invalid_selector(format!("nth integer out of range: `{s}`")))
}

/// A (possibly parameterized) pseudo-class.
///
/// # Same-type sibling pseudos and the forward-sibling limitation
///
/// `NthOfType` and `FirstOfType` count only among previous siblings of the SAME
/// element type. They are fully computable from the previous-sibling identities
/// threaded in by [`CascadeContext`](crate::CascadeContext).
///
/// `:last-of-type`, `:nth-last-of-type`, and `:only-of-type` are deliberately
/// **not** supported: they require the total same-type sibling count, which in
/// turn requires knowledge of siblings that come AFTER the subject. The cascade
/// walk only tracks PREVIOUS siblings, so these variants are not determinable at
/// match time. Parsing them yields an "unsupported pseudo-class" error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pseudo {
    /// A simple state flag: :focus, :hover, :disabled, :checked, :active.
    State(PseudoClass),
    FirstChild,
    LastChild,
    OnlyChild,
    /// :nth-child(an+b)
    NthChild(NthExpr),
    /// :nth-last-child(an+b)
    NthLastChild(NthExpr),
    /// :nth-of-type(an+b) — among same-type previous siblings, the node is the
    /// (an+b)-th of its type. Requires sibling identities (matched through
    /// [`matches_chain`](Selector::matches_chain)); the one-shot
    /// [`matches_values`](Selector::matches_values) path has no siblings, so
    /// `:nth-of-type` does not match there.
    NthOfType(NthExpr),
    /// :first-of-type — no previous sibling shares this element type. Same
    /// sibling-identity requirement as [`NthOfType`](Self::NthOfType).
    FirstOfType,
}

impl Pseudo {
    /// Parse a single pseudo-class token (the part after `:` in a selector).
    ///
    /// `token` is the bare name for unparameterized pseudos (`focus`,
    /// `first-child`); for parameterized pseudos (`nth-child(2n+1)`) the caller
    /// passes the full `name(args)` form here.
    fn parse(token: &str) -> Result<Self> {
        // Parameterized form: name(args).
        if let Some(open) = token.find('(') {
            let name = token[..open].trim();
            if !token.ends_with(')') {
                return Err(CssError::invalid_selector(format!(
                    "unterminated pseudo-class `{token}`"
                )));
            }
            let inner = &token[open + 1..token.len() - 1];
            return match name.to_ascii_lowercase().as_str() {
                "nth-child" => Ok(Self::NthChild(parse_nth(inner)?)),
                "nth-last-child" => Ok(Self::NthLastChild(parse_nth(inner)?)),
                "nth-of-type" => Ok(Self::NthOfType(parse_nth(inner)?)),
                // nth-last-of-type needs forward-sibling knowledge — unsupported.
                other => Err(CssError::invalid_selector(format!(
                    "unsupported pseudo-class `:{other}`"
                ))),
            };
        }

        // Unparameterized: state flag or structural keyword.
        match token.to_ascii_lowercase().as_str() {
            "first-child" => Ok(Self::FirstChild),
            "last-child" => Ok(Self::LastChild),
            "only-child" => Ok(Self::OnlyChild),
            "first-of-type" => Ok(Self::FirstOfType),
            // last-of-type / only-of-type need forward-sibling knowledge —
            // unsupported. Fall through to the generic error below by treating
            // them as unknown names (they have no PseudoClass equivalent).
            other => match PseudoClass::parse(other) {
                Some(p) => Ok(Self::State(p)),
                None => Err(CssError::invalid_selector(format!(
                    "unsupported pseudo-class `:{other}`"
                ))),
            },
        }
    }
}

/// A combinator joining a compound selector to an ancestor/sibling compound.
///
/// Produced by the [`Selector`](crate::Selector) parser for descendant (`A B`),
/// child (`A > B`), adjacent-sibling (`A + B`), and general-sibling (`A ~ B`)
/// combinators. Only reachable through [`Selector::ancestor`].
///
/// Adjacent (`+`) and general (`~`) sibling combinators are matched against the
/// previous-sibling identities threaded in by
/// [`CascadeContext`](crate::CascadeContext). Nested sibling chains such as
/// `A + B + C`, `A ~ B ~ C`, and mixed `A + B ~ C` resolve correctly: when
/// matching recurses into a sibling compound (the `B` in `A + B + C`), the
/// sub-selector is handed the correct *prefix* of the subject's previous
/// siblings — namely the slice that precedes the matched sibling — so the inner
/// sibling combinator can resolve.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Combinator {
    /// `A B` — the subject is a descendant of `A` (any depth).
    Descendant,
    /// `A > B` — the subject is a direct child of `A`.
    Child,
    /// `A + B` — the subject `B` is the immediately-following sibling of `A`.
    Adjacent,
    /// `A ~ B` — the subject `B` follows sibling `A` somewhere (not necessarily
    /// immediately).
    Sibling,
}

/// A compound selector: an optional type, plus class/id/pseudo qualifiers.
///
/// Compound fields (`type_name`, `classes`, `id`, `pseudos`) describe the
/// **subject** — the element this rule applies to. The optional [`ancestor`]
/// chain extends the match leftward: `Panel > Button` parses to a `Selector`
/// whose subject is `Button` and whose `ancestor` is
/// `Some((Child, Selector{ type_name: "Panel", .. }))`.
///
/// Combinator chains only match when evaluated through a
/// [`CascadeContext`](crate::CascadeContext) (the one path that supplies
/// ancestor identities). The one-shot
/// [`Stylesheet::compute`](crate::Stylesheet::compute) API has no ancestor
/// information, so a selector with an `ancestor` chain returns `false` from
/// [`matches`](Self::matches) there — it does not panic.
///
/// [`ancestor`]: Self::ancestor
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selector {
    /// Subject compound's type name (`Button` in `Panel > Button`).
    pub type_name: Option<String>,
    /// Subject compound's class list.
    pub classes: Vec<String>,
    /// Subject compound's id.
    pub id: Option<String>,
    /// Subject compound's pseudo-classes.
    pub pseudos: Vec<Pseudo>,
    /// Optional ancestor compound + the combinator joining it to this selector.
    ///
    /// `Panel > Button` parses to
    /// `Selector{ type_name: "Button", ancestor: Some((Child, Selector{ type_name: "Panel", ancestor: None })) }`.
    /// `None` for a plain compound (the overwhelmingly common case).
    pub ancestor: Option<(Combinator, Box<Selector>)>,
}

impl Default for Selector {
    fn default() -> Self {
        Self::universal()
    }
}

/// A snapshot of a node's selector-relevant data, used for combinator matching.
///
/// Built once per node on the combinator-aware cascade path
/// ([`CascadeContext`](crate::CascadeContext) when the stylesheet
/// `has_combinators()`). It is `pub(crate)` because it is an internal
/// threading detail of the cascade, not part of the public surface.
#[derive(Clone, Debug, Default)]
pub(crate) struct NodeIdentity {
    pub type_name: String,
    pub id: Option<String>,
    pub classes: Vec<String>,
    pub state: State,
    pub position: Position,
}

impl NodeIdentity {
    /// Snapshot a [`StyledNode`]'s selector-relevant fields.
    pub(crate) fn from_node(node: &dyn StyledNode) -> Self {
        Self {
            type_name: node.type_name().to_string(),
            id: node.id().map(str::to_string),
            classes: node.classes().as_slice().iter().map(|s| (*s).to_string()).collect(),
            state: node.state(),
            position: node.position(),
        }
    }
}

impl Selector {
    /// The universal selector `*` — matches every element.
    pub fn universal() -> Self {
        Self {
            type_name: None,
            classes: Vec::new(),
            id: None,
            pseudos: Vec::new(),
            ancestor: None,
        }
    }

    /// Parse one or more comma-separated selectors.
    ///
    /// Each comma-separated part may itself contain descendant (` `) and child
    /// (`>`) combinators, e.g. `"Panel Button, .modal > Button"`.
    pub fn parse_list(s: &str) -> Result<Vec<Self>> {
        s.split(',').map(|part| Self::parse_chain(part.trim())).collect()
    }

    /// Parse a single compound selector with no combinator.
    ///
    /// For a selector that may contain combinators use [`parse_chain`](Self::parse_chain)
    /// (or [`parse_list`](Self::parse_list), which splits on `,` first then calls
    /// `parse_chain` per part).
    pub fn parse_compound(s: &str) -> Result<Self> {
        Self::parse_compound_into(s)
    }

    /// Parse a selector chain — one or more compounds joined by descendant
    /// (` `), child (`>`), adjacent-sibling (`+`), or general-sibling (`~`)
    /// combinators.
    ///
    /// Parentheses are tracked so spaces and `+` inside `:nth-child(2n + 1)`
    /// are not mistaken for combinators.
    pub fn parse_chain(s: &str) -> Result<Self> {
        let s = s.trim();
        if s.is_empty() {
            return Err(CssError::invalid_selector("empty selector"));
        }

        // Tokenize into an ordered list of [compound, combinator, compound, …].
        let tokens = tokenize_chain(s)?;

        // Fold left-to-right: the running `subject` is the rightmost compound
        // parsed so far; each (combinator, compound) pair makes `compound` the
        // new subject and the previous subject its ancestor joined by
        // `combinator`. For `[Panel, Descendant, Button]` this yields
        // `Selector{ subject: Button, ancestor: Some((Descendant, Panel)) }`.
        let mut iter = tokens.into_iter();
        // The first element must be a Compound.
        let first_compound = match iter.next() {
            Some(ChainToken::Compound(c)) => c,
            Some(ChainToken::Combinator(_)) => {
                return Err(CssError::invalid_selector(format!(
                    "selector `{s}` begins with a combinator"
                )));
            }
            None => return Err(CssError::invalid_selector("empty selector")),
        };
        let mut subject = Self::parse_compound_into(&first_compound)?;

        // Walk the remaining tokens as (combinator, compound) pairs.
        while let Some(comb) = iter.next() {
            let combinator = match comb {
                ChainToken::Combinator(c) => c,
                ChainToken::Compound(_) => unreachable!("non-first compound without combinator"),
            };
            let Some(ChainToken::Compound(c)) = iter.next() else {
                return Err(CssError::invalid_selector(format!(
                    "selector `{s}` ends with a combinator"
                )));
            };
            // The new compound becomes the subject; the previous subject is its
            // ancestor, joined by `combinator`.
            let new_subject = Self::parse_compound_into(&c)?;
            subject = Self {
                type_name: new_subject.type_name,
                classes: new_subject.classes,
                id: new_subject.id,
                pseudos: new_subject.pseudos,
                ancestor: Some((combinator, Box::new(subject))),
            };
        }

        Ok(subject)
    }

    /// Parse a single compound selector (no combinators) into a `Selector`
    /// with `ancestor: None`. This is the body of the historical
    /// `parse_compound`; `parse_compound` and `parse_chain` both delegate here.
    fn parse_compound_into(s: &str) -> Result<Self> {
        let s = s.trim();
        if s.is_empty() {
            return Err(CssError::invalid_selector("empty selector"));
        }

        let mut sel = Self::universal();
        let bytes = s.as_bytes();
        let len = s.len();
        let mut idx = 0usize;

        // Optional leading type name or `*`.
        if let Some(&c) = bytes.first() {
            if c == b'*' {
                idx += 1;
            } else if !matches!(c, b'.' | b'#' | b':') {
                let start = idx;
                while idx < len {
                    let c = bytes[idx];
                    if matches!(c, b'.' | b'#' | b':') {
                        break;
                    }
                    idx += 1;
                }
                sel.type_name = Some(s[start..idx].to_string());
            }
        }

        while idx < len {
            let delim = bytes[idx] as char;
            idx += 1;
            let start = idx;
            // Read a token. For `:` we allow a parenthesized argument group that
            // may itself contain delimiters we'd otherwise stop on (none today,
            // but the closing `)` is the terminator). For `.`/`#` we stop at the
            // next delimiter.
            if delim == ':' {
                // Read the name, then — if a `(` follows — consume through the
                // matching `)`.
                while idx < len && !matches!(bytes[idx], b'.' | b'#' | b':' | b'(') {
                    idx += 1;
                }
                if idx < len && bytes[idx] == b'(' {
                    // Consume up to and including the closing `)`. We do not
                    // support nested parens in nth expressions; the first `)`
                    // terminates.
                    idx += 1; // consume `(`
                    while idx < len && bytes[idx] != b')' {
                        idx += 1;
                    }
                    if idx >= len {
                        return Err(CssError::invalid_selector(format!(
                            "unterminated pseudo-class in `{s}`"
                        )));
                    }
                    idx += 1; // consume `)`
                }
            } else {
                while idx < len && !matches!(bytes[idx], b'.' | b'#' | b':') {
                    idx += 1;
                }
            }
            if idx == start {
                return Err(CssError::invalid_selector(format!(
                    "selector `{s}` has a dangling `{delim}`"
                )));
            }
            let token = &s[start..idx];
            match delim {
                '.' => sel.classes.push(token.to_string()),
                '#' => {
                    if sel.id.is_some() {
                        return Err(CssError::invalid_selector(format!(
                            "selector `{s}` has multiple ids"
                        )));
                    }
                    sel.id = Some(token.to_string());
                }
                ':' => sel.pseudos.push(Pseudo::parse(token)?),
                _ => unreachable!("delimiter handled above"),
            }
        }

        Ok(sel)
    }

    /// Specificity as `(ids, classes_and_pseudos, type)`, comparable as a tuple.
    ///
    /// Sums across the subject compound AND every ancestor compound in the
    /// chain: `Panel > Button.primary` has specificity `(0, 1, 2)`.
    pub fn specificity(&self) -> (u32, u32, u32) {
        let ids = if self.id.is_some() { 1 } else { 0 };
        let cp = (self.classes.len() + self.pseudos.len()) as u32;
        let ty = if self.type_name.is_some() { 1 } else { 0 };
        let (a_ids, a_cp, a_ty) = match &self.ancestor {
            None => (0u32, 0u32, 0u32),
            Some((_, anc)) => anc.specificity(),
        };
        (ids + a_ids, cp + a_cp, ty + a_ty)
    }

    /// Whether this selector matches a given node (subject compound only).
    ///
    /// **Combinator limitation**: a selector with an [`ancestor`](Self::ancestor)
    /// chain carries ancestor requirements that this method cannot evaluate
    /// (it has no ancestor context), so it returns `false` for any combinator
    /// selector. Use a [`CascadeContext`](crate::CascadeContext) to match
    /// combinator selectors against a real ancestor stack.
    pub fn matches(&self, node: &dyn StyledNode) -> bool {
        let position = node.position();
        self.matches_values(
            node.type_name(),
            node.id(),
            &node.classes(),
            node.state(),
            &position,
        )
    }

    /// Core match against raw values — **subject compound only**.
    ///
    /// This is what the one-shot cascade path hoists out of the per-rule loop:
    /// callers fetch `classes` once per node and pass the [`Classes`] view in
    /// repeatedly. [`Selector::matches`] delegates here, guaranteeing a single
    /// source of truth for the match semantics.
    ///
    /// # Combinator limitation
    ///
    /// This path has no ancestor information, so a selector with an
    /// [`ancestor`](Self::ancestor) chain does **not** match here (it returns
    /// `false` after testing the subject compound, since the ancestor chain is
    /// unsatisfiable without ancestor identities). Use
    /// [`matches_chain`](Self::matches_chain) (via
    /// [`CascadeContext`](crate::CascadeContext)) to match combinator selectors.
    ///
    /// # Structural pseudo-classes and the `sibling_count > 0` guard
    ///
    /// `:first-child`, `:last-child`, `:only-child`, `:nth-child`, and
    /// `:nth-last-child` all require `position.sibling_count > 0`. A default
    /// [`Position`] (count 0) carries no sibling information, so it is treated
    /// as "does not match" rather than spuriously matching `:first-child`
    /// (whose naive condition `index == 0` would be true for the default).
    ///
    /// The same-type sibling pseudos `:nth-of-type` and `:first-of-type` need
    /// the previous-sibling identity list, which this path does not have. On the
    /// one-shot path they see `same_type_before == 0`, so `:first-of-type` and
    /// `:nth-of-type(1)` trivially match while `:nth-of-type(k>1)` does not.
    /// Use [`matches_chain`](Self::matches_chain) via
    /// [`CascadeContext`](crate::CascadeContext) for accurate same-type counts.
    pub(crate) fn matches_values(
        &self,
        type_name: &str,
        id: Option<&str>,
        classes: &Classes<'_>,
        state: State,
        position: &Position,
    ) -> bool {
        // The one-shot path has no sibling context: same_type_before = 0, so
        // `:nth-of-type(1)` / `:first-of-type` match and higher indices do not.
        if !self.matches_subject_raw(type_name, id, classes, state, position, 0) {
            return false;
        }
        // A combinator chain cannot be satisfied without ancestor identities.
        self.ancestor.is_none()
    }

    /// Subject-compound match against raw values (the historical
    /// `matches_values` body, factored out). Does NOT consult [`ancestor`].
    ///
    /// `same_type_before` is the count of previous siblings sharing the subject's
    /// element type-name — supplied by [`matches_subject`] from the threaded
    /// sibling list, or `0` on the one-shot path. It backs the
    /// `:nth-of-type` / `:first-of-type` pseudos.
    ///
    /// [`ancestor`]: Self::ancestor
    fn matches_subject_raw(
        &self,
        type_name: &str,
        id: Option<&str>,
        classes: &Classes<'_>,
        state: State,
        position: &Position,
        same_type_before: i32,
    ) -> bool {
        // Type: case-insensitive (convenience); universal matches anything.
        if let Some(t) = &self.type_name
            && !type_name.eq_ignore_ascii_case(t)
        {
            return false;
        }
        // Id: case-sensitive.
        if let Some(sel_id) = &self.id
            && id != Some(sel_id.as_str())
        {
            return false;
        }
        // Classes: all must be present (case-sensitive).
        for c in &self.classes {
            if !classes.contains(c.as_str()) {
                return false;
            }
        }
        // Pseudo-classes: all must be satisfied.
        self.pseudos_satisfied(state, position, same_type_before)
    }

    /// Subject compound of this selector against one [`NodeIdentity`].
    ///
    /// `siblings` is the node's list of PREVIOUS siblings (oldest-first), used
    /// only by the same-type-counting pseudos `:nth-of-type` and `:first-of-type`.
    /// The other structural pseudos consult `id.position`. Callers that have no
    /// sibling context (the one-shot `matches_values` path) pass `&[]`, in which
    /// case `:nth-of-type(1)`/`:first-of-type` trivially match (a node with no
    /// previous same-type siblings is the first of its type) and other
    /// `:nth-of-type(k>1)` do not.
    fn matches_subject(&self, id: &NodeIdentity, siblings: &[NodeIdentity]) -> bool {
        let class_strs: Vec<&str> = id.classes.iter().map(String::as_str).collect();
        let classes = Classes::from_slice(&class_strs);
        // The of-type pseudos need the previous same-type sibling count, which
        // we compute here and pass into the raw matcher via a side channel.
        let same_type_before = siblings
            .iter()
            .filter(|s| s.type_name.eq_ignore_ascii_case(&id.type_name))
            .count() as i32;
        self.matches_subject_raw(
            &id.type_name,
            id.id.as_deref(),
            &classes,
            id.state,
            &id.position,
            same_type_before,
        )
    }

    /// Subject matches `node_id`, and the ancestor chain matches `ancestors`
    /// (closest ancestor = last element). Right-to-left CSS matching.
    ///
    /// `siblings` is the node's list of PREVIOUS siblings (oldest-first,
    /// closest = last), threaded in by [`CascadeContext`](crate::CascadeContext).
    /// It backs the adjacent (`+`), general (`~`), and same-type (`:nth-of-type`,
    /// `:first-of-type`) sibling matching. When recursing into a descendant or
    /// child ancestor compound the sub-selector is given `&[]` for siblings
    /// (ancestor matching uses the `ancestors` slice, not siblings); when
    /// recursing into a sibling-joined compound the sub-selector is given the
    /// correct PREFIX of `siblings` so a nested sibling chain (`A + B + C`)
    /// resolves.
    pub(crate) fn matches_chain(
        &self,
        node_id: &NodeIdentity,
        ancestors: &[NodeIdentity],
        siblings: &[NodeIdentity],
    ) -> bool {
        if !self.matches_subject(node_id, siblings) {
            return false;
        }
        match &self.ancestor {
            None => true,
            Some((Combinator::Child, anc)) => match ancestors.last() {
                None => false,
                Some(parent) => anc.matches_chain(parent, &ancestors[..ancestors.len() - 1], &[]),
            },
            Some((Combinator::Descendant, anc)) => {
                // Search closest-first (last) backward through the ancestor stack.
                for i in (0..ancestors.len()).rev() {
                    if anc.matches_chain(&ancestors[i], &ancestors[..i], &[]) {
                        return true;
                    }
                }
                false
            }
            Some((Combinator::Adjacent, anc)) => {
                // The matched sibling is `siblings.last()`; that sibling's OWN
                // previous siblings are `&siblings[..n-1]`, which we thread in so
                // a nested adjacent chain like `A + B + C` can resolve the inner
                // `A + B`. Siblings share ancestors, so `ancestors` is forwarded.
                let n = siblings.len();
                if n == 0 {
                    return false;
                }
                let last = &siblings[n - 1];
                anc.matches_chain(last, ancestors, &siblings[..n - 1])
            }
            Some((Combinator::Sibling, anc)) => {
                // Some previous sibling matches the ancestor compound. Search
                // closest-first (reverse). For a match at index `i`, that
                // sibling's OWN previous siblings are `&siblings[..i]`, threaded
                // so a nested general-sibling chain like `A ~ B ~ C` resolves.
                for i in (0..siblings.len()).rev() {
                    if anc.matches_chain(&siblings[i], ancestors, &siblings[..i]) {
                        return true;
                    }
                }
                false
            }
        }
    }

    /// Evaluate all [`pseudos`](Self::pseudos) against `state`, `position`, and
    /// `same_type_before` (the count of previous siblings sharing the subject's
    /// element type-name — used by `:nth-of-type` / `:first-of-type`).
    fn pseudos_satisfied(&self, state: State, position: &Position, same_type_before: i32) -> bool {
        for p in &self.pseudos {
            let on = match p {
                Pseudo::State(PseudoClass::Focus) => state.focus,
                Pseudo::State(PseudoClass::Hover) => state.hover,
                Pseudo::State(PseudoClass::Disabled) => state.disabled,
                Pseudo::State(PseudoClass::Checked) => state.checked,
                Pseudo::State(PseudoClass::Active) => state.active,
                Pseudo::FirstChild => position.sibling_count > 0 && position.index == 0,
                Pseudo::LastChild => {
                    position.sibling_count > 0 && position.index == position.sibling_count - 1
                }
                Pseudo::OnlyChild => position.sibling_count == 1,
                Pseudo::NthChild(expr) => {
                    position.sibling_count > 0 && expr.matches(position.index as i32 + 1)
                }
                Pseudo::NthLastChild(expr) => {
                    position.sibling_count > 0
                        && expr.matches(position.sibling_count as i32 - position.index as i32)
                }
                // of_type_index = 1 + (count of previous same-type siblings).
                Pseudo::NthOfType(expr) => expr.matches(same_type_before + 1),
                // No previous sibling shares this type.
                Pseudo::FirstOfType => same_type_before == 0,
            };
            if !on {
                return false;
            }
        }
        true
    }
}

/// A token emitted by [`tokenize_chain`]: either a compound string or a
/// combinator joining two compounds.
enum ChainToken {
    Compound(String),
    Combinator(Combinator),
}

/// Split a (comma-already-split) selector string into an ordered list of
/// compounds and combinators, respecting parenthesis depth so spaces/`+` inside
/// `:nth-child(2n + 1)` are not mistaken for combinators.
///
/// Returns `[Compound, Combinator, Compound, …]`, always starting with a
/// `Compound` (possibly empty for input like `> Button`).
fn tokenize_chain(s: &str) -> Result<Vec<ChainToken>> {
    let mut tokens: Vec<ChainToken> = Vec::new();
    let mut cur = String::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    let mut depth: u32 = 0;

    // Helper to flush the accumulated compound, if any.
    macro_rules! flush {
        () => {{
            if !cur.is_empty() {
                tokens.push(ChainToken::Compound(std::mem::take(&mut cur)));
            }
        }};
    }

    while i < bytes.len() {
        let b = bytes[i];
        if depth == 0 {
            match b {
                b'(' => {
                    depth += 1;
                    cur.push('(');
                }
                b')' => {
                    // Unbalanced `)` at depth 0 — accumulate it; the compound
                    // parser will reject it downstream.
                    cur.push(')');
                }
                b' ' | b'\t' | b'\n' | b'\r' => {
                    // A run of whitespace: peek the next non-whitespace char.
                    // If it is `>`, this whitespace is just spacing around an
                    // explicit combinator — consume without emitting a
                    // Descendant. Otherwise emit a Descendant combinator (if we
                    // have an accumulated compound).
                    let mut j = i;
                    while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                        j += 1;
                    }
                    i = j;
                    if i >= bytes.len() {
                        // trailing whitespace — nothing follows.
                        break;
                    }
                    match bytes[i] {
                        b'>' => {
                            flush!();
                            tokens.push(ChainToken::Combinator(Combinator::Child));
                            i += 1; // consume `>`
                            // Skip whitespace after the explicit combinator so it
                            // does not re-trigger a spurious Descendant on the
                            // next loop iteration.
                            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                                i += 1;
                            }
                            continue;
                        }
                        b'+' => {
                            flush!();
                            tokens.push(ChainToken::Combinator(Combinator::Adjacent));
                            i += 1;
                            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                                i += 1;
                            }
                            continue;
                        }
                        b'~' => {
                            flush!();
                            tokens.push(ChainToken::Combinator(Combinator::Sibling));
                            i += 1;
                            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                                i += 1;
                            }
                            continue;
                        }
                        _ => {
                            flush!();
                            tokens.push(ChainToken::Combinator(Combinator::Descendant));
                            continue;
                        }
                    }
                }
                b'>' => {
                    flush!();
                    tokens.push(ChainToken::Combinator(Combinator::Child));
                    // Skip whitespace after the explicit combinator (mirrors the
                    // whitespace-peek branch above).
                    i += 1;
                    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                        i += 1;
                    }
                    continue;
                }
                b'+' => {
                    flush!();
                    tokens.push(ChainToken::Combinator(Combinator::Adjacent));
                    i += 1;
                    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                        i += 1;
                    }
                    continue;
                }
                b'~' => {
                    flush!();
                    tokens.push(ChainToken::Combinator(Combinator::Sibling));
                    i += 1;
                    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                        i += 1;
                    }
                    continue;
                }
                _ => {
                    // Regular char: append to the current compound. Copy the
                    // whole UTF-8 char.
                    let ch = s[i..].chars().next().expect("non-empty slice");
                    cur.push(ch);
                    i += ch.len_utf8();
                    continue;
                }
            }
        } else {
            // Inside parentheses — accumulate verbatim, tracking depth.
            match b {
                b'(' => depth += 1,
                b')' => depth -= 1,
                _ => {}
            }
            let ch = s[i..].chars().next().expect("non-empty slice");
            cur.push(ch);
            i += ch.len_utf8();
            continue;
        }
        i += 1;
    }
    flush!();
    Ok(tokens)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::{OwnedNode, Position, State};

    #[test]
    fn parse_compound() {
        let s = Selector::parse_compound("Button.primary#save:focus").unwrap();
        assert_eq!(s.type_name.as_deref(), Some("Button"));
        assert_eq!(s.classes, vec!["primary"]);
        assert_eq!(s.id.as_deref(), Some("save"));
        assert_eq!(s.pseudos, vec![Pseudo::State(PseudoClass::Focus)]);
        assert_eq!(s.specificity(), (1, 2, 1));
    }

    #[test]
    fn universal_specificity() {
        assert_eq!(Selector::universal().specificity(), (0, 0, 0));
    }

    #[test]
    fn matching() {
        let sel = Selector::parse_compound("Button.primary").unwrap();
        let n = OwnedNode::new("Button").with_classes(["primary"]);
        assert!(sel.matches(&n));

        let wrong_type = OwnedNode::new("Text").with_classes(["primary"]);
        assert!(!sel.matches(&wrong_type));

        let missing_class = OwnedNode::new("Button");
        assert!(!sel.matches(&missing_class));
    }

    #[test]
    fn matching_with_state() {
        let sel = Selector::parse_compound("Button:disabled").unwrap();
        let on = OwnedNode::new("Button").with_state(State::disabled());
        let off = OwnedNode::new("Button");
        assert!(sel.matches(&on));
        assert!(!sel.matches(&off));
    }

    #[test]
    fn comma_list() {
        let list = Selector::parse_list("Text, .muted, #title").unwrap();
        assert_eq!(list.len(), 3);
    }

    // ---------------------------------------------------------------------
    // NthExpr::matches
    // ---------------------------------------------------------------------

    #[test]
    fn nth_matches_odd_even() {
        let odd = NthExpr { a: 2, b: 1 };
        let even = NthExpr { a: 2, b: 0 };
        // 1-based indices 1..=6.
        for i in 1..=6 {
            assert_eq!(odd.matches(i), i % 2 == 1, "odd @ {i}");
            assert_eq!(even.matches(i), i % 2 == 0, "even @ {i}");
        }
    }

    #[test]
    fn nth_matches_2n_plus_1() {
        let e = NthExpr { a: 2, b: 1 };
        // matches 1,3,5,...
        for i in 1..=6 {
            assert_eq!(e.matches(i), i % 2 == 1, "2n+1 @ {i}");
        }
    }

    #[test]
    fn nth_matches_minus_n_plus_2() {
        // -n+2 => matches n=0 -> 2, n=1 -> 1; nothing else (n>=0, i>=1).
        let e = NthExpr { a: -1, b: 2 };
        assert!(e.matches(1));
        assert!(e.matches(2));
        for i in 3..=6 {
            assert!(!e.matches(i), "-n+2 should not match {i}");
        }
    }

    #[test]
    fn nth_matches_bare_int() {
        let e = NthExpr { a: 0, b: 3 };
        assert!(e.matches(3));
        assert!(!e.matches(1));
        assert!(!e.matches(2));
        assert!(!e.matches(4));
    }

    // ---------------------------------------------------------------------
    // NthExpr parsing
    // ---------------------------------------------------------------------

    #[test]
    fn nth_parse_keywords() {
        assert_eq!(NthExpr::parse("odd").unwrap(), NthExpr { a: 2, b: 1 });
        assert_eq!(NthExpr::parse("even").unwrap(), NthExpr { a: 2, b: 0 });
        assert_eq!(NthExpr::parse("ODD").unwrap(), NthExpr { a: 2, b: 1 });
        assert_eq!(NthExpr::parse("Even").unwrap(), NthExpr { a: 2, b: 0 });
    }

    #[test]
    fn nth_parse_bare_int() {
        assert_eq!(NthExpr::parse("3").unwrap(), NthExpr { a: 0, b: 3 });
        assert_eq!(NthExpr::parse("+3").unwrap(), NthExpr { a: 0, b: 3 });
        assert_eq!(NthExpr::parse("-3").unwrap(), NthExpr { a: 0, b: -3 });
    }

    #[test]
    fn nth_parse_n_forms() {
        assert_eq!(NthExpr::parse("n").unwrap(), NthExpr { a: 1, b: 0 });
        assert_eq!(NthExpr::parse("2n").unwrap(), NthExpr { a: 2, b: 0 });
        assert_eq!(NthExpr::parse("-3n").unwrap(), NthExpr { a: -3, b: 0 });
        assert_eq!(NthExpr::parse("-n").unwrap(), NthExpr { a: -1, b: 0 });
    }

    #[test]
    fn nth_parse_an_plus_b_forms() {
        assert_eq!(NthExpr::parse("2n+1").unwrap(), NthExpr { a: 2, b: 1 });
        assert_eq!(NthExpr::parse("2n-1").unwrap(), NthExpr { a: 2, b: -1 });
        assert_eq!(NthExpr::parse("-n+2").unwrap(), NthExpr { a: -1, b: 2 });
        assert_eq!(NthExpr::parse("-2n+3").unwrap(), NthExpr { a: -2, b: 3 });
        // Tolerate interior spaces.
        assert_eq!(NthExpr::parse("2n + 1").unwrap(), NthExpr { a: 2, b: 1 });
        assert_eq!(NthExpr::parse(" -n + 2 ").unwrap(), NthExpr { a: -1, b: 2 });
    }

    #[test]
    fn nth_parse_garbage_errors() {
        assert!(NthExpr::parse("").is_err());
        assert!(NthExpr::parse("abc").is_err());
        assert!(NthExpr::parse("2x").is_err());
        assert!(NthExpr::parse("n+").is_err());
        assert!(NthExpr::parse("2n+").is_err());
        assert!(NthExpr::parse("--").is_err());
        assert!(NthExpr::parse("2n+1x").is_err());
        assert!(NthExpr::parse("+").is_err());
    }

    // ---------------------------------------------------------------------
    // Selector parsing — structural pseudos
    // ---------------------------------------------------------------------

    #[test]
    fn parse_nth_child_selector() {
        let s = Selector::parse_compound("Item:nth-child(2n+1)").unwrap();
        assert_eq!(s.type_name.as_deref(), Some("Item"));
        assert_eq!(s.pseudos.len(), 1);
        assert_eq!(s.pseudos[0], Pseudo::NthChild(NthExpr { a: 2, b: 1 }));
    }

    #[test]
    fn parse_first_child_selector() {
        let s = Selector::parse_compound("tr:first-child").unwrap();
        assert_eq!(s.pseudos, vec![Pseudo::FirstChild]);
    }

    #[test]
    fn parse_last_child_selector() {
        let s = Selector::parse_compound("li:last-child").unwrap();
        assert_eq!(s.pseudos, vec![Pseudo::LastChild]);
    }

    #[test]
    fn parse_only_child_selector() {
        let s = Selector::parse_compound("td:only-child").unwrap();
        assert_eq!(s.pseudos, vec![Pseudo::OnlyChild]);
    }

    #[test]
    fn parse_nth_last_child_selector() {
        let s = Selector::parse_compound("tr:nth-last-child(odd)").unwrap();
        assert_eq!(
            s.pseudos,
            vec![Pseudo::NthLastChild(NthExpr { a: 2, b: 1 })]
        );
    }

    #[test]
    fn nth_child_specificity_counts_as_one() {
        let s = Selector::parse_compound(":nth-child(2n+1)").unwrap();
        // pseudo bucket counts as 1, no type, no id → (0,1,0).
        assert_eq!(s.specificity(), (0, 1, 0));
    }

    #[test]
    fn unknown_pseudo_errors() {
        assert!(Selector::parse_compound("a:visited").is_err());
        // The forward-sibling-dependent of-type variants are unsupported.
        assert!(Selector::parse_compound("a:last-of-type").is_err());
        assert!(Selector::parse_compound("a:only-of-type").is_err());
        assert!(Selector::parse_compound("a:nth-last-of-type(2)").is_err());
    }

    #[test]
    fn unterminated_pseudo_errors() {
        assert!(Selector::parse_compound("Item:nth-child(2n+1").is_err());
    }

    // ---------------------------------------------------------------------
    // Structural matching against Position
    // ---------------------------------------------------------------------

    fn pos(index: usize, count: usize) -> Position {
        Position::new(index, count)
    }

    #[test]
    fn first_child_matches() {
        let sel = Selector::parse_compound("Item:first-child").unwrap();
        let classes = Classes::from_slice(&[]);
        // index 0 of 3 matches.
        assert!(sel.matches_values("Item", None, &classes, State::empty(), &pos(0, 3)));
        // index 1 of 3 does not.
        assert!(!sel.matches_values("Item", None, &classes, State::empty(), &pos(1, 3)));
    }

    #[test]
    fn last_child_matches() {
        let sel = Selector::parse_compound("Item:last-child").unwrap();
        let classes = Classes::from_slice(&[]);
        assert!(sel.matches_values("Item", None, &classes, State::empty(), &pos(2, 3)));
        assert!(!sel.matches_values("Item", None, &classes, State::empty(), &pos(1, 3)));
    }

    #[test]
    fn only_child_matches() {
        let sel = Selector::parse_compound("Item:only-child").unwrap();
        let classes = Classes::from_slice(&[]);
        assert!(sel.matches_values("Item", None, &classes, State::empty(), &pos(0, 1)));
        assert!(!sel.matches_values("Item", None, &classes, State::empty(), &pos(0, 3)));
    }

    #[test]
    fn nth_child_matches() {
        let sel = Selector::parse_compound("Item:nth-child(odd)").unwrap();
        let classes = Classes::from_slice(&[]);
        // index 0 (1-based 1) odd → match; index 1 (1-based 2) even → no; index 2 (1-based 3) odd → yes.
        assert!(sel.matches_values("Item", None, &classes, State::empty(), &pos(0, 3)));
        assert!(!sel.matches_values("Item", None, &classes, State::empty(), &pos(1, 3)));
        assert!(sel.matches_values("Item", None, &classes, State::empty(), &pos(2, 3)));
    }

    #[test]
    fn nth_last_child_matches() {
        let sel = Selector::parse_compound("Item:nth-last-child(1)").unwrap();
        let classes = Classes::from_slice(&[]);
        // 1-based-from-end = sibling_count - index. Last child (index 2 of 3) → 1.
        assert!(sel.matches_values("Item", None, &classes, State::empty(), &pos(2, 3)));
        assert!(!sel.matches_values("Item", None, &classes, State::empty(), &pos(1, 3)));
    }

    #[test]
    fn default_position_does_not_match_structural() {
        // sibling_count == 0 means "no position info" — must NOT match even
        // though index defaults to 0 (which would otherwise satisfy
        // :first-child naively).
        let sel = Selector::parse_compound("Item:first-child").unwrap();
        let classes = Classes::from_slice(&[]);
        let default = Position::default();
        assert_eq!(default.sibling_count, 0);
        assert_eq!(default.index, 0);
        assert!(!sel.matches_values("Item", None, &classes, State::empty(), &default));

        let only = Selector::parse_compound("Item:only-child").unwrap();
        assert!(!only.matches_values("Item", None, &classes, State::empty(), &default));

        let nth = Selector::parse_compound("Item:nth-child(1)").unwrap();
        assert!(!nth.matches_values("Item", None, &classes, State::empty(), &default));
    }

    #[test]
    fn structural_matching_via_owned_node() {
        // End-to-end through the public matches() wrapper using with_position.
        let sel = Selector::parse_compound("Item:first-child").unwrap();
        let first = OwnedNode::new("Item").with_position(Position::new(0, 3));
        let second = OwnedNode::new("Item").with_position(Position::new(1, 3));
        assert!(sel.matches(&first));
        assert!(!sel.matches(&second));
    }

    // ---------------------------------------------------------------------
    // Combinator parsing — descendant (`A B`) and child (`A > B`)
    // ---------------------------------------------------------------------

    #[test]
    fn parse_descendant_chain() {
        let s = Selector::parse_chain("Panel Button").unwrap();
        // Subject is Button, ancestor is Panel joined by Descendant.
        assert_eq!(s.type_name.as_deref(), Some("Button"));
        assert!(s.classes.is_empty());
        let (comb, anc) = s.ancestor.as_ref().expect("ancestor present");
        assert_eq!(*comb, Combinator::Descendant);
        assert_eq!(anc.type_name.as_deref(), Some("Panel"));
        assert!(anc.ancestor.is_none());
        // Specificity sums both type compounds: (0, 0, 2).
        assert_eq!(s.specificity(), (0, 0, 2));
    }

    #[test]
    fn parse_child_chain() {
        let s = Selector::parse_chain("Panel > Button").unwrap();
        assert_eq!(s.type_name.as_deref(), Some("Button"));
        let (comb, anc) = s.ancestor.as_ref().expect("ancestor present");
        assert_eq!(*comb, Combinator::Child);
        assert_eq!(anc.type_name.as_deref(), Some("Panel"));
    }

    #[test]
    fn parse_child_chain_with_rich_subject() {
        let s = Selector::parse_chain("Panel > Button.primary#save:focus:nth-child(2n+1)").unwrap();
        // Subject fields.
        assert_eq!(s.type_name.as_deref(), Some("Button"));
        assert_eq!(s.classes, vec!["primary"]);
        assert_eq!(s.id.as_deref(), Some("save"));
        assert_eq!(s.pseudos.len(), 2);
        assert_eq!(s.pseudos[0], Pseudo::State(PseudoClass::Focus));
        assert_eq!(s.pseudos[1], Pseudo::NthChild(NthExpr { a: 2, b: 1 }));
        // Ancestor is Panel joined by Child.
        let (comb, anc) = s.ancestor.as_ref().expect("ancestor present");
        assert_eq!(*comb, Combinator::Child);
        assert_eq!(anc.type_name.as_deref(), Some("Panel"));
        // The rich subject keeps the `2n + 1` interior intact.
    }

    #[test]
    fn parse_three_compound_chain() {
        // A B C — two descendant combinators: subject C, ancestor B (Descendant),
        // B's ancestor A (Descendant).
        let s = Selector::parse_chain("A B C").unwrap();
        assert_eq!(s.type_name.as_deref(), Some("C"));
        let (comb_b, anc_b) = s.ancestor.as_ref().expect("ancestor B");
        assert_eq!(*comb_b, Combinator::Descendant);
        assert_eq!(anc_b.type_name.as_deref(), Some("B"));
        let (comb_a, anc_a) = anc_b.ancestor.as_ref().expect("ancestor A");
        assert_eq!(*comb_a, Combinator::Descendant);
        assert_eq!(anc_a.type_name.as_deref(), Some("A"));
        assert!(anc_a.ancestor.is_none());
    }

    #[test]
    fn parse_combinator_preserves_nth_paren_spaces() {
        // The spaces and `+` inside `:nth-child(2n + 1)` must NOT be treated
        // as combinators. The whole thing is a single compound `Item:nth-child(2n + 1)`
        // joined by Descendant to `List`.
        let s = Selector::parse_chain("List Item:nth-child(2n + 1)").unwrap();
        assert_eq!(s.type_name.as_deref(), Some("Item"));
        assert_eq!(s.pseudos.len(), 1);
        assert_eq!(s.pseudos[0], Pseudo::NthChild(NthExpr { a: 2, b: 1 }));
        let (comb, anc) = s.ancestor.as_ref().expect("ancestor");
        assert_eq!(*comb, Combinator::Descendant);
        assert_eq!(anc.type_name.as_deref(), Some("List"));
    }

    #[test]
    fn parse_adjacent_sibling_chain() {
        // `Label + Input` — subject is Input, ancestor Label joined by Adjacent.
        let s = Selector::parse_chain("Label + Input").unwrap();
        assert_eq!(s.type_name.as_deref(), Some("Input"));
        let (comb, anc) = s.ancestor.as_ref().expect("ancestor present");
        assert_eq!(*comb, Combinator::Adjacent);
        assert_eq!(anc.type_name.as_deref(), Some("Label"));
        assert!(anc.ancestor.is_none());
        // Specificity sums both type compounds: (0, 0, 2) — same as the
        // descendant chain `Label Input`.
        assert_eq!(s.specificity(), (0, 0, 2));
    }

    #[test]
    fn parse_general_sibling_chain() {
        let s = Selector::parse_chain("Label ~ Input").unwrap();
        assert_eq!(s.type_name.as_deref(), Some("Input"));
        let (comb, anc) = s.ancestor.as_ref().expect("ancestor present");
        assert_eq!(*comb, Combinator::Sibling);
        assert_eq!(anc.type_name.as_deref(), Some("Label"));
    }

    #[test]
    fn sibling_specificity_matches_descendant() {
        // A sibling combinator contributes the same specificity as a descendant
        // joining the same two compounds — combinators themselves carry no
        // specificity weight.
        let desc = Selector::parse_chain("Label Input").unwrap();
        let adj = Selector::parse_chain("Label + Input").unwrap();
        let sib = Selector::parse_chain("Label ~ Input").unwrap();
        assert_eq!(desc.specificity(), adj.specificity());
        assert_eq!(desc.specificity(), sib.specificity());
    }

    #[test]
    fn sibling_combinator_inside_parens_is_not_a_combinator() {
        // `Item:nth-child(2n + 1) + Item` — the inner `+` lives inside the
        // `:nth-child(...)` parens (depth 1) and must be parsed as part of the
        // nth expression, NOT as an adjacent combinator. The OUTER `+` (depth 0)
        // is the adjacent combinator joining the two compounds.
        let s = Selector::parse_chain("Item:nth-child(2n + 1) + Item").unwrap();
        // Subject is the second Item.
        assert_eq!(s.type_name.as_deref(), Some("Item"));
        assert!(s.classes.is_empty());
        // Ancestor is the first compound joined by Adjacent.
        let (comb, anc) = s.ancestor.as_ref().expect("ancestor present");
        assert_eq!(*comb, Combinator::Adjacent);
        assert_eq!(anc.type_name.as_deref(), Some("Item"));
        // The nth expression on the ancestor compound is intact: `2n + 1`.
        assert_eq!(anc.pseudos.len(), 1);
        assert_eq!(anc.pseudos[0], Pseudo::NthChild(NthExpr { a: 2, b: 1 }));
    }

    #[test]
    fn sibling_combinator_with_descendant_ancestor() {
        // `Panel Label + Input` — Input is adjacent to Label, and Label is a
        // descendant of Panel. Subject Input; ancestor (Adjacent) Label;
        // Label's ancestor (Descendant) Panel.
        let s = Selector::parse_chain("Panel Label + Input").unwrap();
        assert_eq!(s.type_name.as_deref(), Some("Input"));
        let (comb_label, anc_label) = s.ancestor.as_ref().expect("ancestor Label");
        assert_eq!(*comb_label, Combinator::Adjacent);
        assert_eq!(anc_label.type_name.as_deref(), Some("Label"));
        let (comb_panel, anc_panel) = anc_label.ancestor.as_ref().expect("ancestor Panel");
        assert_eq!(*comb_panel, Combinator::Descendant);
        assert_eq!(anc_panel.type_name.as_deref(), Some("Panel"));
    }

    // ---------------------------------------------------------------------
    // Sibling-combinator matching against hand-built NodeIdentity lists
    // ---------------------------------------------------------------------

    fn nid(type_name: &str) -> NodeIdentity {
        NodeIdentity {
            type_name: type_name.to_string(),
            id: None,
            classes: Vec::new(),
            state: State::empty(),
            position: Position::default(),
        }
    }

    #[test]
    fn adjacent_matches_when_last_sibling_is_label() {
        let sel = Selector::parse_chain("Label + Input").unwrap();
        let input = nid("Input");
        let label = nid("Label");
        // siblings = [Label] → last is Label → match.
        assert!(sel.matches_chain(&input, &[], std::slice::from_ref(&label)));
    }

    #[test]
    fn adjacent_does_not_match_empty_siblings() {
        let sel = Selector::parse_chain("Label + Input").unwrap();
        let input = nid("Input");
        assert!(!sel.matches_chain(&input, &[], &[]));
    }

    #[test]
    fn adjacent_does_not_match_when_last_sibling_is_not_label() {
        let sel = Selector::parse_chain("Label + Input").unwrap();
        let input = nid("Input");
        let span = nid("Span");
        // siblings = [Span] → last is Span, not Label → no match.
        assert!(!sel.matches_chain(&input, &[], &[span]));
    }

    #[test]
    fn adjacent_checks_only_immediate_sibling() {
        // siblings = [Label, Span] (Label is older, Span is the immediate
        // predecessor) → immediate = Span, not Label → no match.
        let sel = Selector::parse_chain("Label + Input").unwrap();
        let input = nid("Input");
        let label = nid("Label");
        let span = nid("Span");
        assert!(!sel.matches_chain(&input, &[], &[label, span]));
    }

    #[test]
    fn general_sibling_matches_when_some_prior_sibling_is_label() {
        let sel = Selector::parse_chain("Label ~ Input").unwrap();
        let input = nid("Input");
        let label = nid("Label");
        let span = nid("Span");
        // [Label, Span] — Label is among prior siblings → match.
        assert!(sel.matches_chain(&input, &[], &[label.clone(), span.clone()]));
        // [Span, Label] — Label is among prior siblings → match.
        assert!(sel.matches_chain(&input, &[], &[span, label]));
    }

    #[test]
    fn general_sibling_does_not_match_when_no_label_among_prior() {
        let sel = Selector::parse_chain("Label ~ Input").unwrap();
        let input = nid("Input");
        let span = nid("Span");
        assert!(!sel.matches_chain(&input, &[], &[span]));
        assert!(!sel.matches_chain(&input, &[], &[]));
    }

    #[test]
    fn specificity_of_child_chain_sums() {
        // Panel > Button.primary: ids 0, classes+pseudos 1, types 2 → (0,1,2).
        let s = Selector::parse_chain("Panel > Button.primary").unwrap();
        assert_eq!(s.specificity(), (0, 1, 2));
    }

    #[test]
    fn comma_list_with_combinators() {
        let list = Selector::parse_list("Panel Button, .modal > Button").unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].type_name.as_deref(), Some("Button"));
        assert_eq!(list[1].type_name.as_deref(), Some("Button"));
        // first is descendant, second is child.
        let (c0, _) = list[0].ancestor.as_ref().expect("anc0");
        assert_eq!(*c0, Combinator::Descendant);
        let (c1, _) = list[1].ancestor.as_ref().expect("anc1");
        assert_eq!(*c1, Combinator::Child);
    }

    // ---------------------------------------------------------------------
    // P5-1: nested sibling chains (`A + B + C`, `A ~ B ~ C`, mixed)
    // ---------------------------------------------------------------------

    #[test]
    fn nested_adjacent_chain_matches() {
        // `Label + Input + Button` — Button whose immediate previous sibling is
        // Input, whose immediate previous sibling is Label. Subject = Button.
        let sel = Selector::parse_chain("Label + Input + Button").unwrap();
        assert_eq!(sel.type_name.as_deref(), Some("Button"));
        let label = nid("Label");
        let input = nid("Input");
        let button = nid("Button");
        // siblings for Button = [Label, Input] (oldest-first, closest = last).
        // Input's own previous siblings = [Label].
        assert!(sel.matches_chain(&button, &[], &[label.clone(), input.clone()]));

        // Broken chain: Button's immediate prev is Input, but Input's prev is NOT
        // Label (it's Span). siblings = [Span, Input] → no match.
        let span = nid("Span");
        assert!(!sel.matches_chain(&button, &[], &[span, input]));

        // Broken chain: missing the second hop entirely (siblings = [Label]).
        assert!(!sel.matches_chain(&button, &[], std::slice::from_ref(&label)));
    }

    #[test]
    fn nested_general_sibling_chain() {
        // `A ~ B ~ C` — C has some prior sibling B, which has some prior sibling A.
        let sel = Selector::parse_chain("A ~ B ~ C").unwrap();
        assert_eq!(sel.type_name.as_deref(), Some("C"));
        let a = nid("A");
        let b = nid("B");
        let c = nid("C");
        // siblings for C = [A, X, B] (B somewhere before C; A before B).
        let x = nid("X");
        assert!(sel.matches_chain(&c, &[], &[a.clone(), x.clone(), b.clone()]));

        // siblings = [B, A] (A is AFTER B, so B has no prior sibling A) → no match.
        assert!(!sel.matches_chain(&c, &[], &[b, a.clone()]));

        // No B at all before C.
        assert!(!sel.matches_chain(&c, &[], &[a, x]));
    }

    #[test]
    fn mixed_adjacent_then_general_sibling_chain() {
        // `A + B ~ C` — C has some prior sibling B; B's immediate prior is A.
        let sel = Selector::parse_chain("A + B ~ C").unwrap();
        assert_eq!(sel.type_name.as_deref(), Some("C"));
        let a = nid("A");
        let b = nid("B");
        let c = nid("C");
        let x = nid("X");
        // siblings for C = [A, B, X] → B (with A immediately before it) is a
        // prior sibling of C → match.
        assert!(sel.matches_chain(&c, &[], &[a.clone(), b.clone(), x.clone()]));
        // siblings = [X, A, B] → B is found, B's immediate prev is A → match.
        assert!(sel.matches_chain(&c, &[], &[x, a, b]));
        // B present but its immediate prev is X (not A): siblings = [X, B].
        let x2 = nid("X");
        let b2 = nid("B");
        assert!(!sel.matches_chain(&c, &[], &[x2, b2]));
    }

    #[test]
    fn mixed_sibling_and_descendant() {
        // `Panel Item + Item` — an Item that immediately follows an Item, both
        // descendants of Panel. Subject = the second Item.
        let sel = Selector::parse_chain("Panel Item + Item").unwrap();
        assert_eq!(sel.type_name.as_deref(), Some("Item"));
        // The ancestor of the second Item is the first Item joined by Adjacent;
        // the first Item's ancestor is Panel joined by Descendant.
        let panel = nid("Panel");
        let item1 = nid("Item");
        let item2 = nid("Item");
        // ancestors for item2 = [Panel]; siblings = [Item].
        assert!(sel.matches_chain(&item2, std::slice::from_ref(&panel), std::slice::from_ref(&item1)));
        // Wrong ancestor type → no match.
        let other = nid("Other");
        assert!(!sel.matches_chain(&item2, std::slice::from_ref(&other), &[item1]));
    }

    #[test]
    fn single_adjacent_still_matches() {
        // Regression: the P4-2 single-level adjacent still works after the
        // sibling-prefix refactor.
        let sel = Selector::parse_chain("Label + Input").unwrap();
        let label = nid("Label");
        let input = nid("Input");
        assert!(sel.matches_chain(&input, &[], std::slice::from_ref(&label)));
        // Empty siblings → no match.
        assert!(!sel.matches_chain(&input, &[], &[]));
    }

    // ---------------------------------------------------------------------
    // P5-5: :nth-of-type and :first-of-type
    // ---------------------------------------------------------------------

    #[test]
    fn parse_nth_of_type() {
        let s = Selector::parse_compound("Item:nth-of-type(2n+1)").unwrap();
        assert_eq!(s.type_name.as_deref(), Some("Item"));
        assert_eq!(s.pseudos.len(), 1);
        assert_eq!(s.pseudos[0], Pseudo::NthOfType(NthExpr { a: 2, b: 1 }));
    }

    #[test]
    fn parse_first_of_type() {
        let s = Selector::parse_compound("Item:first-of-type").unwrap();
        assert_eq!(s.pseudos, vec![Pseudo::FirstOfType]);
    }

    #[test]
    fn nth_of_type_counts_same_type_only() {
        // Full sibling order in the parent: [Div, Item, Div, Item].
        // For the 2nd Item as subject, its PREVIOUS siblings are [Div, Item, Div]
        // — exactly one Item before it → of_type_index = 2.
        let div = nid("Div");
        let item = nid("Item");
        let second_item = nid("Item");
        let siblings = [div.clone(), item.clone(), div.clone()];

        let nth2 = Selector::parse_compound("Item:nth-of-type(2)").unwrap();
        assert!(nth2.matches_chain(&second_item, &[], &siblings));
        let nth1 = Selector::parse_compound("Item:nth-of-type(1)").unwrap();
        assert!(!nth1.matches_chain(&second_item, &[], &siblings));

        // A Div subject with one prior Div in its previous siblings
        // ([Div, Item]) → of_type_index 2; `Div:nth-of-type(2)` matches,
        // `Div:nth-of-type(1)` does not.
        let second_div = nid("Div");
        let div_siblings = [div.clone(), item.clone()];
        let div_nth1 = Selector::parse_compound("Div:nth-of-type(1)").unwrap();
        let div_nth2 = Selector::parse_compound("Div:nth-of-type(2)").unwrap();
        assert!(!div_nth1.matches_chain(&second_div, &[], &div_siblings));
        assert!(div_nth2.matches_chain(&second_div, &[], &div_siblings));

        // First Div (no previous same-type siblings) → of_type_index 1.
        let first_div = nid("Div");
        let first_siblings = [item.clone(), item.clone()]; // no Div before it
        assert!(div_nth1.matches_chain(&first_div, &[], &first_siblings));
    }

    #[test]
    fn first_of_type_matches_first_same_type() {
        let item = nid("Item");
        let div = nid("Div");
        let second_item = nid("Item");

        let sel = Selector::parse_compound("Item:first-of-type").unwrap();

        // No previous same-type sibling → match.
        assert!(sel.matches_chain(&item, &[], std::slice::from_ref(&div)));
        // One previous Item → no match.
        assert!(!sel.matches_chain(&second_item, &[], std::slice::from_ref(&item)));
        // Previous siblings of other types don't count.
        assert!(sel.matches_chain(&item, &[], &[div.clone(), div.clone()]));
    }

    #[test]
    fn nth_of_type_specificity_counts_as_one() {
        let s = Selector::parse_compound(":nth-of-type(2n+1)").unwrap();
        assert_eq!(s.specificity(), (0, 1, 0));
        let s2 = Selector::parse_compound(":first-of-type").unwrap();
        assert_eq!(s2.specificity(), (0, 1, 0));
    }

    #[test]
    fn nth_of_type_does_not_match_on_oneshot_path() {
        // The one-shot matches_values path has no sibling context: same_type_before
        // = 0, so :nth-of-type(2) does NOT match (it would need index 2).
        let nth2 = Selector::parse_compound("Item:nth-of-type(2)").unwrap();
        let classes = Classes::from_slice(&[]);
        assert!(!nth2.matches_values("Item", None, &classes, State::empty(), &pos(1, 3)));
        // :first-of-type / :nth-of-type(1) trivially match on the one-shot path.
        let first = Selector::parse_compound("Item:first-of-type").unwrap();
        assert!(first.matches_values("Item", None, &classes, State::empty(), &pos(0, 3)));
        let nth1 = Selector::parse_compound("Item:nth-of-type(1)").unwrap();
        assert!(nth1.matches_values("Item", None, &classes, State::empty(), &pos(0, 3)));
    }
}
