// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! tyinc: ty's incremental checking against fresh checks (the `incr__` oracle of tyz).
//!
//!   tyinc <project-dir> [+]<step-dir>...
//!
//! Each step directory holds the full set of project files (flat) at that step. The first step
//! starts a database the way watch mode and the language server do (LRU on, inputs mutable) and
//! checks it. Every later step syncs <project-dir> to the step's files (unique mtimes, since ty
//! keys file revisions on them), applies the change events to the same database and checks
//! again. A `+` prefix also checks that step with a fresh database configured like a one-shot
//! `ty check` (LRU off, inputs frozen). One JSON line per check on stdout:
//! `{"step": k, "mode": "inc"|"fresh", "diags": {"<file>": ["<line>:<severity>[<id>] <msg>", ...]}}`.
//!
//! After each incremental check, IDE requests (hover, goto, references, highlights, signature
//! help, completions, inlay hints, semantic tokens, symbols) run on a sample of the `c_*`/`u_*`
//! files at sampled identifier offsets, as a language server would after an edit; panics are
//! caught and reported as `{"step": k, "mode": "ide", "diags": {"<file>": ["0:Fatal[panic] Panicked at <loc> when running <op> ..."]}}`.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime};

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Mutex;

use ruff_db::files::system_path_to_file;
use ruff_db::source::{line_index, source_text};
use ruff_text_size::{TextRange, TextSize};
use ruff_db::system::{OsSystem, SystemPathBuf};
use ty_project::watch::{ChangeEvent, ChangedKind, CreatedKind, DeletedKind};
use ty_project::{Db as _, ProjectDatabase, ProjectMetadata};

struct Defer<F: FnMut()>(F);
impl<F: FnMut()> Drop for Defer<F> {
    fn drop(&mut self) {
        (self.0)()
    }
}

static PANIC: Mutex<Option<(String, String)>> = Mutex::new(None);

fn hash(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100_0000_01b3))
}

