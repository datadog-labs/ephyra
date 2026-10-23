// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! The `graph` oracle: commit-graph algorithms of gix against the reference git on the same
//! random repositories as the revspec oracle.
//!
//! Queries are rendered into the case's spec list as `#<kind> <words...>` (see
//! `gen::render_query`); all commits in them are full object ids.
//!
//! | query | gix | git (answer of record) |
//! |---|---|---|
//! | `#merge_base A B` | `Repository::merge_base` | `git merge-base --all A B` (gix's one answer must be among them) |
//! | `#merge_bases_many A B..` | `Repository::merge_bases_many` | `git merge-base --all A B..` (same set) |
//! | `#octopus A B..` | `Repository::merge_base_octopus` | `git merge-base --octopus --all A B..` (one of them) |
//! | `#walk <sort> <fp> <graph> T.. ^H..` | `Repository::rev_walk(T).with_hidden(H)` | `git rev-list [--first-parent] T.. ^H..` (same set) |
//! | `#topo <topo|date> <fp> T.. ^E..` | `gix_traverse::commit::topo` | `git rev-list --topo-order|--date-order` (same order) |
//! | `#describe <sel> <fp> <n> C` | `Commit::describe()` | `git describe [--tags|--all] --candidates=n --long` (same name and depth) |

/// gix's or git's answer, canonical text: a list of object ids (one per line) or `name depth`.
pub type Ans = Result<Vec<String>, String>;

/// git's describe answer used a tag object's internal name (see `git_answer`).
pub const EXTERNAL_NAME: &str = "?externally-known";

pub struct Query<'a> {
    pub kind: &'a str,
    pub words: Vec<&'a str>,
}

pub fn parse(q: &str) -> Option<Query<'_>> {
    let mut it = q.strip_prefix('#')?.split(' ');
    let kind = it.next()?;
    Some(Query { kind, words: it.collect() })
}

fn oid(s: &str) -> Result<gix::ObjectId, String> {
    gix::ObjectId::from_hex(s.trim_start_matches('^').as_bytes()).map_err(|e| e.to_string())
}

fn split_tips<'a>(words: &[&'a str]) -> Result<(Vec<gix::ObjectId>, Vec<gix::ObjectId>), String> {
    let mut tips = Vec::new();
    let mut hidden = Vec::new();
    for w in words {
        if w.starts_with('^') { hidden.push(oid(w)?) } else { tips.push(oid(w)?) }
    }
    Ok((tips, hidden))
}

/// Signature area of a query (`graph__<kind>__<area>`).
pub fn area(q: &Query<'_>) -> String {
    match q.kind {
        "walk" => {
            let fp = q.words.get(1) == Some(&"fp");
            if fp { "revwalk_first_parent".into() } else { "revwalk".into() }
        },
        "topo" => format!("{}_order", q.words.first().copied().unwrap_or("topo")),
        "describe" => format!("describe_{}", q.words.first().copied().unwrap_or("x")),
        k => k.to_string(),
    }
}

