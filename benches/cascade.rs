//! Performance benchmarks for the cascade hot path.
//!
//! These substantiate the crate's perf claims and act as a regression guard:
//!   - `compute_owned_vs_noderef` — OwnedNode allocates a Vec per `classes()`;
//!     NodeRef does not.
//!   - `compute_vs_compute_with` — `compute` allocates a fresh `ComputeScratch`
//!     each call; `compute_with` reuses one across the draw loop.
//!   - `with_parent_inheritance` — cost of the inheritance path
//!     (`resolve_explicit_inherit` + `inherit_from`).
//!   - `var_resolution` — the `resolve_vars_in_place` path for `var(--token)`
//!     consumers.
//!   - `cascade_context_enter_leave` — a realistic Root→Panel→Text tree walk
//!     via `CascadeContext::enter`, including the `.clone()` onto the internal
//!     stack.
//!
//! The medium stylesheet is built once via `LazyLock` (parsing is NOT measured),
//! and all node data is `&'static str` so NodeRef is genuinely zero-allocation.

use std::sync::LazyLock;

use criterion::{black_box, criterion_group, criterion_main, Criterion};

use ratatui_style::{
    CascadeContext, ComputeScratch, NodeRef, Origin, OwnedNode, State, Stylesheet,
};

// ---------------------------------------------------------------------------
// Medium stylesheet — ~70 rules, built once.
// ---------------------------------------------------------------------------
//
// Realistic variety: `:root` custom properties, type rules, class rules, an id
// rule, pseudo-state rules (`:focus`/`:disabled`), and several `var(--token)`
// consumers. Mostly `Origin::User` (the `parse` default), a couple `Origin::
// Theme`, one `Origin::Inline`.

static SHEET: LazyLock<Stylesheet> = LazyLock::new(build_sheet);