/// IDE requests on a sample of files and offsets; the panics they raise.
fn ide(db: &ProjectDatabase, root: &SystemPathBuf, names: &[String], step: usize, check: &BTreeMap<String, Vec<String>>) -> BTreeMap<String, Vec<String>> {
    let mut panics = Vec::new();
    // A query that panicked earlier in this revision (a poisoned cycle head) is not run again: salsa
    // rethrows `Cancelled::PropagatedPanic` with `resume_unwind`, so no hook call and no location.
    // Name it after the check's panic of this step, which is the original one: its location, and
    // the query a salsa assertion names (`query place_by_id(..) ...`), so the propagated panic
    // signs like the original one (`crash__panic__<file>_<line>_<query>`).
    let check_panic = check.values().flatten().find_map(|d| d.split_once("[panic] Panicked at ").and_then(|(_, r)| r.split_whitespace().next()).map(str::to_string));
    let check_query = check
        .values()
        .flatten()
        .filter(|d| d.contains("[panic] Panicked at "))
        .find_map(|d| d.split_once("`query ").and_then(|(_, r)| r.split_once('(')).map(|(q, _)| q.to_string()));
    let timing = std::env::var("TYINC_IDE_TIME").is_ok();
    let program = db.project().program(db);
    for name in names {
        if !(name.starts_with("c_") || name.starts_with("u_")) || hash(&format!("{name}/{step}")) % 6 != 0 {
            continue;
        }
        let Ok(file) = system_path_to_file(db, root.join(name)) else { continue };
        let pf = program.program_file(db, file);
        let text = source_text(db, file).as_str().to_string();
        let b = text.as_bytes();
        let starts: Vec<usize> = (0..b.len())
            .filter(|&i| (b[i].is_ascii_alphabetic() || b[i] == b'_') && (i == 0 || !(b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_')))
            .collect();
        let n = starts.len();
        let mut pick: Vec<usize> = (0..n.min(12)).map(|k| starts[(k * n / 12.min(n).max(1) + (hash(name) as usize % 7)) % n.max(1)]).collect();
        // Right after a `.`, `(` or `[` (where completions and signature help are triggered).
        let triggers: Vec<usize> = (1..b.len()).filter(|&i| matches!(b[i - 1], b'.' | b'(' | b'[')).collect();
        for k in 0..triggers.len().min(4) {
            pick.push(triggers[(k * 7919 + hash(name) as usize) % triggers.len()]);
        }
        let mut run = |op: &str, off: usize, f: &mut dyn FnMut()| {
            *PANIC.lock().unwrap() = None;
            let t0 = std::time::Instant::now();
            let _time = Defer(move || {
                if timing {
                    eprintln!("time {op} {:?}", t0.elapsed());
                }
            });
            if let Err(e) = catch_unwind(AssertUnwindSafe(f)) {
                let (loc, msg) = PANIC.lock().unwrap().take().unwrap_or_else(|| match e.downcast_ref::<salsa::Cancelled>() {
                    Some(c) => (
                        check_panic.clone().unwrap_or_else(|| "propagated".into()),
                        match &check_query {
                            Some(q) => format!("salsa::Cancelled: {c} (after `query {q}(..)` panicked in the check)"),
                            None => format!("salsa::Cancelled: {c}"),
                        },
                    ),
                    None => ("unknown".into(), String::new()),
                });
                panics.push((name.clone(), format!("Panicked at {loc} when running {op} at {name}@{off}: {msg}")));
            }
        };
        let whole = TextRange::new(TextSize::new(0), TextSize::new(text.len() as u32));
        run("inlay_hints", 0, &mut || {
            let _ = ty_ide::inlay_hints(db, pf, whole, &ty_ide::InlayHintSettings::default());
        });
        run("semantic_tokens", 0, &mut || {
            let _ = ty_ide::semantic_tokens(db, pf, None);
        });
        run("document_symbols", 0, &mut || {
            let _ = ty_ide::document_symbols(db, pf);
        });
        run("hints", 0, &mut || {
            let _ = ty_ide::hints(db, file);
        });
        for (k, &off) in pick.iter().enumerate() {
            let o = TextSize::new(off as u32);
            // The middle of the identifier, and its end (where a completion request lands).
            let end = off + b[off..].iter().take_while(|c| c.is_ascii_alphanumeric() || **c == b'_').count();
            let oe = TextSize::new(end as u32);
            run("hover", off, &mut || {
                let _ = ty_ide::hover(db, pf, o);
            });
            run("goto_definition", off, &mut || {
                let _ = ty_ide::goto_definition(db, pf, o);
            });
            run("goto_type_definition", off, &mut || {
                let _ = ty_ide::goto_type_definition(db, pf, o);
            });
            run("document_highlights", off, &mut || {
                let _ = ty_ide::document_highlights(db, pf, o);
            });
            run("signature_help", end, &mut || {
                let _ = ty_ide::signature_help(db, pf, oe);
            });
            // The expensive ones (references scan every file; completions every module) once.
            if k == 0 && hash(name) % 2 == 0 {
                run("find_references", off, &mut || {
                    let _ = ty_ide::find_references(db, pf, o, true);
                });
            }
            if k == 1 {
                run("completion", end, &mut || {
                    let _ = ty_ide::completion(db, &ty_ide::CompletionSettings::default(), ty_ide::CompletionCapabilities::default(), pf, oe);
                });
            }
        }
    }
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (f, p) in panics {
        out.entry(f).or_default().push(format!("0:Fatal[panic] {p}"));
    }
    out
}

fn open_db(root: &SystemPathBuf) -> ProjectDatabase {
    let system = OsSystem::new(root);
    let mut meta = ProjectMetadata::discover(root, &system).expect("discover");
    meta.apply_configuration_files(&system).expect("configuration");
    ProjectDatabase::fallible(meta, system).expect("database")
}

fn render(db: &ProjectDatabase) -> BTreeMap<String, Vec<String>> {
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for d in db.check() {
        let (file, line) = match d.primary_span() {
            Some(span) => {
                let f = span.expect_ty_file();
                let name = f.path(db).as_system_path().and_then(|p| p.file_name()).unwrap_or("?").to_string();
                let line = span.range().map(|r| line_index(db, f).line_index(r.start()).get()).unwrap_or(0);
                (name, line)
            }
            None => ("-".to_string(), 0),
        };
        out.entry(file).or_default().push(format!("{line}:{:?}[{}] {}", d.severity(), d.id(), d.concise_message()));
    }
    out
}

/// Runs `f`; a panic that escapes ty's own per-file handling (a propagated one carries no hook
/// call, so no location) is printed as `Panicked at <loc> in tyinc: ...` and ends the process with
/// 101. Without a location, a fresh check of the files on disk names the original panic.
fn guard<R>(what: &str, root: &SystemPathBuf, f: impl FnOnce() -> R) -> R {
    match ruff_db::panic::catch_unwind(AssertUnwindSafe(f)) {
        Ok(r) => r,
        Err(e) => {
            let first = e.to_string().lines().nth(1).unwrap_or("").chars().take(300).collect::<String>();
            let loc = e.location.clone().or_else(|| {
                let fresh = ruff_db::panic::catch_unwind(AssertUnwindSafe(|| render(&open_db(root)))).ok()?;
                let d = fresh.values().flatten().find(|d| d.contains("[panic] Panicked at "))?;
                d.split_once("[panic] Panicked at ").and_then(|(_, r)| r.split_whitespace().next()).map(str::to_string)
            });
            eprintln!("Panicked at {} in tyinc ({what}): {first}", loc.unwrap_or_else(|| "unknown".into()));
            std::process::exit(101);
        }
    }
}

fn files_in(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut m = BTreeMap::new();
    for e in fs::read_dir(dir).expect("step dir") {
        let e = e.unwrap();
        if e.file_type().unwrap().is_file() {
            m.insert(e.file_name().to_string_lossy().to_string(), fs::read(e.path()).unwrap());
        }
    }
    m
}

/// `tyinc --rename <project-dir> <in-dir> <requests.json>`: the `ren__` oracle's half in ty. Copies
/// the files of <in-dir> (flat) into <project-dir>, opens one database (like the language server),
/// and for each request `{"file", "offset", "new"}` asks `can_rename` and then `rename`, as an editor
/// does on F2. One JSON line per request on stdout: `{"id", "range": [s, e] | null, "edits": [[file,
/// s, e], ...] | null}`, or `{"id", "panic": "Panicked at <loc> ..."}`.
fn rename_main(args: &[String]) {
    let proj = Path::new(&args[0]);
    let _ = fs::remove_dir_all(proj);
    fs::create_dir_all(proj).unwrap();
    // Recursively: a program split over a package has `<pkg>/__init__.py`.
    fn copy(from: &Path, to: &Path) {
        for e in fs::read_dir(from).unwrap().flatten() {
            if e.path().is_dir() {
                fs::create_dir_all(to.join(e.file_name())).unwrap();
                copy(&e.path(), &to.join(e.file_name()));
            } else {
                fs::copy(e.path(), to.join(e.file_name())).unwrap();
            }
        }
    }
    copy(Path::new(&args[1]), proj);
    let root = SystemPathBuf::from_path_buf(proj.canonicalize().unwrap()).expect("utf-8 path");
    let reqs: Vec<serde_json::Value> = serde_json::from_str(&fs::read_to_string(&args[2]).unwrap()).unwrap();
    std::panic::set_hook(Box::new(|info| {
        let loc = info.location().map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column())).unwrap_or_else(|| "unknown".into());
        let msg = info.payload().downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| info.payload().downcast_ref::<String>().cloned()).unwrap_or_default();
        *PANIC.lock().unwrap() = Some((loc, msg.chars().take(300).collect()));
    }));
    let db = open_db(&root);
    let program = db.project().program(&db);
    for (id, r) in reqs.iter().enumerate() {
        let name = r["file"].as_str().unwrap();
        let off = TextSize::new(r["offset"].as_u64().unwrap() as u32);
        let new = r["new"].as_str().unwrap();
        let Ok(file) = system_path_to_file(&db, root.join(name)) else {
            println!("{}", serde_json::json!({"id": id, "range": null, "edits": null}));
            continue;
        };
        *PANIC.lock().unwrap() = None;
        let res = catch_unwind(AssertUnwindSafe(|| {
            let pf = program.program_file(&db, file);
            let range = ty_ide::can_rename(&db, pf, off)?;
            let edits = ty_ide::rename(&db, pf, off, new);
            Some((range, edits))
        }));
        let line = match res {
            Ok(None) => serde_json::json!({"id": id, "range": null, "edits": null}),
            Ok(Some((range, edits))) => {
                let edits = edits.map(|es| {
                    es.iter()
                        .map(|e| {
                            let f = e.file().path(&db).as_system_path().and_then(|p| p.strip_prefix(&root).ok()).map(|p| p.as_str().to_string()).unwrap_or_else(|| e.file().path(&db).to_string());
                            serde_json::json!([f, e.range().start().to_u32(), e.range().end().to_u32()])
                        })
                        .collect::<Vec<_>>()
                });
                serde_json::json!({"id": id, "range": [range.start().to_u32(), range.end().to_u32()], "edits": edits})
            }
            Err(e) => {
                let (loc, msg) = PANIC.lock().unwrap().take().unwrap_or_else(|| match e.downcast_ref::<salsa::Cancelled>() {
                    Some(c) => ("unknown".into(), format!("{c}")),
                    None => ("unknown".into(), String::new()),
                });
                serde_json::json!({"id": id, "panic": format!("Panicked at {loc} when running rename at {name}@{}: {msg}", off.to_u32())})
            }
        };
        println!("{line}");
    }
}