/// Signature of a disagreement on a query.
pub fn signature(kind: &str, q: &Query<'_>, gix: &Ans, git: &Ans) -> String {
    match q.kind {
        "diff" => format!("treediff__{kind}__{}", crate::treediff::area(q, gix, git)),
        "merge" => {
            let clean = |a: &Ans| a.as_ref().is_ok_and(|v| v.first().is_some_and(|l| l.starts_with("clean")));
            let verdict = |a: &Ans| a.as_ref().ok().and_then(|v| v.first()).and_then(|l| l.split(' ').next()).map(String::from);
            let area = if kind != "wrong" {
                "merge"
            } else if verdict(gix) != verdict(git) {
                if clean(gix) { "gix_clean" } else { "gix_conflict" }
            } else if clean(git) {
                "clean_tree"
            } else {
                // Listings differ; a file/symlink/gitlink type change among the differing entries
                // is its own area.
                let (Ok(a), Ok(b)) = (gix, git) else { unreachable!() };
                let diff = a.iter().filter(|l| !b.contains(l)).chain(b.iter().filter(|l| !a.contains(l)));
                // A path only gix leaves with conflict markers (git resolved it) is a spurious conflict,
                // its own area; a conflict gix misses or a resolved entry it writes differently stays
                // `conflict_tree`.
                let path = |l: &str| l.splitn(3, ' ').nth(2).map(str::to_string);
                let markers = |v: &Vec<String>| -> std::collections::HashSet<String> {
                    v.iter().filter(|l| l.split(' ').nth(1) == Some("MARKERS")).filter_map(|l| path(l)).collect()
                };
                let (ma, mb) = (markers(a), markers(b));
                let diff: Vec<&String> = diff.collect();
                if diff.iter().any(|l| l.starts_with("120000") || l.starts_with("160000")) {
                    "conflict_tree_typechange"
                } else if mb.is_subset(&ma) && ma.difference(&mb).any(|p| b.iter().any(|l| path(l).as_deref() == Some(p))) {
                    "conflict_tree_gix_markers"
                } else {
                    "conflict_tree"
                }
            };
            format!("merge__{kind}__{area}")
        },
        "status" => format!("status__{kind}__{}", crate::wt::status_area(gix, git)),
        "index" => format!("index__{kind}__{}", crate::wt::index_area(gix, git)),
        "index_rt" => format!("index__{kind}__roundtrip"),
        _ => format!("graph__{kind}__{}", area(q)),
    }
}

pub fn gix_answer(repo: &gix::Repository, q: &Query<'_>) -> Ans {
    let w = &q.words;
    let ids = |v: Vec<gix::Id<'_>>| v.into_iter().map(|i| i.to_string()).collect::<Vec<_>>();
    match q.kind {
        "merge_base" => {
            let r = repo.merge_base(oid(w[0])?, oid(w[1])?).map_err(|e| e.to_string())?;
            Ok(r.map(|i| i.to_string()).into_iter().collect())
        },
        "merge_bases_many" => {
            let others = w[1..].iter().map(|s| oid(s)).collect::<Result<Vec<_>, _>>()?;
            Ok(ids(repo.merge_bases_many(oid(w[0])?, &others).map_err(|e| e.to_string())?))
        },
        "octopus" => {
            let all = w.iter().map(|s| oid(s)).collect::<Result<Vec<_>, _>>()?;
            let r = repo.merge_base_octopus(all).map_err(|e| e.to_string())?;
            Ok(r.map(|i| i.to_string()).into_iter().collect())
        },
        "walk" => {
            use gix::revision::walk::Sorting;
            use gix::traverse::commit::simple::CommitTimeOrder;
            let (tips, hidden) = split_tips(&w[3..])?;
            let sorting = match w[0] {
                "newest" => Sorting::ByCommitTime(CommitTimeOrder::NewestFirst),
                "oldest" => Sorting::ByCommitTime(CommitTimeOrder::OldestFirst),
                _ => Sorting::BreadthFirst,
            };
            let mut p = repo.rev_walk(tips).sorting(sorting).use_commit_graph(w[2] == "graph");
            if !hidden.is_empty() {
                p = p.with_hidden(hidden);
            }
            if w[1] == "fp" {
                p = p.first_parent_only();
            }
            let walk = p.all().map_err(|e| e.to_string())?;
            let mut out = Vec::new();
            for info in walk {
                out.push(info.map_err(|e| e.to_string())?.id.to_string());
                if out.len() > 100_000 {
                    return Err("walk does not end".into());
                }
            }
            Ok(out)
        },
        "topo" => {
            use gix::traverse::commit::{Parents, topo};
            let (tips, ends) = split_tips(&w[2..])?;
            let mut b = topo::Builder::from_iters(&repo.objects, tips, Some(ends))
                .sorting(if w[0] == "topo" { topo::Sorting::TopoOrder } else { topo::Sorting::DateOrder })
                .parents(if w[1] == "fp" { Parents::First } else { Parents::All });
            if let Ok(Some(g)) = repo.commit_graph_if_enabled() {
                b = b.with_commit_graph(Some(g));
            }
            let mut out = Vec::new();
            for info in b.build().map_err(|e| e.to_string())? {
                out.push(info.map_err(|e| e.to_string())?.id.to_string());
                if out.len() > 100_000 {
                    return Err("walk does not end".into());
                }
            }
            Ok(out)
        },
        "describe" => {
            use gix::commit::describe::SelectRef;
            let select = match w[0] {
                "tags" => SelectRef::AllTags,
                "all" => SelectRef::AllRefs,
                _ => SelectRef::AnnotatedTags,
            };
            let commit = repo.find_commit(oid(w[3])?).map_err(|e| e.to_string())?;
            let r = commit
                .describe()
                .names(select)
                .traverse_first_parent(w[1] == "fp")
                .max_candidates(w[2].parse().unwrap_or(10))
                .try_resolve()
                .map_err(|e| e.to_string())?;
            match r {
                Some(r) => match r.outcome.name {
                    Some(n) => Ok(vec![format!("{n} {}", r.outcome.depth)]),
                    None => Err("no name".into()),
                },
                None => Err("no name".into()),
            }
        },
        "diff" => crate::treediff::gix_answer(repo, q),
        "merge" => crate::merge::gix_answer(repo, q),
        "status" => crate::wt::gix_status(repo),
        "index" => crate::wt::gix_index(repo),
        "index_rt" => crate::wt::gix_index_rt(repo),
        k => Err(format!("unknown query {k}")),
    }
}