fn build_sheet() -> Stylesheet {
    // Base CSS: custom properties + the bulk of the rules at Origin::User.
    let base = r#"
        :root {
            --bg:        #1e1e2e;
            --fg:        #cdd6f4;
            --muted:     #6c7086;
            --accent:    #89b4fa;
            --danger:    #f38ba8;
            --border:    #45475a;
            --width:     50%;
        }

        Button {
            color: var(--fg);
            background: var(--bg);
            padding: 0 1;
            border: rounded var(--border);
        }
        Text {
            color: var(--fg);
        }
        Label {
            color: var(--muted);
        }
        Panel {
            border: rounded var(--border);
            padding: 1;
        }
        List {
            color: var(--fg);
        }
        ListItem {
            color: var(--muted);
        }
        Input {
            color: var(--fg);
            background: var(--bg);
            border: rounded var(--border);
        }
        Div {
            color: inherit;
        }
        Span { color: var(--fg); }
        Header { color: var(--accent); bold: true; }
        Footer { color: var(--muted); }
        Scrollbar { color: var(--border); }

        .primary {
            background: var(--accent);
            color: #1e1e2e;
        }
        .muted { color: var(--muted); }
        .active { color: var(--accent); bold: true; }
        .danger { color: var(--danger); }
        .compact { padding: 0; }
        .wide { padding: 0 2; }
        .hidden { color: var(--muted); }
        .selected { background: var(--accent); color: var(--bg); }
        .focused { border: rounded var(--accent); }
        .disabled-state { color: var(--muted); }

        Button:focus { border: rounded var(--accent); }
        Button:disabled { color: var(--muted); }
        Input:focus { border: rounded var(--accent); }
        Input:disabled { color: var(--muted); }
        ListItem:active { color: var(--accent); }
        List:focus { border: rounded var(--accent); }
        Button:active { background: var(--accent); }
        Text:hover { color: var(--accent); }
        Span:hover { color: var(--accent); }

        #save {
            color: #1e1e2e;
            background: var(--accent);
            bold: true;
        }
        #cancel { color: var(--muted); }
        #main-panel { padding: 2; width: var(--width); }
        #status { color: var(--accent); }

        Button.primary.active { bold: true; underline: true; }
        Panel.compact { padding: 0; }
        Input.danger { border: rounded var(--danger); }
        List.muted { color: var(--muted); }
        Text.danger { color: var(--danger); }
        Span.muted { color: var(--muted); }
        Div.wide { padding: 0 2; }
        Header.accent { color: var(--accent); }
        Footer.muted { color: var(--muted); }

        Button:focus.primary { background: var(--accent); }
        ListItem:checked { background: var(--accent); color: var(--bg); }
        Input:focus.danger { border: rounded var(--danger); }
    "#;

    let mut sheet = Stylesheet::parse(base).expect("base CSS parses");

    // A couple of Theme-origin rules (origin lower priority than User).
    let theme = Stylesheet::parse_with_origin(
        r#"
        Button { color: #cdd6f4; }
        Text   { color: #cdd6f4; }
        Panel  { border: rounded #45475a; }
        "#,
        Origin::Theme,
    )
    .expect("theme CSS parses");
    sheet.extend(&theme);

    // One Inline-origin rule (highest priority, applied last).
    sheet
        .add(
            "Button#save",
            ratatui_style::CssStyle::new().bold(),
            Origin::Inline,
        )
        .expect("inline rule adds");

    debug_assert!(sheet.rules().len() >= 60, "medium sheet has enough rules");
    sheet
}

// ---------------------------------------------------------------------------
// Shared &'static str node data — NodeRef is genuinely zero-allocation.
// ---------------------------------------------------------------------------

const BTN_TYPE: &str = "Button";
const BTN_ID: &str = "save";
const BTN_CLASSES: &[&str] = &["primary", "active"];
const TXT_TYPE: &str = "Text";
const TXT_CLASSES: &[&str] = &["muted", "accented"];
const PANEL_TYPE: &str = "Panel";
const ROOT_TYPE: &str = "Root";

// A NodeRef mirroring the OwnedNode used in the OwnedNode-vs-NodeRef group.
fn button_noderef() -> NodeRef<'static> {
    NodeRef::new(BTN_TYPE)
        .id(BTN_ID)
        .classes(BTN_CLASSES)
        .state(State::focus())
}

fn button_owned() -> OwnedNode {
    OwnedNode::new(BTN_TYPE)
        .with_id(BTN_ID)
        .with_classes(BTN_CLASSES.iter().copied())
        .with_state(State::focus())
}

// ---------------------------------------------------------------------------
// Benchmark groups
// ---------------------------------------------------------------------------

fn compute_owned_vs_noderef(c: &mut Criterion) {
    let sheet = &*SHEET;
    let owned = button_owned();
    let noderef = button_noderef();

    let mut g = c.benchmark_group("compute_owned_vs_noderef");
    g.sample_size(60);
    g.bench_function("owned_node", |b| {
        b.iter(|| black_box(sheet.compute(black_box(&owned), None)))
    });
    g.bench_function("noderef", |b| {
        b.iter(|| black_box(sheet.compute(black_box(&noderef), None)))
    });
    g.finish();
}

fn compute_vs_compute_with(c: &mut Criterion) {
    let sheet = &*SHEET;
    let noderef = button_noderef();

    let mut g = c.benchmark_group("compute_vs_compute_with");
    g.sample_size(60);

    // `compute` allocates a fresh ComputeScratch each call.
    g.bench_function("compute", |b| {
        b.iter(|| black_box(sheet.compute(black_box(&noderef), None)))
    });

    // `compute_with` reuses a single scratch held outside the measured closure,
    // so its one-time allocation is not measured.
    let mut scratch = ComputeScratch::new();
    g.bench_function("compute_with", |b| {
        b.iter(|| black_box(sheet.compute_with(black_box(&noderef), None, black_box(&mut scratch))))
    });

    g.finish();
}

fn with_parent_inheritance(c: &mut Criterion) {
    let sheet = &*SHEET;
    let noderef = button_noderef();

    // Pre-compute a parent so its construction is outside the measured loop.
    let parent = sheet.compute(&button_owned(), None);

    let mut g = c.benchmark_group("with_parent_inheritance");
    g.sample_size(60);
    let mut scratch = ComputeScratch::new();
    g.bench_function("inherit_from_parent", |b| {
        b.iter(|| {
            black_box(sheet.compute_with(
                black_box(&noderef),
                Some(black_box(&parent)),
                black_box(&mut scratch),
            ))
        })
    });
    g.finish();
}

fn var_resolution(c: &mut Criterion) {
    // A node whose matched rules include var(--token) consumers. The base sheet
    // already has many var() consumers (Button, .primary, etc.); we additionally
    // exercise a `.accented` class-style consumer and a border-color var via a
    // dedicated sheet so the resolve_vars_in_place border path is hit too.
    //
    // Built once, outside the measured loop.
    let sheet = &*SHEET;

    // `.accented` is not in the base sheet; use a node whose matched rules are
    // rich in var() references: a Button.primary.active — Button alone consumes
    // 4+ vars (fg, bg, padding-x, padding-y, border).
    let noderef = NodeRef::new(BTN_TYPE)
        .classes(&["primary", "active"])
        .state(State::focus());

    let mut g = c.benchmark_group("var_resolution");
    g.sample_size(60);
    let mut scratch = ComputeScratch::new();
    g.bench_function("resolve_var_consumers", |b| {
        b.iter(|| black_box(sheet.compute_with(black_box(&noderef), None, black_box(&mut scratch))))
    });

    // A second variant: a Text node that only matches the type rule + a var()
    // class consumer, isolating the var path from the heavier Button path.
    let txt = NodeRef::new(TXT_TYPE).classes(TXT_CLASSES);
    g.bench_function("resolve_var_text", |b| {
        b.iter(|| black_box(sheet.compute_with(black_box(&txt), None, black_box(&mut scratch))))
    });

    g.finish();
}

fn cascade_context_enter_leave(c: &mut Criterion) {
    let sheet = &*SHEET;

    // 3-level tree as &'static str NodeRefs: Root → Panel → Text.
    let root = NodeRef::new(ROOT_TYPE);
    let panel = NodeRef::new(PANEL_TYPE);
    let text = NodeRef::new(TXT_TYPE);

    let mut g = c.benchmark_group("cascade_context_enter_leave");
    g.sample_size(60);

    // Full enter×3 / leave×3 cycle — the realistic per-component cost, including
    // the internal computed.clone() pushed onto the stack at each enter.
    g.bench_function("enter_3_leave_3", |b| {
        b.iter_with_setup(
            || CascadeContext::new(sheet),
            |mut ctx| {
                let _r = ctx.enter(black_box(&root));
                let _p = ctx.enter(black_box(&panel));
                let _t = ctx.enter(black_box(&text));
                ctx.leave();
                ctx.leave();
                ctx.leave();
                ctx
            },
        )
    });

    // Isolate just `enter` at the leaf (deepest node, parent inheritance active)
    // so the cost of a single enter-with-inherit is visible without the setup
    // noise of building the context. The context is rebuilt per iteration so the
    // stack starts empty and the measurement covers enter-root + enter-panel +
    // enter-text (the leaf enter is what we care about, but it requires the
    // ancestors on the stack first).
    g.bench_function("enter_leaf_with_ancestors", |b| {
        b.iter_with_setup(
            || {
                let mut ctx = CascadeContext::new(sheet);
                ctx.enter(&root);
                ctx.enter(&panel);
                ctx
            },
            |mut ctx| black_box(ctx.enter(black_box(&text))),
        )
    });

    g.finish();
}

// ---------------------------------------------------------------------------
// Custom Criterion config: larger sample size for fast-op stability.
// ---------------------------------------------------------------------------

criterion_group! {
    name = benches;
    config = Criterion::default().sample_size(60);
    targets =
        compute_owned_vs_noderef,
        compute_vs_compute_with,
        with_parent_inheritance,
        var_resolution,
        cascade_context_enter_leave,
}

criterion_main!(benches);