/// `tyinc --hints <project-dir> <in-dir>`: the `hint__` oracle's half in ty. Copies the files of
/// <in-dir> (flat) into <project-dir>, opens one database (like the language server) and asks for
/// the inlay hints (default settings) of every `p_*.py` over its whole text. One JSON line per file:
/// `{"file", "hints": [{"pos", "kind": "type"|"arg", "label", "edits": [[s, e, text], ...]}, ...]}`,
/// or `{"file", "panic": "Panicked at <loc> ..."}`.
fn hints_main(args: &[String]) {
    let proj = Path::new(&args[0]);
    let _ = fs::remove_dir_all(proj);
    fs::create_dir_all(proj).unwrap();
    let mut names = Vec::new();
    for e in fs::read_dir(&args[1]).unwrap().flatten() {
        if e.path().is_file() {
            fs::copy(e.path(), proj.join(e.file_name())).unwrap();
            let n = e.file_name().to_string_lossy().to_string();
            if n.starts_with("p_") && n.ends_with(".py") {
                names.push(n);
            }
        }
    }
    names.sort();
    let root = SystemPathBuf::from_path_buf(proj.canonicalize().unwrap()).expect("utf-8 path");
    std::panic::set_hook(Box::new(|info| {
        let loc = info.location().map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column())).unwrap_or_else(|| "unknown".into());
        let msg = info.payload().downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| info.payload().downcast_ref::<String>().cloned()).unwrap_or_default();
        *PANIC.lock().unwrap() = Some((loc, msg.chars().take(300).collect()));
    }));
    let db = open_db(&root);
    let program = db.project().program(&db);
    for name in &names {
        let Ok(file) = system_path_to_file(&db, root.join(name)) else { continue };
        *PANIC.lock().unwrap() = None;
        let res = catch_unwind(AssertUnwindSafe(|| {
            let pf = program.program_file(&db, file);
            let whole = TextRange::up_to(ruff_text_size::TextLen::text_len(source_text(&db, file).as_str()));
            ty_ide::inlay_hints(&db, pf, whole, &ty_ide::InlayHintSettings::default())
                .into_iter()
                .map(|h| {
                    let kind = match h.kind {
                        ty_ide::InlayHintKind::Type => "type",
                        ty_ide::InlayHintKind::CallArgumentName => "arg",
                    };
                    let edits: Vec<serde_json::Value> = h.text_edits.iter().map(|e| serde_json::json!([e.range.start().to_u32(), e.range.end().to_u32(), e.new_text])).collect();
                    serde_json::json!({"pos": h.position.to_u32(), "kind": kind, "label": h.display().to_string(), "edits": edits})
                })
                .collect::<Vec<_>>()
        }));
        let line = match res {
            Ok(hs) => serde_json::json!({"file": name, "hints": hs}),
            Err(e) => {
                let (loc, msg) = PANIC.lock().unwrap().take().unwrap_or_else(|| match e.downcast_ref::<salsa::Cancelled>() {
                    Some(c) => ("unknown".into(), format!("{c}")),
                    None => ("unknown".into(), String::new()),
                });
                serde_json::json!({"file": name, "panic": format!("Panicked at {loc} when running inlay_hints at {name}: {msg}")})
            }
        };
        println!("{line}");
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--rename") && args.len() == 5 {
        rename_main(&args[2..]);
        return;
    }
    if args.get(1).map(String::as_str) == Some("--hints") && args.len() == 4 {
        hints_main(&args[2..]);
        return;
    }
    if args.len() < 3 {
        eprintln!("usage: tyinc <project-dir> [+]<step-dir>...");
        std::process::exit(2);
    }
    let proj = Path::new(&args[1]);
    let _ = fs::remove_dir_all(proj);
    fs::create_dir_all(proj).unwrap();
    let root = SystemPathBuf::from_path_buf(proj.canonicalize().unwrap()).expect("utf-8 path");
    let mut tick = 0u64;
    let mut current: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    let mut db: Option<ProjectDatabase> = None;
    // TYINC_IDE=0 turns the IDE requests off.
    let ide_on = std::env::var("TYINC_IDE").map(|v| v != "0").unwrap_or(true);
    std::panic::set_hook(Box::new(|info| {
        let loc = info.location().map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column())).unwrap_or_else(|| "unknown".into());
        let msg = info.payload().downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| info.payload().downcast_ref::<String>().cloned()).unwrap_or_default();
        let msg: String = msg.chars().take(300).collect();
        eprintln!("Panicked at {loc} in tyinc: {msg}");
        *PANIC.lock().unwrap() = Some((loc, msg));
    }));
    for (k, a) in args[2..].iter().enumerate() {
        let (fresh, dir) = match a.strip_prefix('+') {
            Some(d) => (true, d),
            None => (false, a.as_str()),
        };
        let want = files_in(Path::new(dir));
        let mut events = Vec::new();
        tick += 1;
        let mtime = SystemTime::UNIX_EPOCH + Duration::from_secs(1_500_000_000 + tick);
        for (name, text) in &want {
            if current.get(name) == Some(text) {
                continue;
            }
            let p = root.join(name);
            fs::write(p.as_std_path(), text).unwrap();
            fs::File::options().write(true).open(p.as_std_path()).unwrap().set_modified(mtime).unwrap();
            events.push(if current.contains_key(name) {
                ChangeEvent::Changed { path: p, kind: ChangedKind::FileContent }
            } else {
                ChangeEvent::Created { path: p, kind: CreatedKind::File }
            });
        }
        for name in current.keys() {
            if !want.contains_key(name) {
                let p = root.join(name);
                let _ = fs::remove_file(p.as_std_path());
                events.push(ChangeEvent::Deleted { path: p, kind: DeletedKind::File });
            }
        }
        current = want;
        match &mut db {
            None => db = Some(open_db(&root)),
            Some(db) => {
                guard("apply_changes", &root, || db.apply_changes(&events));
            }
        }
        let diags = guard("check", &root, || render(db.as_ref().unwrap()));
        println!("{}", serde_json::json!({"step": k, "mode": "inc", "diags": diags}));
        // After the first and the last edit only (the requests are single-threaded).
        if ide_on && (k == 1 || k + 2 == args.len() - 2) {
            let names: Vec<String> = current.keys().cloned().collect();
            let panics = ide(db.as_ref().unwrap(), &root, &names, k, &diags);
            println!("{}", serde_json::json!({"step": k, "mode": "ide", "diags": panics}));
        }
        if fresh {
            let mut f = open_db(&root);
            ruff_db::disable_lru(&mut f);
            f.freeze_open_files();
            f.freeze();
            let diags = guard("fresh check", &root, || render(&f));
            println!("{}", serde_json::json!({"step": k, "mode": "fresh", "diags": diags}));
        }
    }
}
