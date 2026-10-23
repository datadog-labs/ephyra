// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! Scenario generation: a random git history (rendered as a bash script that drives the
//! reference git) plus a list of revspecs (an AST rendered once the repo exists, because
//! short-hash leaves need the actual object ids).

use ephyra::Source;
use std::fmt::Write as _;
use std::ops::ControlFlow;

// Small pools so names collide: a branch and a tag share "v1" and "main", "origin" is both a
// branch and the remote, "abcd" looks like a hex prefix.
pub const BRANCHES: &[&str] = &["main", "dev", "topic", "feat/x", "v1", "origin", "abcd"];
pub const TAGS: &[&str] = &["v1", "v2", "main", "t/x"];
pub const PATHS: &[&str] = &["a", "b", "d/c", "d/e/f"];
pub const CONTENTS: &[&str] = &["1", "2", "3", ""];
/// Paths for tree-shape changes: the commit pool plus names that sort around `d/` in git's tree
/// order (`d.x` < `d/` < `d0`) and deeper ones.
pub const SHAPE_PATHS: &[&str] = &["a", "b", "d/c", "d/e/f", "d.x", "d0", "e/a", "d/e", "x/y/z", "d"];
pub const LINK_TARGETS: &[&str] = &["a", "../b", "nowhere", "d", "d/c"];
pub const MSGS: &[&str] = &["init", "fix bug", "add feature", "fix typo", "wip", "feat: more"];
// Regexes for `^{/re}` and `:/re`. `!-x` is git's negation, `!!` an escaped `!`.
pub const REGEXES: &[&str] = &["fix", "add", "bug", "^fix", "feat", "wip", "typo", "nomatch", "!-fix", "f.x", "", "!!x", "^w"];
pub const TREE_PATHS: &[&str] = &["a", "b", "d", "d/c", "d/e", "d/e/f", "", "nope", "d/"];
pub const PEELS: &[&str] = &["commit", "tree", "blob", "tag", "object", ""];

#[derive(Clone, Debug)]
pub enum TagTarget {
    Back(u8),
    Tree,
    Blob(u8),
    OtherTag(u8),
}

#[derive(Clone, Debug)]
pub enum Op {
    Commit { files: Vec<(u8, u8)>, del: Option<u8>, msg: u8 },
    Checkout(u8),
    BranchAt { name: u8, back: u8 },
    Detach { back: u8 },
    Merge { names: Vec<u8>, msg: u8 },
    Tag { name: u8, annotated: bool, target: TagTarget, msg: u8 },
    Reset { back: u8 },
    UpstreamLocal { branch: u8, to: u8 },
    UpstreamRemote { branch: u8, to: u8 },
    RemoteRef { name: u8, back: u8 },
    RemoteHead(u8),
    PushCfg(u8),
    PackRefs,
    Repack,
    DeleteBranch(u8),
    SymRef { alias: u8, to: u8 },
    HexBranch { len: u8, back: u8 },
    UpdateRef { name: u8, back: u8, msg: u8 },
    /// `git commit-graph write` (0: --reachable, 1: --reachable --split, 2: --reachable
    /// --changed-paths, 3: --split=no-merge after another one).
    CommitGraph(u8),
    /// Move the clock back (or keep it) so commit times repeat or go backwards (clock skew).
    ClockBack(u8),
    /// A worktree change of tree shape, staged by the next commit (0: toggle +x, 1: symlink,
    /// 2: `git mv`, 3: replace by a directory, 4: multi-line content, 5: copy, 6: truncate,
    /// 7: commit a gitlink, 8: `rm -rf`).
    Shape { kind: u8, p: u8, q: u8 },
    /// A worktree/index change that is not committed right away (the next commit's `git add -A`
    /// picks up what it can; at the end of the history it is what `#status`/`#index` see), see
    /// `render_dirty`.
    Dirty { kind: u8, p: u8, q: u8 },
}

/// A commit-graph query for the `graph` oracle. Commits are indices into the repo's commit
/// list (resolved when rendering, so the rendered case holds plain object ids).
#[derive(Clone, Debug)]
pub enum Query {
    MergeBase(u16, u16),
    MergeBasesMany(u16, Vec<u16>),
    Octopus(Vec<u16>),
    /// gix `rev_walk` (sorting 0: breadth-first, 1: newest first, 2: oldest first), hidden tips,
    /// first-parent, commit-graph use toggle.
    /// Hidden commits equal to a tip are dropped unless `overlap` (rare: G1-like bugs flood).
    Walk { tips: Vec<u16>, hidden: Vec<u16>, sorting: u8, first_parent: bool, graph: bool, overlap: bool },
    /// gix-traverse `Topo` (0: date order, 1: topo order).
    Topo { tips: Vec<u16>, ends: Vec<u16>, topo: bool, first_parent: bool, overlap: bool },
    /// describe (select 0: annotated tags, 1: all tags, 2: all refs).
    Describe { commit: u16, select: u8, first_parent: bool, candidates: u8 },
    /// Tree diff between two commits (mode 0: no rewrites, 1: exact renames).
    Diff { mode: u8, a: u16, b: u16, parent: bool },
    /// Merge of two commits (`#merge`).
    Merge(u16, u16),
    /// `#status`, `#index`, `#index_rt` (the `wt` oracles).
    Status,
    Index,
    IndexRt,
}

