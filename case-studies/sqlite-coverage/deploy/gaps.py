#!/usr/bin/env python3
"""The gap list: what of SQLite a run's inputs never reach, from llvm-cov's
export of its merged coverage profile (deploy/gaps.sh makes both).

Every function of the amalgamation is put in its source file (sections.tsv),
and files are split into in scope (the core: parser, planner, code
generator, VDBE, B-tree, pager, built-in functions, ...) and out of scope
(other platforms, extensions the oracles never load). For the files in
scope it lists:

- functions never called: a feature the generator never produces;
- in functions that are called, regions never executed and branches taken
  only one way: a shape it never produces, each with its source line.

    python3 gaps.py coverage.json --sections sections.tsv --source sqlite3.c --out DIR

writes DIR/gaps.md (for reading) and DIR/gaps.json (for a generator's
synthesis loop: one record per gap, with file, function, line, kind and the
source line).
"""
import argparse
import collections
import json
import os

# Files the oracles cannot reach by design: other platforms, and extensions
# the cases never load or enable.
OUT_OF_SCOPE = {
    "os_win.c", "os_kv.c", "mutex_w32.c", "mutex_noop.c", "mem0.c", "mem2.c", "mem3.c", "mem5.c",
    "fts3.c", "fts3_aux.c", "fts3_expr.c", "fts3_hash.c", "fts3_icu.c", "fts3_porter.c", "fts3_snippet.c",
    "fts3_tokenize_vtab.c", "fts3_tokenizer.c", "fts3_tokenizer1.c", "fts3_unicode.c", "fts3_unicode2.c",
    "fts3_write.c", "icu.c", "sqlite3session.c", "sqlite3rbu.c",
    "stmt.c", "carray.c", "test_vfstrace.c", "loadext.c", "notify.c", "userauth.c", "malloc.c-oom",
}

REGION_CODE = 0


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("export")
    ap.add_argument("--sections", required=True)
    ap.add_argument("--source", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--top", type=int, default=40)
    args = ap.parse_args()
    os.makedirs(args.out, exist_ok=True)

    sections = {}
    for line in open(args.sections):
        if line.startswith("#"):
            continue
        name, _, file = line.rstrip("\n").partition("\t")
        sections[name] = file
    source = open(args.source, encoding="latin-1").read().split("\n")

    def text(line):
        return source[line - 1].strip()[:120] if 0 < line <= len(source) else ""

    data = json.load(open(args.export))["data"][0]
    files = collections.defaultdict(lambda: collections.Counter())
    uncalled = collections.defaultdict(list)
    partial = []  # (function, file, uncovered regions, one-sided branches)
    records = []
    for f in data["functions"]:
        name = f["name"].rsplit(":", 1)[-1]
        file = sections.get(name, "?")
        scope = "out" if file in OUT_OF_SCOPE else "in"
        regions = [r for r in f["regions"] if r[7] == REGION_CODE and r[5] == 0]
        if not regions:
            continue
        c = files[(file, scope)]
        c["functions"] += 1
        c["regions"] += len(regions)
        c["covered regions"] += sum(1 for r in regions if r[4] > 0)
        branches = [b for b in f.get("branches", []) if b[6] == 0]
        c["branch directions"] += 2 * len(branches)
        c["covered directions"] += sum((b[4] > 0) + (b[5] > 0) for b in branches)
        if f["count"] == 0:
            c["uncalled"] += 1
            uncalled[(file, scope)].append((name, len(regions), regions[0][0]))
            if scope == "in":
                records.append({"kind": "function never called", "file": file, "function": name, "line": regions[0][0], "regions": len(regions), "source": text(regions[0][0])})
            continue
        c["called"] += 1
        missed = [r for r in regions if r[4] == 0]
        one_sided = [b for b in branches if (b[4] > 0) != (b[5] > 0)]
        if scope == "in" and (missed or one_sided):
            partial.append((name, file, missed, one_sided))
            seen = set()
            for r in missed:
                if r[0] not in seen:
                    seen.add(r[0])
                    records.append({"kind": "region never executed", "file": file, "function": name, "line": r[0], "source": text(r[0])})
            for b in one_sided:
                records.append({"kind": "branch taken one way", "file": file, "function": name, "line": b[0],
                                "taken": "true" if b[4] > 0 else "false", "source": text(b[0])})

    json.dump(records, open(os.path.join(args.out, "gaps.json"), "w"), indent=1)

    out = []
    w = out.append
    total = collections.Counter()
    for (file, scope), c in files.items():
        if scope == "in":
            total.update(c)
    w("# SQLite coverage gaps\n")
    w(f"In scope: {total['functions']} functions, {total['called']} called "
      f"({100 * total['called'] / max(total['functions'], 1):.1f}%); regions {100 * total['covered regions'] / max(total['regions'], 1):.1f}% "
      f"covered; branch directions {100 * total['covered directions'] / max(total['branch directions'], 1):.1f}% covered. "
      f"{len(records)} gap records in gaps.json.\n")
    w("## By file (in scope), most uncovered regions first\n")
    w("| File | Functions called | Regions covered | Branch directions covered | Uncovered regions |")
    w("|---|---|---|---|---|")
    rows = [(file, c) for (file, scope), c in files.items() if scope == "in"]
    rows.sort(key=lambda fc: -(fc[1]["regions"] - fc[1]["covered regions"]))
    for file, c in rows[: args.top]:
        w(f"| {file} | {c['called']}/{c['functions']} | {100 * c['covered regions'] / c['regions']:.0f}% | "
          f"{100 * c['covered directions'] / max(c['branch directions'], 1):.0f}% | {c['regions'] - c['covered regions']} |")
    w("\n## Functions never called (in scope), by file\n")
    for (file, scope), fs in sorted(uncalled.items(), key=lambda kv: -sum(n for _, n, _ in kv[1])):
        if scope != "in":
            continue
        fs.sort(key=lambda x: -x[1])
        names = ", ".join(f"`{n}` ({r})" for n, r, _ in fs[:25])
        more = f", and {len(fs) - 25} more" if len(fs) > 25 else ""
        w(f"- **{file}** ({len(fs)}, {sum(r for _, r, _ in fs)} regions): {names}{more}")
    w("\n## Called functions with the most uncovered regions (in scope)\n")
    partial.sort(key=lambda p: -(len(p[2]) + len(p[3])))
    for name, file, missed, one_sided in partial[: args.top]:
        w(f"### `{name}` ({file}): {len(missed)} regions never executed, {len(one_sided)} branches taken one way\n")
        shown = set()
        for r in missed[:8]:
            if r[0] in shown:
                continue
            shown.add(r[0])
            w(f"- line {r[0]}: never executed: `{text(r[0])}`")
        for b in one_sided[:6]:
            w(f"- line {b[0]}: only {'true' if b[4] > 0 else 'false'}: `{text(b[0])}`")
        w("")
    w("## Out of scope\n")
    oos = [(file, c) for (file, scope), c in files.items() if scope == "out"]
    w(", ".join(f"{file} ({c['called']}/{c['functions']} called)" for file, c in sorted(oos)))
    open(os.path.join(args.out, "gaps.md"), "w").write("\n".join(out) + "\n")
    print(f"{len(records)} gap records; {total['called']}/{total['functions']} in-scope functions called; written to {args.out}")


if __name__ == "__main__":
    main()