/// git's answer; `None` when git could not be run.
pub fn git_answer(run: &dyn Fn(&[&str]) -> Option<(bool, String, String)>, q: &Query<'_>) -> Option<Ans> {
    let w = &q.words;
    let mut args: Vec<&str> = Vec::new();
    match q.kind {
        "merge_base" | "merge_bases_many" => {
            args.extend(["merge-base", "--all"]);
            args.extend(w.iter().copied());
        },
        "octopus" => {
            args.extend(["merge-base", "--octopus", "--all"]);
            args.extend(w.iter().copied());
        },
        "walk" => {
            args.push("rev-list");
            if w[1] == "fp" {
                args.push("--first-parent");
            }
            args.extend(w[3..].iter().copied());
        },
        "topo" => {
            args.extend(["rev-list", if w[0] == "topo" { "--topo-order" } else { "--date-order" }]);
            if w[1] == "fp" {
                args.push("--first-parent");
            }
            args.extend(w[2..].iter().copied());
        },
        "describe" => {
            args.extend(["describe", "--long", "--abbrev=40"]);
            match w[0] {
                "tags" => args.push("--tags"),
                "all" => args.push("--all"),
                _ => {},
            }
            if w[1] == "fp" {
                args.push("--first-parent");
            }
            let cand = format!("--candidates={}", w[2]);
            let mut a: Vec<String> = args.iter().map(|s| s.to_string()).collect();
            a.push(cand);
            a.push(w[3].to_string());
            let a: Vec<&str> = a.iter().map(String::as_str).collect();
            let (ok, out, err) = run(&a)?;
            if !ok {
                return Some(Err(err.trim().to_string()));
            }
            // A tag ref whose tag object carries another name: git prints the tag object's own
            // name ("warning: tag 'main' is externally known as 'v2'"), gix the ref name. Which is
            // right is a naming choice, not a different description; no reference answer.
            if err.contains("is externally known as") {
                return Some(Ok(vec![EXTERNAL_NAME.into()]));
            }
            // `<name>-<depth>-g<hex>`; with --all the name keeps its `heads/`, `tags/`, `remotes/`
            // namespace, which gix's names do not (they are `shorten()`ed ref names).
            let line = out.trim();
            let mut parts = line.rsplitn(3, '-');
            let (_hex, depth, name) = (parts.next()?, parts.next()?, parts.next()?);
            let name = if w[0] == "all" {
                name.strip_prefix("heads/").or_else(|| name.strip_prefix("tags/")).or_else(|| name.strip_prefix("remotes/")).unwrap_or(name)
            } else {
                name
            };
            return Some(Ok(vec![format!("{name} {depth}")]));
        },
        "diff" => return crate::treediff::git_answer(run, q),
        "merge" => return crate::merge::git_answer(run, q),
        "status" => return crate::wt::git_status(run),
        "index" => return crate::wt::git_index(run),
        "index_rt" => return crate::wt::git_index_rt(run),
        _ => return None,
    }
    let (ok, out, err) = run(&args)?;
    // `git merge-base` exits 1 without output when there is no merge base.
    if !ok && !(q.kind != "walk" && q.kind != "topo" && out.trim().is_empty() && err.trim().is_empty()) {
        return Some(Err(err.trim().to_string()));
    }
    Some(Ok(out.lines().map(String::from).collect()))
}