#[derive(Clone, Debug)]
pub enum Base {
    Head,
    At,
    Branch(u8),
    Tag(u8),
    RefsHeads(u8),
    Heads(u8),
    RefsTags(u8),
    Tags(u8),
    Remote(u8),
    Remotes(u8),
    RefsRemotes(u8),
    Origin,
    Prefix { k: u16, len: u8 },
    MsgSearch(u8),
    Empty,
}

#[derive(Clone, Debug)]
pub enum Suffix {
    Tilde(Option<u8>),
    Caret(Option<u8>),
    Peel(u8),
    PeelRe(u8),
    Reflog(u8),
    Prev(u8),
    Upstream(u8),
    Push,
    Date { step: u16, fmt: u8 },
}

#[derive(Clone, Debug)]
pub struct Spec {
    pub base: Base,
    pub suffixes: Vec<Suffix>,
    /// `:path` or `:<stage>:path` (stage only renders when the base is empty).
    pub tail: Option<(Option<u8>, u8)>,
}

#[derive(Clone, Debug)]
pub struct Scenario {
    pub sha256: bool,
    pub ops: Vec<Op>,
    pub specs: Vec<Spec>,
    pub queries: Vec<Query>,
}

pub const T0: u64 = 1_700_000_000;
pub const TICK: u64 = 3600;

fn op(g: &mut Source) -> Op {
    let b = |g: &mut Source| g.int(0u8..=(BRANCHES.len() as u8 - 1));
    let back = |g: &mut Source| *g.weighted(&[(4, 0u8), (3, 1), (2, 2), (1, 3)]);
    match *g.weighted(&[
        (40, 0u8), (8, 1), (5, 2), (3, 3), (6, 4), (8, 5), (2, 6), (3, 7), (4, 8), (4, 9), (1, 10), (2, 11),
        (2, 12), (1, 13), (1, 14), (1, 15), (2, 16), (3, 17), (2, 18), (2, 19), (14, 20), (8, 21),
    ]) {
        0 => {
            let n = g.int(0usize..=3);
            let files = (0..n).map(|_| (g.int(0u8..=3), g.int(0u8..=3))).collect();
            let del = if g.bool_with(0.1) { Some(g.int(0u8..=3)) } else { None };
            Op::Commit { files, del, msg: g.int(0u8..=(MSGS.len() as u8 - 1)) }
        },
        1 => Op::Checkout(b(g)),
        2 => Op::BranchAt { name: b(g), back: back(g) },
        3 => Op::Detach { back: back(g) },
        4 => {
            let n = *g.weighted(&[(4, 1usize), (1, 2)]);
            Op::Merge { names: (0..n).map(|_| b(g)).collect(), msg: g.int(0u8..=5) }
        },
        5 => {
            let target = match *g.weighted(&[(6, 0u8), (1, 1), (1, 2), (2, 3)]) {
                0 => TagTarget::Back(back(g)),
                1 => TagTarget::Tree,
                2 => TagTarget::Blob(g.int(0u8..=3)),
                _ => TagTarget::OtherTag(g.int(0u8..=3)),
            };
            Op::Tag { name: g.int(0u8..=3), annotated: g.bool_with(0.6), target, msg: g.int(0u8..=5) }
        },
        6 => Op::Reset { back: g.int(1u8..=2) },
        7 => Op::UpstreamLocal { branch: b(g), to: b(g) },
        8 => Op::UpstreamRemote { branch: b(g), to: b(g) },
        9 => Op::RemoteRef { name: b(g), back: back(g) },
        10 => Op::RemoteHead(b(g)),
        11 => Op::PushCfg(g.int(0u8..=6)),
        12 => Op::PackRefs,
        13 => Op::Repack,
        14 => Op::DeleteBranch(b(g)),
        15 => Op::SymRef { alias: b(g), to: b(g) },
        16 => Op::HexBranch { len: g.int(4u8..=7), back: back(g) },
        17 => Op::UpdateRef { name: b(g), back: back(g), msg: g.int(0u8..=5) },
        18 => Op::CommitGraph(g.int(0u8..=3)),
        19 => Op::ClockBack(*g.weighted(&[(2, 0u8), (2, 1), (1, 3), (1, 8)])),
        21 => dirty(g),
        _ => Op::Shape {
            kind: *g.weighted(&[(3, 0u8), (2, 1), (5, 2), (2, 3), (3, 4), (2, 5), (1, 6), (1, 7), (1, 8)]),
            p: g.int(0u8..=(SHAPE_PATHS.len() as u8 - 1)),
            q: g.int(0u8..=(SHAPE_PATHS.len() as u8 - 1)),
        },
    }
}

fn at_form(g: &mut Source) -> Suffix {
    match *g.weighted(&[(5, 0u8), (3, 1), (3, 2), (1, 3), (2, 4)]) {
        0 => Suffix::Reflog(*g.weighted(&[(5, 0u8), (5, 1), (3, 2), (2, 3), (1, 9)])),
        1 => Suffix::Prev(g.int(1u8..=3)),
        2 => Suffix::Upstream(g.int(0u8..=3)),
        3 => Suffix::Push,
        _ => Suffix::Date { step: g.int(0u16..=40), fmt: g.int(0u8..=1) },
    }
}

fn nav(g: &mut Source) -> Suffix {
    match *g.weighted(&[(6, 0u8), (6, 1), (3, 2), (2, 3)]) {
        0 => Suffix::Tilde(if g.bool_with(0.2) { None } else { Some(*g.weighted(&[(3, 0u8), (5, 1), (3, 2), (2, 3), (1, 7)])) }),
        1 => Suffix::Caret(if g.bool_with(0.3) { None } else { Some(*g.weighted(&[(2, 0u8), (5, 1), (3, 2), (1, 3)])) }),
        2 => Suffix::Peel(g.int(0u8..=(PEELS.len() as u8 - 1))),
        _ => Suffix::PeelRe(g.int(0u8..=(REGEXES.len() as u8 - 1))),
    }
}

/// `base [@{..}] nav* [:path]`; reflog-like forms go right after a ref name (or alone), and
/// only rarely somewhere else (then both sides should reject them).
fn suffixes(g: &mut Source, base: &Base) -> Vec<Suffix> {
    let mut v = Vec::new();
    let refish = !matches!(base, Base::Prefix { .. } | Base::Tag(_) | Base::RefsTags(_) | Base::Tags(_));
    if matches!(base, Base::Empty) || (refish && g.bool_with(0.35)) || g.bool_with(0.04) {
        v.push(at_form(g));
        if g.bool_with(0.08) {
            v.push(at_form(g));
        }
    }
    let n = *g.weighted(&[(4, 0usize), (5, 1), (3, 2), (1, 3)]);
    for _ in 0..n {
        v.push(nav(g));
    }
    if g.bool_with(0.0) {
        let at = g.index(v.len() + 1);
        v.insert(at, at_form(g));
    }
    v
}

fn spec(g: &mut Source) -> Spec {
    let b = |g: &mut Source| g.int(0u8..=(BRANCHES.len() as u8 - 1));
    let base = match *g.weighted(&[
        (8, 0u8), (2, 1), (8, 2), (5, 3), (2, 4), (1, 5), (1, 6), (1, 7), (4, 8), (1, 9), (1, 10), (1, 11),
        (5, 12), (2, 13), (5, 14),
    ]) {
        0 => Base::Head,
        1 => Base::At,
        2 => Base::Branch(b(g)),
        3 => Base::Tag(g.int(0u8..=3)),
        4 => Base::RefsHeads(b(g)),
        5 => Base::Heads(b(g)),
        6 => Base::RefsTags(g.int(0u8..=3)),
        7 => Base::Tags(g.int(0u8..=3)),
        8 => Base::Remote(b(g)),
        9 => Base::Remotes(b(g)),
        10 => Base::RefsRemotes(b(g)),
        11 => Base::Origin,
        12 => Base::Prefix { k: g.int(0u16..=999), len: *g.weighted(&[(5, 4u8), (3, 5), (2, 7), (1, 12), (1, 40)]) },
        13 => Base::MsgSearch(g.int(0u8..=(REGEXES.len() as u8 - 1))),
        _ => Base::Empty,
    };
    let (suffixes, tail) = match base {
        Base::MsgSearch(_) => (vec![], None),
        _ => {
            let empty = matches!(base, Base::Empty);
            let suffixes = suffixes(g, &base);
            let tail = if g.bool_with(if empty { 0.15 } else { 0.15 }) {
                let stage = if empty && g.bool_with(0.3) { Some(g.int(0u8..=3)) } else { None };
                Some((stage, g.int(0u8..=(TREE_PATHS.len() as u8 - 1))))
            } else {
                None
            };
            // Bare `:path` (index lookup) with an empty base.
            if empty && g.bool_with(0.1) {
                (vec![], Some((if g.bool() { Some(0) } else { None }, g.int(0u8..=(TREE_PATHS.len() as u8 - 1)))))
            } else {
                (suffixes, tail)
            }
        },
    };
    Spec { base, suffixes, tail }
}

fn dirty(g: &mut Source) -> Op {
    Op::Dirty {
        kind: *g.weighted(&[
            (6, 0u8), (5, 1), (4, 2), (3, 3), (2, 4), (3, 5), (2, 6), (2, 7), (2, 8), (2, 9), (2, 10), (1, 11), (3, 12),
            (2, 13), (2, 14), (1, 15), (2, 16), (1, 17), (2, 18),
        ]),
        p: g.int(0u8..=(SHAPE_PATHS.len() as u8 - 1)),
        q: g.int(0u8..=15),
    }
}

fn commits(g: &mut Source, lo: usize, hi: usize) -> Vec<u16> {
    let n = g.int(lo..=hi);
    (0..n).map(|_| g.int(0u16..=999)).collect()
}

fn query(g: &mut Source) -> Query {
    let c = |g: &mut Source| g.int(0u16..=999);
    match *g.weighted(&[(3, 0u8), (2, 1), (1, 2), (3, 3), (3, 4), (3, 5), (5, 6), (4, 7), (4, 8), (2, 9), (2, 10)]) {
        0 => Query::MergeBase(c(g), c(g)),
        1 => Query::MergeBasesMany(c(g), commits(g, 1, 3)),
        2 => Query::Octopus(commits(g, 2, 4)),
        3 => Query::Walk {
            tips: commits(g, 1, 3),
            hidden: commits(g, 0, 2),
            sorting: g.int(0u8..=2),
            first_parent: g.bool_with(0.25),
            graph: g.bool(),
            overlap: g.bool_with(0.004),
        },
        4 => Query::Topo { tips: commits(g, 1, 3), ends: commits(g, 0, 2), topo: g.bool(), first_parent: g.bool_with(0.25), overlap: g.bool_with(0.003) },
        5 => Query::Describe { commit: c(g), select: g.int(0u8..=2), first_parent: g.bool_with(0.25), candidates: *g.weighted(&[(1, 1u8), (1, 2), (4, 10)]) },
        6 => Query::Diff { mode: g.int(0u8..=1), a: c(g), b: c(g), parent: g.bool() },
        7 => Query::Merge(c(g), c(g)),
        8 => Query::Status,
        9 => Query::Index,
        _ => Query::IndexRt,
    }
}