/// For `walk`/`topo`: the exact set `reachable(tips) - reachable(hidden)`, from rev-lists without
/// negative revisions. git's own `rev-list T ^H` stops painting early when commit dates are skewed
/// (date-based limiting without a commit-graph) and then lists ancestors of `H`. Hidden commits
/// hide all their ancestors, also with `--first-parent` (git black-box; gix: "don't return any of
/// their ancestors", walk.rs `with_hidden`).
pub fn exact_set(run: &dyn Fn(&[&str]) -> Option<(bool, String, String)>, q: &Query<'_>) -> Option<Vec<String>> {
    let (skip, fp) = match q.kind {
        "walk" => (3, q.words.get(1) == Some(&"fp")),
        "topo" => (2, q.words.get(1) == Some(&"fp")),
        _ => return None,
    };
    let words = q.words.get(skip..)?;
    let tips: Vec<&str> = words.iter().copied().filter(|w| !w.starts_with('^')).collect();
    let hidden: Vec<&str> = words.iter().filter_map(|w| w.strip_prefix('^')).collect();
    let mut args = vec!["rev-list"];
    if fp {
        args.push("--first-parent");
    }
    args.extend(&tips);
    let (ok, out, _) = run(&args)?;
    if !ok {
        return None;
    }
    let mut minus = std::collections::HashSet::new();
    if !hidden.is_empty() {
        let mut args = vec!["rev-list"];
        args.extend(&hidden);
        let (ok, h, _) = run(&args)?;
        if !ok {
            return None;
        }
        minus.extend(h.lines().map(String::from));
    }
    let mut v: Vec<String> = out.lines().filter(|l| !minus.contains(*l)).map(String::from).collect();
    v.sort();
    Some(v)
}

pub fn sorted(a: &Ans) -> Option<Vec<String>> {
    let mut v = a.as_ref().ok()?.clone();
    v.sort();
    Some(v)
}

/// Compare; `Some((kind, message))` on a disagreement.
pub fn compare(q: &Query<'_>, gix: &Ans, git: &Ans) -> Option<(&'static str, String)> {
    let show = |a: &Ans| match a {
        Ok(v) if v.len() > 12 => format!("{} items: {} ...", v.len(), v[..12].join(" ")),
        Ok(v) => format!("[{}]", v.join(" ")),
        Err(e) => format!("error {e:?}"),
    };
    let msg = || format!("query {:?}: gix {}, git {}", format!("#{} {}", q.kind, q.words.join(" ")), show(gix), show(git));
    match (gix, git) {
        (Err(_), Err(_)) => None,
        (Err(_), Ok(_)) => Some(("gix_rejects", msg())),
        (Ok(_), Err(_)) => Some(("gix_accepts", msg())),
        (Ok(a), Ok(b)) => {
            let same = match q.kind {
                // gix returns the best merge base; it must be one of git's (none iff none).
                "merge_base" | "octopus" => match a.first() {
                    None => b.is_empty(),
                    Some(x) => a.len() == 1 && b.contains(x),
                },
                "merge_bases_many" | "walk" => {
                    let mut a = a.clone();
                    let mut b = b.clone();
                    a.sort();
                    b.sort();
                    let dup = a.windows(2).any(|w| w[0] == w[1]);
                    !dup && a == b
                },
                _ => a == b,
            };
            (!same).then(|| ("wrong", msg()))
        },
    }
}