pub fn scenario(g: &mut Source) -> Scenario {
    let sha256 = g.bool_with(0.3);
    let nops = g.int(6usize..=45);
    let mut ops = vec![Op::Commit { files: vec![(0, 0)], del: None, msg: 0 }];
    ops.extend((0..nops).map(|_| op(g)));
    // Half the histories end with uncommitted worktree/index changes.
    if g.bool() {
        let n = g.int(1usize..=4);
        ops.extend((0..n).map(|_| dirty(g)));
    }
    let nspecs = g.int(10usize..=60);
    let specs = (0..nspecs).map(|_| spec(g)).collect();
    let nq = *g.weighted(&[(1, 0usize), (2, 4), (2, 10), (1, 20)]);
    let queries = (0..nq).map(|_| query(g)).collect();
    Scenario { sha256, ops, specs, queries }
}

/// Simpler queries: one commit argument fewer, flags off.
fn shrink_query(q: &Query) -> Vec<Query> {
    let drop_each = |v: &Vec<u16>, min: usize| -> Vec<Vec<u16>> {
        if v.len() <= min {
            return vec![];
        }
        (0..v.len()).map(|i| v.iter().enumerate().filter(|(j, _)| *j != i).map(|(_, x)| *x).collect()).collect()
    };
    let mut out = Vec::new();
    match q {
        Query::MergeBase(..) | Query::Describe { .. } | Query::Merge(..) | Query::Status | Query::Index | Query::IndexRt => {},
        Query::Diff { mode, a, b, parent } => {
            if *mode == 1 {
                out.push(Query::Diff { mode: 0, a: *a, b: *b, parent: *parent });
            }
        },
        Query::MergeBasesMany(a, o) => out.extend(drop_each(o, 1).into_iter().map(|o| Query::MergeBasesMany(*a, o))),
        Query::Octopus(v) => out.extend(drop_each(v, 2).into_iter().map(Query::Octopus)),
        Query::Walk { tips, hidden, sorting, first_parent, graph, overlap } => {
            out.extend(drop_each(tips, 1).into_iter().map(|t| Query::Walk { tips: t, hidden: hidden.clone(), sorting: *sorting, first_parent: *first_parent, graph: *graph, overlap: *overlap }));
            out.extend(drop_each(hidden, 0).into_iter().map(|h| Query::Walk { tips: tips.clone(), hidden: h, sorting: *sorting, first_parent: *first_parent, graph: *graph, overlap: *overlap }));
            if *first_parent {
                out.push(Query::Walk { tips: tips.clone(), hidden: hidden.clone(), sorting: *sorting, first_parent: false, graph: *graph, overlap: *overlap });
            }
        },
        Query::Topo { tips, ends, topo, first_parent, overlap } => {
            out.extend(drop_each(tips, 1).into_iter().map(|t| Query::Topo { tips: t, ends: ends.clone(), topo: *topo, first_parent: *first_parent, overlap: *overlap }));
            out.extend(drop_each(ends, 0).into_iter().map(|e| Query::Topo { tips: tips.clone(), ends: e, topo: *topo, first_parent: *first_parent, overlap: *overlap }));
            if *first_parent {
                out.push(Query::Topo { tips: tips.clone(), ends: ends.clone(), topo: *topo, first_parent: false, overlap: *overlap });
            }
        },
    }
    out
}

/// Shrink candidates, most aggressive first: halves of the spec list, single specs, op removal,
/// then spec simplification.
pub fn shrink_scenario(s: &Scenario, visit: &mut dyn FnMut(Scenario) -> ControlFlow<()>) -> ControlFlow<()> {
    let q = s.queries.len();
    if q > 0 {
        if !s.specs.is_empty() {
            visit(Scenario { specs: vec![], ..s.clone() })?;
        }
        if q > 1 {
            for i in 0..q {
                visit(Scenario { queries: vec![s.queries[i].clone()], ..s.clone() })?;
            }
        }
        if !s.specs.is_empty() {
            visit(Scenario { queries: vec![], ..s.clone() })?;
        }
        for (i, q) in s.queries.iter().enumerate() {
            for c in shrink_query(q) {
                let mut queries = s.queries.clone();
                queries[i] = c;
                visit(Scenario { queries, ..s.clone() })?;
            }
        }
    }
    let n = s.specs.len();
    if n > 1 {
        for part in [&s.specs[..n / 2], &s.specs[n / 2..]] {
            visit(Scenario { specs: part.to_vec(), ..s.clone() })?;
        }
        for i in 0..n {
            visit(Scenario { specs: vec![s.specs[i].clone()], ..s.clone() })?;
        }
    }
    let m = s.ops.len();
    if m > 2 {
        let mut chunk = m / 2;
        while chunk >= 1 {
            let mut start = 0;
            while start < m {
                let mut ops = s.ops[..start].to_vec();
                ops.extend_from_slice(&s.ops[(start + chunk).min(m)..]);
                if !ops.is_empty() {
                    visit(Scenario { ops, ..s.clone() })?;
                }
                start += chunk;
            }
            chunk /= 2;
        }
    } else if m == 2 {
        visit(Scenario { ops: vec![s.ops[0].clone()], ..s.clone() })?;
    }
    if s.sha256 {
        visit(Scenario { sha256: false, ..s.clone() })?;
    }
    for (i, sp) in s.specs.iter().enumerate() {
        let mut simpler = Vec::new();
        if sp.tail.is_some() && !(matches!(sp.base, Base::Empty) && sp.suffixes.is_empty()) {
            simpler.push(Spec { tail: None, ..sp.clone() });
        }
        for j in 0..sp.suffixes.len() {
            let mut suffixes = sp.suffixes.clone();
            suffixes.remove(j);
            if !(matches!(sp.base, Base::Empty) && suffixes.is_empty() && sp.tail.is_none()) {
                simpler.push(Spec { suffixes, ..sp.clone() });
            }
        }
        if !matches!(sp.base, Base::Head | Base::Empty | Base::MsgSearch(_)) {
            simpler.push(Spec { base: Base::Head, ..sp.clone() });
        }
        for c in simpler {
            let mut specs = s.specs.clone();
            specs[i] = c;
            visit(Scenario { specs, ..s.clone() })?;
        }
    }
    ControlFlow::Continue(())
}

/// The setup script: bash, run in an empty directory with the isolated git environment.
pub fn render_setup(s: &Scenario) -> String {
    let mut o = String::new();
    let fmt = if s.sha256 { "sha256" } else { "sha1" };
    let _ = writeln!(o, "set +e\nN=0\ntick() {{ N=$((N+1)); export GIT_COMMITTER_DATE=\"$(({T0}+N*{TICK})) +0000\" GIT_AUTHOR_DATE=\"$(({T0}+N*{TICK})) +0000\"; }}");
    let _ = writeln!(o, "git init -q --object-format={fmt} -b main repo && cd repo || exit 99");
    let _ = writeln!(o, "git config remote.origin.url ../origin.git\ngit config remote.origin.fetch '+refs/heads/*:refs/remotes/origin/*'");
    let br = |i: u8| BRANCHES[i as usize % BRANCHES.len()];
    let tg = |i: u8| TAGS[i as usize % TAGS.len()];
    let msg = |i: u8| MSGS[i as usize % MSGS.len()];
    for op in &s.ops {
        match op {
            Op::Commit { files, del, msg: m } => {
                for (p, c) in files {
                    let p = PATHS[*p as usize % PATHS.len()];
                    if let Some((dir, _)) = p.rsplit_once('/') {
                        let _ = write!(o, "mkdir -p {dir}; ");
                    }
                    let _ = writeln!(o, "printf '{}\\n' >> {p}", CONTENTS[*c as usize % CONTENTS.len()]);
                }
                if let Some(d) = del {
                    let _ = writeln!(o, "git rm -q -r --ignore-unmatch -- {}", PATHS[*d as usize % PATHS.len()]);
                }
                let _ = writeln!(o, "tick; git add -A && git commit -q --allow-empty -m '{}'", msg(*m));
            },
            Op::Checkout(b) => {
                let _ = writeln!(o, "tick; git checkout -q '{0}' -- 2>/dev/null || git checkout -q -b '{0}'", br(*b));
            },
            Op::BranchAt { name, back } => {
                let _ = writeln!(o, "tick; git branch -f '{}' HEAD~{back}", br(*name));
            },
            Op::Detach { back } => {
                let _ = writeln!(o, "tick; git checkout -q --detach HEAD~{back}");
            },
            Op::Merge { names, msg: m } => {
                let names: Vec<_> = names.iter().map(|n| format!("'{}'", br(*n))).collect();
                let _ = writeln!(o, "tick; git merge -q --no-ff -s ours -m '{}' {}", msg(*m), names.join(" "));
            },
            Op::Tag { name, annotated, target, msg: m } => {
                let t = match target {
                    TagTarget::Back(b) => format!("HEAD~{b}"),
                    TagTarget::Tree => "'HEAD^{tree}'".into(),
                    TagTarget::Blob(p) => format!("HEAD:{}", PATHS[*p as usize % PATHS.len()]),
                    TagTarget::OtherTag(t) => format!("refs/tags/{}", tg(*t)),
                };
                let a = if *annotated { format!("-a -m '{}'", msg(*m)) } else { String::new() };
                let _ = writeln!(o, "tick; git tag -f {a} '{}' {t}", tg(*name));
            },
            Op::Reset { back } => {
                let _ = writeln!(o, "tick; git reset -q --hard HEAD~{back}");
            },
            Op::UpstreamLocal { branch, to } => {
                let _ = writeln!(o, "git branch -q -u '{}' '{}'", br(*to), br(*branch));
            },
            Op::UpstreamRemote { branch, to } => {
                let _ = writeln!(o, "git branch -q -u 'origin/{}' '{}'", br(*to), br(*branch));
            },
            Op::RemoteRef { name, back } => {
                let _ = writeln!(o, "tick; git update-ref 'refs/remotes/origin/{}' HEAD~{back}", br(*name));
            },
            Op::RemoteHead(b) => {
                let _ = writeln!(o, "git symbolic-ref refs/remotes/origin/HEAD 'refs/remotes/origin/{}'", br(*b));
            },
            Op::PushCfg(v) => {
                let line = match v {
                    0 => "git config push.default current".to_string(),
                    1 => "git config push.default upstream".into(),
                    2 => "git config push.default simple".into(),
                    3 => "git config push.default nothing".into(),
                    4 => "git config push.default matching".into(),
                    5 => "git config remote.pushDefault origin".into(),
                    _ => "git config branch.main.pushRemote origin".into(),
                };
                let _ = writeln!(o, "{line}");
            },
            Op::PackRefs => {
                let _ = writeln!(o, "git pack-refs --all");
            },
            Op::Repack => {
                let _ = writeln!(o, "git repack -adq");
            },
            Op::DeleteBranch(b) => {
                let _ = writeln!(o, "git branch -q -D '{}'", br(*b));
            },
            Op::SymRef { alias, to } => {
                // Mostly to an existing, different branch; sometimes dangling or a self-loop.
                let _ = writeln!(o, "git symbolic-ref 'refs/heads/{}' 'refs/heads/{}'", br(*alias), br(*to));
            },
            Op::HexBranch { len, back } => {
                let _ = writeln!(o, "tick; git branch -f \"$(git rev-parse --short={len} HEAD~1)\" HEAD~{back}");
            },
            Op::UpdateRef { name, back, msg: m } => {
                let _ = writeln!(o, "tick; git update-ref -m '{}' 'refs/heads/{}' HEAD~{back}", msg(*m), br(*name));
            },
            Op::CommitGraph(v) => {
                let args = ["--reachable", "--reachable --split", "--reachable --changed-paths", "--reachable --split=no-merge"][*v as usize % 4];
                let _ = writeln!(o, "git commit-graph write {args} 2>/dev/null");
            },
            Op::Shape { kind, p, q } => {
                let p = SHAPE_PATHS[*p as usize % SHAPE_PATHS.len()];
                let q = SHAPE_PATHS[*q as usize % SHAPE_PATHS.len()];
                let parent = |x: &str| x.rsplit_once('/').map(|(d, _)| format!("mkdir -p {d} 2>/dev/null; ")).unwrap_or_default();
                let line = match kind {
                    0 => format!("[ -f {p} ] && {{ [ -x {p} ] && chmod -x {p} || chmod +x {p}; }}"),
                    1 => format!("rm -rf {p}; {}ln -s {} {p}", parent(p), LINK_TARGETS[(p.len() + q.len()) % LINK_TARGETS.len()]),
                    // Mostly to a fresh name (a rename), sometimes onto an existing path.
                    2 if p.len() % 2 == q.len() % 2 => format!("[ -e {p} ] && {{ {}git mv -k {p} {q}.mv; }}", parent(q)),
                    2 => format!("[ -e {p} ] && {{ {}git mv -k {p} {q}; }}", parent(q)),
                    3 => format!("rm -rf {p}; mkdir -p {p} && printf 'z\\n' > {p}/z"),
                    4 => format!("rm -rf {p}; {}seq 1 {} > {p}", parent(p), 3 + q.len() * 2),
                    5 => format!("[ -f {p} ] && {{ rm -rf {q}; {}cp -p {p} {q}; }}", parent(q)),
                    6 => format!("[ -f {p} ] && : > {p}"),
                    7 => format!("rm -rf {p}; tick; git add -A; git update-index --add --cacheinfo \"160000,$(git rev-parse HEAD),{p}\" && git commit -q -m 'add sub'"),
                    _ => format!("rm -rf {p}"),
                };
                let _ = writeln!(o, "{line}");
            },
            Op::Dirty { kind, p, q } => {
                let _ = writeln!(o, "{}", render_dirty(*kind, *p, *q));
            },
            Op::ClockBack(k) => {
                // The next tick lands k+1 steps back (k = 0: the same time as the last one).
                let _ = writeln!(o, "N=$((N-{}))", *k as u32 + 1);
            },
        }
    }
    o
}