/// git-describe(1): "the tag which has the fewest commits different from the input commit-ish will
/// be selected and output. Here fewest commits different is defined as the number of commits which
/// would be shown by git log tag..input". git (2.55, 2.56) stops its walk once it has seen as many
/// tags as exist and keeps the first-found one, so on a merge whose other side has the closer tag
/// it names a farther one. True when gix's name is a valid candidate for the selection, its depth
/// is exactly `git rev-list --count <ref>..C`, and that is strictly fewer than for git's name.
/// Without --first-parent only (the doc defines the count for the full history).
pub fn describe_git_not_fewest(run: &dyn Fn(&[&str]) -> Option<(bool, String, String)>, q: &Query<'_>, gix: &Ans, git: &Ans) -> bool {
    let w = &q.words;
    if w.len() < 4 || w[1] == "fp" {
        return false;
    }
    let (Ok(a), Ok(b)) = (gix, git) else { return false };
    let (Some((an, ad)), Some((bn, bd))) = (a.first().and_then(|s| s.rsplit_once(' ')), b.first().and_then(|s| s.rsplit_once(' '))) else {
        return false;
    };
    let (Ok(ad), Ok(bd)) = (ad.parse::<u64>(), bd.parse::<u64>()) else { return false };
    if ad >= bd {
        return false;
    }
    let namespaces: &[&str] = if w[0] == "all" { &["refs/tags/", "refs/heads/", "refs/remotes/"] } else { &["refs/tags/"] };
    // `tag..C` counts of every ref the short name may stand for under this selection.
    let counts = |n: &str| -> Vec<u64> {
        namespaces
            .iter()
            .filter_map(|ns| {
                let full = format!("{ns}{n}");
                let (ok, kind, _) = run(&["cat-file", "-t", &full])?;
                if !ok || (w[0] == "annotated" && kind.trim() != "tag") {
                    return None;
                }
                let (ok, out, _) = run(&["rev-list", "--count", &format!("{full}..{}", w[3])])?;
                ok.then(|| out.trim().parse().ok()).flatten()
            })
            .collect()
    };
    let (ca, cb) = (counts(an), counts(bn));
    !cb.is_empty() && ca.contains(&ad) && cb.iter().all(|&c| c > ad)
}

/// `git describe` and gix name the commit at the same depth with different refs that point to the
/// same commit. Which of several equally good names wins is not documented (gix sorts by name,
/// and ranks lightweight tags like branches), except that annotated tags come first: a tie between
/// an annotated tag and something else is not masked, nor are two annotated tags (newest wins).
pub fn describe_tie(run: &dyn Fn(&[&str]) -> Option<(bool, String, String)>, gix: &Ans, git: &Ans) -> bool {
    let (Ok(a), Ok(b)) = (gix, git) else { return false };
    let (Some((an, ad)), Some((bn, bd))) = (a.first().and_then(|s| s.rsplit_once(' ')), b.first().and_then(|s| s.rsplit_once(' '))) else {
        return false;
    };
    if ad != bd {
        return false;
    }
    // (commit, annotated) of every ref the short name may stand for.
    let refs = |n: &str| -> Vec<(String, bool)> {
        ["refs/tags/", "refs/heads/", "refs/remotes/"]
            .iter()
            .filter_map(|ns| {
                let full = format!("{ns}{n}");
                let commit = run(&["rev-parse", "--verify", "-q", &format!("{full}^{{commit}}")])?;
                let kind = run(&["cat-file", "-t", &full])?;
                commit.0.then(|| (commit.1.trim().to_string(), kind.1.trim() == "tag"))
            })
            .collect()
    };
    let (ra, rb) = (refs(an), refs(bn));
    ra.iter().any(|(ca, ta)| rb.iter().any(|(cb, tb)| ca == cb && !*ta && !*tb))
}