/// Render a spec; `objects` is the sorted list of all object ids in the repo.
pub fn render_spec(sp: &Spec, objects: &[String]) -> String {
    let br = |i: u8| BRANCHES[i as usize % BRANCHES.len()];
    let tg = |i: u8| TAGS[i as usize % TAGS.len()];
    let mut o = match &sp.base {
        Base::Head => "HEAD".to_string(),
        Base::At => "@".into(),
        Base::Branch(b) => br(*b).into(),
        Base::Tag(t) => tg(*t).into(),
        Base::RefsHeads(b) => format!("refs/heads/{}", br(*b)),
        Base::Heads(b) => format!("heads/{}", br(*b)),
        Base::RefsTags(t) => format!("refs/tags/{}", tg(*t)),
        Base::Tags(t) => format!("tags/{}", tg(*t)),
        Base::Remote(b) => format!("origin/{}", br(*b)),
        Base::Remotes(b) => format!("remotes/origin/{}", br(*b)),
        Base::RefsRemotes(b) => format!("refs/remotes/origin/{}", br(*b)),
        Base::Origin => "origin".into(),
        Base::Prefix { k, len } => match objects.len() {
            0 => "0000".into(),
            n => {
                let id = &objects[*k as usize % n];
                id[..(*len as usize).min(id.len())].to_string()
            },
        },
        Base::MsgSearch(r) => format!(":/{}", REGEXES[*r as usize % REGEXES.len()]),
        Base::Empty => String::new(),
    };
    for s in &sp.suffixes {
        match s {
            Suffix::Tilde(None) => o.push('~'),
            Suffix::Tilde(Some(n)) => {
                let _ = write!(o, "~{n}");
            },
            Suffix::Caret(None) => o.push('^'),
            Suffix::Caret(Some(n)) => {
                let _ = write!(o, "^{n}");
            },
            Suffix::Peel(p) => {
                let _ = write!(o, "^{{{}}}", PEELS[*p as usize % PEELS.len()]);
            },
            Suffix::PeelRe(r) => {
                let _ = write!(o, "^{{/{}}}", REGEXES[*r as usize % REGEXES.len()]);
            },
            Suffix::Reflog(n) => {
                let _ = write!(o, "@{{{n}}}");
            },
            Suffix::Prev(n) => {
                let _ = write!(o, "@{{-{n}}}");
            },
            Suffix::Upstream(v) => o.push_str(["@{u}", "@{upstream}", "@{U}", "@{UpStream}"][*v as usize % 4]),
            Suffix::Push => o.push_str("@{push}"),
            Suffix::Date { step, fmt } => {
                // Absolute dates only. Formats git's approxidate accepts but gix-date does not
            // (`2023-11-15.21:23:20`, `... UTC`) are a known gix-date gap, left to a date oracle.
            // Absolute dates only (relative ones depend on "now"); a few minutes off the
                // tick grid so entries are never exactly on the boundary.
                let t = T0 + *step as u64 * TICK + 600;
                let (y, mo, d, h, mi, se) = civil(t);
                let _ = match fmt {
                    0 => write!(o, "@{{{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}:{se:02} +0000}}"),
                    1 => write!(o, "@{{{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{se:02}Z}}"),
                    _ => write!(o, "@{{{y:04}-{mo:02}-{d:02}.{h:02}:{mi:02}:{se:02}.UTC}}"),
                };
            },
        }
    }
    if let Some((stage, p)) = &sp.tail {
        let p = TREE_PATHS[*p as usize % TREE_PATHS.len()];
        match (stage, o.is_empty()) {
            (Some(st), true) => {
                let _ = write!(o, ":{st}:{p}");
            },
            _ => {
                let _ = write!(o, ":{p}");
            },
        }
    }
    o
}

fn civil(t: u64) -> (i64, u32, u32, u64, u64, u64) {
    let days = (t / 86400) as i64;
    let rem = t % 86400;
    // Howard Hinnant's days-to-civil.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d, rem / 3600, rem % 3600 / 60, rem % 60)
}

/// Render a query; `commits` is the sorted list of all commit ids in the repo (empty: no query).
/// Format: `#<kind> <args...>`, words separated by spaces, hidden/end commits prefixed with `^`.
pub fn render_query(q: &Query, commits: &[String]) -> Option<String> {
    if commits.is_empty() {
        return None;
    }
    let c = |k: &u16| commits[*k as usize % commits.len()].clone();
    let list = |v: &[u16], pre: &str| v.iter().map(|k| format!("{pre}{}", c(k))).collect::<Vec<_>>();
    let fp = |b: bool| if b { "fp" } else { "all" };
    let ends_of = |tips: &[u16], ends: &[u16], overlap: bool| {
        let t: Vec<String> = tips.iter().map(c).collect();
        ends.iter().map(c).filter(|e| overlap || !t.contains(e)).map(|e| format!("^{e}")).collect::<Vec<_>>()
    };
    Some(match q {
        Query::MergeBase(a, b) => format!("#merge_base {} {}", c(a), c(b)),
        Query::MergeBasesMany(a, o) => format!("#merge_bases_many {} {}", c(a), list(o, "").join(" ")),
        Query::Octopus(v) => format!("#octopus {}", list(v, "").join(" ")),
        Query::Walk { tips, hidden, sorting, first_parent, graph, overlap } => {
            let s = ["bfs", "newest", "oldest"][*sorting as usize % 3];
            let mut w = vec![format!("#walk {s} {} {}", fp(*first_parent), if *graph { "graph" } else { "nograph" })];
            w.extend(list(tips, ""));
            w.extend(ends_of(tips, hidden, *overlap));
            w.join(" ")
        },
        Query::Topo { tips, ends, topo, first_parent, overlap } => {
            let mut w = vec![format!("#topo {} {}", if *topo { "topo" } else { "date" }, fp(*first_parent))];
            w.extend(list(tips, ""));
            w.extend(ends_of(tips, ends, *overlap));
            w.join(" ")
        },
        Query::Describe { commit, select, first_parent, candidates } => {
            let s = ["annotated", "tags", "all"][*select as usize % 3];
            format!("#describe {s} {} {candidates} {}", fp(*first_parent), c(commit))
        },
        // `parent`: the commit's first parent against it (what a shape op changed); `B~1` is
        // resolved the same way by both sides (a revspec bug would show in the revspec oracle).
        Query::Diff { mode, a, b, parent } => {
            let old = if *parent { format!("{}~1", c(b)) } else { c(a) };
            format!("#diff {} {old} {}", ["nore", "exact"][*mode as usize % 2], c(b))
        },
        Query::Merge(a, b) => format!("#merge {} {}", c(a), c(b)),
        Query::Status => "#status".into(),
        Query::Index => "#index".into(),
        Query::IndexRt => "#index_rt".into(),
    })
}

/// Uncommitted worktree/index changes (p: a path of `SHAPE_PATHS`, q: a variant selector).
fn render_dirty(kind: u8, p: u8, q: u8) -> String {
    let p = SHAPE_PATHS[p as usize % SHAPE_PATHS.len()];
    let parent = |x: &str| x.rsplit_once('/').map(|(d, _)| format!("mkdir -p {d} 2>/dev/null; ")).unwrap_or_default();
    let br = BRANCHES[q as usize % BRANCHES.len()];
    match kind {
        // Unstaged edit; q odd: same size (racy-git: same second, same length as the index entry).
        0 if q % 2 == 1 => format!("[ -f {p} ] && [ ! -L {p} ] && {{ tr '0-9a-z' '1-9a-z0' < {p} > {p}.tmp && cat {p}.tmp > {p}; rm -f {p}.tmp; }}"),
        0 => format!("[ -f {p} ] && printf 'w{q}\\n' >> {p}"),
        // Staged edit; q odd: edited again after staging.
        1 if q % 2 == 1 => format!("[ -f {p} ] && {{ printf 's\\n' >> {p}; git add -- {p}; printf 't\\n' >> {p}; }}"),
        1 => format!("{}printf 's{q}\\n' >> {p}; git add -- {p}", parent(p)),
        // Untracked files (also in new directories, also next to ignored ones).
        2 => match q % 4 {
            0 => format!("{}printf 'u\\n' > {p}.new", parent(p)),
            1 => format!("mkdir -p {p}.dir/sub 2>/dev/null && printf 'u\\n' > {p}.dir/sub/f"),
            2 => format!("[ -d {p} ] && printf 'u\\n' > {p}/new"),
            _ => format!("mkdir -p {p}.empty 2>/dev/null"),
        },
        // Intent-to-add.
        3 if q % 4 == 1 => format!("[ -f {p} ] && {{ git rm -q --cached -- {p}; git add -N -- {p}; }}"),
        3 => format!("{}printf 'i\\n' > {p}.ita; git add -N -- {p}.ita", parent(p)),
        4 => format!("git rm -q -r --cached --ignore-unmatch -- {p}"),
        // Gone from the worktree only; q odd: replaced by a directory / by a symlink.
        5 => match q % 3 {
            0 => format!("rm -rf {p}"),
            1 => format!("rm -rf {p}; mkdir -p {p} && printf 'z\\n' > {p}/z"),
            _ => format!("rm -rf {p}; {}ln -s nowhere {p}", parent(p)),
        },
        // Index bits; q odd: the file changes afterwards.
        6 => format!("git update-index -q --skip-worktree -- {p}{}", if q % 2 == 1 { format!("; [ -f {p} ] && printf 'k\\n' >> {p}") } else if q % 4 == 2 { format!("; rm -f {p}") } else { String::new() }),
        7 => format!("git update-index -q --assume-unchanged -- {p}{}", if q % 2 == 1 { format!("; [ -f {p} ] && printf 'k\\n' >> {p}") } else { String::new() }),
        8 => format!("git update-index -q --no-skip-worktree --no-assume-unchanged -- {p}"),
        // Ignore rules and ignored files.
        9 => match q % 4 {
            0 => format!("printf '*.new\\n' >> .gitignore; {}printf 'g\\n' > {p}.new", parent(p)),
            1 => format!("printf '{p}\\n' >> .gitignore"),
            2 => format!("printf '/{p}.dir/\\n!/{p}.dir/keep\\n' >> .gitignore; mkdir -p {p}.dir 2>/dev/null; printf 'g\\n' > {p}.dir/keep; printf 'g\\n' > {p}.dir/drop"),
            _ => format!("printf '*.ita\\n*.empty\\n' > .git/info/exclude"),
        },
        // Index format: version, split index, untracked cache, refresh.
        10 => format!("git update-index --index-version {}", 2 + q % 3),
        11 => {
            if q % 3 == 0 { "git update-index --no-split-index".to_string() } else { "git -c splitIndex.maxPercentChange=100 update-index --split-index".into() }
        },
        12 => match q % 3 {
            0 => "git update-index --force-untracked-cache; git status -uall >/dev/null".to_string(),
            1 => "git update-index -q --refresh".into(),
            _ => "git status >/dev/null".into(),
        },
        // A real merge (default strategy), which may stop with conflicts (unmerged stages,
        // MERGE_HEAD); the next commit concludes it.
        13 => format!("tick; git merge -q --no-ff -m 'merge {br}' '{br}' >/dev/null 2>&1"),
        // Staged mode change, worktree untouched.
        14 => format!("[ -f {p} ] && git update-index --chmod={} -- {p}", if q % 2 == 0 { "+x" } else { "-x" }),
        // A nested repository, untracked.
        15 => format!("git init -q {p}.repo{}", if q % 2 == 1 { format!(" && git -C {p}.repo commit -q --allow-empty -m nested") } else { String::new() }),
        // Unstage.
        16 => format!("git reset -q -- {p}"),
        // Line-ending conversion on.
        17 => match q % 3 {
            0 => format!("git config core.autocrlf true; printf 'c\\r\\nd\\r\\n' > {p}.crlf"),
            1 => format!("printf '*.crlf text eol=crlf\\n' >> .gitattributes; printf 'c\\nd\\n' > {p}.crlf"),
            _ => "git config core.autocrlf input".into(),
        },
        // Unmerged entries by hand: three stages for one path.
        _ => format!("[ -f {p} ] && {{ h=$(git hash-object -w {p}); z=$(echo $h | tr 0-9a-f 0); printf '0 %s\\t{p}\\n100644 %s 1\\t{p}\\n100644 %s {}\\t{p}\\n' $z $h $h | git update-index --index-info; }}", 2 + q % 2),
    }
}
