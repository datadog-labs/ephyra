#!/usr/bin/env python3
# Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
# This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

"""Shared oracle code: compile configs, crash classification, miscompile classification.

Also the self-contained checker used by every finding's repro.sh:
    oracle.py repro <build-dir>      (reads case.json + prog.rs from its own directory)
exit 1 = the failure reproduces, 0 = it does not, 2 = cannot decide.
"""
import json
import os
import re
import subprocess
import sys

# Flags shared by every config. Overflow checks and debug assertions default to on only at
# opt-level=0, so pin them; -Zub-checks likewise follows debug-assertions. -Zvalidate-mir
# turns broken pass output into an ICE.
COMMON = ["--edition", "2021", "-Awarnings", "-Coverflow-checks=off", "-Cdebug-assertions=off",
          "-Zub-checks=no", "-Zvalidate-mir"]
# for generated programs that may panic on purpose (gen.py `panicky`): overflow checks on everywhere
COMMON_OVF = [f if f != "-Coverflow-checks=off" else "-Coverflow-checks=on" for f in COMMON]
# cranelift aborts on panic (Readme: "-Cpanic=abort is enabled by default"): no panicky programs there
PANIC_FREE_ONLY = {"cl0", "cl3"}

# name -> (flags, family). Every flag here is documented as semantics-preserving for UB-free
# programs (never -Zunsound-mir-opts). Baseline: no optimisation at all.
CONFIGS = {
    "base":   (["-Copt-level=0", "-Zmir-opt-level=0"], "base"),
    "O0m1":   (["-Copt-level=0", "-Zmir-opt-level=1"], "mir"),
    "O0m2":   (["-Copt-level=0", "-Zmir-opt-level=2"], "mir"),
    "O0m3":   (["-Copt-level=0", "-Zmir-opt-level=3"], "mir"),
    "O0m4":   (["-Copt-level=0", "-Zmir-opt-level=4"], "mir"),
    "O0m2me": (["-Copt-level=0", "-Zmir-opt-level=2", "-Zmir-move-elimination"], "move_elim"),
    "O0m3me": (["-Copt-level=0", "-Zmir-opt-level=3", "-Zmir-move-elimination"], "move_elim"),
    "O3me":   (["-Copt-level=3", "-Zmir-move-elimination"], "move_elim"),
    "O3m0":   (["-Copt-level=3", "-Zmir-opt-level=0"], "llvm"),
    "O1":     (["-Copt-level=1"], "mir_llvm"),
    "O2":     (["-Copt-level=2"], "mir_llvm"),
    "O3":     (["-Copt-level=3"], "mir_llvm"),
    "Os":     (["-Copt-level=s"], "mir_llvm"),
    "Oz":     (["-Copt-level=z"], "mir_llvm"),
    "O3m4":   (["-Copt-level=3", "-Zmir-opt-level=4"], "mir_llvm"),
    "O3inl":  (["-Copt-level=3", "-Zinline-mir=yes", "-Zinline-mir-threshold=1000",
                "-Zinline-mir-hint-threshold=2000"], "inline"),
    "O0inl":  (["-Copt-level=0", "-Zmir-opt-level=2", "-Zinline-mir=yes", "-Zinline-mir-threshold=1000"], "inline"),
    "cl0":    (["-Zcodegen-backend=cranelift", "-Copt-level=0", "-Zmir-opt-level=0"], "cranelift"),
    "cl3":    (["-Zcodegen-backend=cranelift", "-Copt-level=3"], "mir_cranelift"),
    # Option toggles that must not change behaviour either: full debuginfo (MIR opts must keep
    # VarDebugInfo consistent; -Zverify-llvm-ir runs the LLVM verifier, which checks debug metadata),
    # MIR debuginfo stripping, wider target features (other LLVM ISel/vectoriser paths), one CGU
    # with shared generics (other inlining/partitioning; fat/thin LTO cost ~10 s per compile).
    "O3dbg":  (["-Copt-level=3", "-Cdebuginfo=2", "-Zverify-llvm-ir"], "debuginfo"),
    "O0m3dbg": (["-Copt-level=0", "-Zmir-opt-level=3", "-Cdebuginfo=2", "-Zverify-llvm-ir"], "debuginfo"),
    "O2medbg": (["-Copt-level=2", "-Zmir-move-elimination", "-Cdebuginfo=2", "-Zverify-llvm-ir"], "debuginfo"),
    "O1strip": (["-Copt-level=1", "-Cdebuginfo=1", "-Zmir-strip-debuginfo=locals-in-tiny-functions",
                 "-Zinline-mir=yes"], "debuginfo"),
    "O3v3":   (["-Copt-level=3", "-Ctarget-cpu=x86-64-v3"], "target_cpu"),
    "O3v4":   (["-Copt-level=3", "-Ctarget-cpu=x86-64-v4"], "target_cpu"),
    "O3cgu1": (["-Copt-level=3", "-Ccodegen-units=1", "-Zshare-generics=yes"], "cgu"),
}

# Per-case MIR pass toggles (`-Zmir-enable-passes`, options.rs: "use like
# `-Zmir-enable-passes=+DestinationPropagation,-InstSimplify`"): optional passes switched off,
# or default-off optional passes switched on, on top of a MIR opt level. Optional passes must each
# preserve semantics (pass_manager.rs PassPolicy::Optional), so any subset must too.
TOGGLE_OFF = ["GVN", "ReferencePropagation", "ScalarReplacementOfAggregates", "DeadStoreElimination-initial",
              "DeadStoreElimination-final", "CopyProp", "DestinationPropagation", "JumpThreading",
              "MatchBranchSimplification", "SsaRangePropagation", "SingleUseConsts", "InstSimplify-before-inline",
              "InstSimplify-after-simplifycfg", "EarlyOtherwiseBranch", "UnreachablePropagation",
              "UnreachableEnumBranching", "SimplifyComparisonIntegral", "SimplifyLocals-before-const-prop",
              "SimplifyLocals-after-value-numbering", "RemoveZsts", "LowerSliceLenCalls", "Inline",
              "RemoveUnneededDrops", "TailCopyToMove", "MoveElimination"]
TOGGLE_ON = ["DataflowConstProp", "MultipleReturnTerminators", "ReorderBasicBlocks", "ReorderLocals",
             "MoveElimination", "TailCopyToMove", "DestinationPropagation", "Inline"]


TOGGLE_FAMILIES = {"pass_toggle", "debuginfo", "target_cpu", "cgu"}


def toggle_flags(rng):
    """Random pass-toggle config -> flags (deterministic in rng)."""
    opt = rng.choice(["-Copt-level=0", "-Copt-level=0", "-Copt-level=2", "-Copt-level=3"])
    lvl = rng.choice([1, 2, 2, 2, 3, 4])
    passes = [f"-{p}" for p in rng.sample(TOGGLE_OFF, rng.randint(1, 4))]
    passes += [f"+{p}" for p in rng.sample(TOGGLE_ON, rng.randint(0, 2))]
    rng.shuffle(passes)
    flags = [opt, f"-Zmir-opt-level={lvl}", "-Zmir-enable-passes=" + ",".join(passes)]
    if rng.random() < 0.3:
        flags.append("-Zmir-move-elimination")
    if rng.random() < 0.3:
        flags += ["-Cdebuginfo=2", "-Zverify-llvm-ir"]
    return flags

COMPILE_TIMEOUT = 90
RUN_TIMEOUT = 10
MAX_OUT = 1 << 20


def compile_prog(rustc, src, exe, flags, timeout=COMPILE_TIMEOUT, common=COMMON):
    """-> dict(status: ok|error|ice|timeout, rc, stderr, secs)"""
    import time
    t0 = time.time()
    try:
        p = subprocess.run([rustc] + common + flags + [src, "-o", exe], capture_output=True,
                           timeout=timeout, cwd=os.path.dirname(exe) or ".")
    except subprocess.TimeoutExpired:
        return {"status": "timeout", "rc": None, "stderr": "", "secs": time.time() - t0}
    err = p.stderr.decode("utf-8", "replace")
    st = "ok" if p.returncode == 0 else "error"
    if p.returncode != 0 and (p.returncode == 101 or p.returncode < 0 or "internal compiler error" in err
                              or "panicked at" in err or "Assertion `" in err or "LLVM ERROR" in err):
        st = "ice"
    return {"status": st, "rc": p.returncode, "stderr": err, "secs": time.time() - t0}


def _big_stack():
    # Stack use depends on frame sizes, which differ per config (unoptimised frames are the
    # biggest); a 1 GiB stack limit keeps generated programs from overflowing in any config.
    import resource
    soft, hard = resource.getrlimit(resource.RLIMIT_STACK)
    want = 1 << 30
    if hard != resource.RLIM_INFINITY:
        want = min(want, hard)
    resource.setrlimit(resource.RLIMIT_STACK, (want, hard))


def run_exe(exe, timeout=RUN_TIMEOUT):
    """-> (kind, stdout, rc). kind: exit (normal or nonzero exit), signal, timeout"""
    try:
        p = subprocess.run([exe], capture_output=True, timeout=timeout, stdin=subprocess.DEVNULL,
                           preexec_fn=_big_stack)
    except subprocess.TimeoutExpired:
        return ("timeout", b"", None)
    out = p.stdout[:MAX_OUT]
    if p.returncode < 0:
        return ("signal", out, p.returncode)
    return ("exit", out, p.returncode)


def _slug(s):
    return re.sub(r"[^a-z0-9]+", "_", s.lower()).strip("_")


def area_of_path(path):
    """compiler/rustc_codegen_ssa/src/mir/analyze.rs -> codegen_ssa_analyze;
    library/core/src/... -> core_<stem>; llvm .../GVN.cpp -> llvm_gvn"""
    path = path.replace("\\", "/")
    stem = os.path.splitext(os.path.basename(path))[0]
    m = re.search(r"compiler/rustc_([a-z0-9_]+)/", path)
    if m:
        crate = m.group(1)
        if stem in ("mod", "lib"):
            parts = path.split("/")
            stem = parts[-2] if len(parts) > 1 else stem
        return _slug(f"{crate}_{stem}")
    m = re.search(r"library/([a-z0-9_]+)/", path)
    if m:
        return _slug(f"{m.group(1)}_{stem}")
    if path.endswith((".cpp", ".h", ".c")):
        return _slug(f"llvm_{stem}")
    return _slug(stem) or "unknown"


def crash_signature(res):
    """Signature for a compiler crash (res from compile_prog with status ice/timeout)."""
    if res["status"] == "timeout":
        return "ice__hang__compile"
    err = res["stderr"]
    m = re.search(r"broken MIR in .*? \((?:after pass |after phase change to )?([A-Za-z0-9_\-]+)", err)
    if m:
        return f"ice__broken_mir__{_slug(m.group(1))}"
    m = re.search(r"Assertion `.*' failed", err)
    fatal = err.find("LLVM ERROR: ")
    if m and 0 <= fatal < m.start():
        m = None   # after a fatal LLVM error another codegen thread may trip over its debris: blame the first
    if m:
        # rustc: /path/llvm/lib/X/Y.cpp:123: func: Assertion `...' failed.
        m2 = re.search(r"(\S+\.(?:cpp|h|c)):\d+: .*Assertion `", err)
        return f"ice__llvm_assert__{area_of_path(m2.group(1)) if m2 else 'llvm'}"
    m = re.search(r"LLVM ERROR: ([^\n]*)", err)
    if m:
        words = _slug(re.sub(r"[^A-Za-z ]", " ", m.group(1))).split("_")[:3]
        return f"ice__llvm_error__{'_'.join(words) or 'llvm'}"
    m = re.search(r"Error while defining \S+: (\w+)\((\w+)\(\"should be implemented in ISLE: inst = `[^=`]*= ?(\w+)(\.\w+)?", err)
    if m:
        # cranelift lowering gap: one signature per missing opcode/type
        return f"ice__bug__cranelift_isle_{_slug(m.group(3) + (m.group(4) or ''))}"
    if "cranelift verify error" in err:
        return "ice__bug__cranelift_verify"
    m = re.search(r"panicked at ([^\s:]+\.rs):\d+", err)
    if m:
        kind = "debug_assert" if re.search(r"assertion `?\w*`? ?failed|assertion failed|debug_assert", err) else "panic"
        return f"ice__{kind}__{area_of_path(m.group(1))}"
    m = re.search(r"internal compiler error: ([^\s:]+\.rs):\d+", err)
    if m:
        return f"ice__bug__{area_of_path(m.group(1))}"
    if res["rc"] is not None and res["rc"] < 0:
        return f"ice__signal__sig{-res['rc']}"
    if "SIGSEGV" in err or "stack overflow" in err:
        return "ice__signal__stack_overflow"
    return "ice__unknown__rustc"


def classify_diff(diffs, cfgmap=CONFIGS):
    """diffs: set of config names whose behaviour differs from base -> area."""
    fam = {c: cfgmap[c][1] for c in diffs}
    fams = set(fam.values())
    if "mir" in fams:
        return "mir_opt"
    # option-toggle configs (some also enable move elimination) don't count against move_elim
    if "move_elim" in fams and fams - TOGGLE_FAMILIES <= {"move_elim", "base"}:
        # only blame -Zmir-move-elimination when no config without it differs too
        return "mir_move_elim"
    if "llvm" in fams:
        return "llvm"
    if "inline" in fams and not ("mir_llvm" in fams):
        return "mir_inline"
    if "mir_llvm" in fams or "inline" in fams:
        return "mir_llvm"
    if "cranelift" in fams:
        return "cranelift"
    if "mir_cranelift" in fams:
        return "mir_cranelift"
    # option toggles: only blamed when no config of the families above differs
    for f, area in (("pass_toggle", "mir_pass_toggle"), ("debuginfo", "debuginfo"),
                    ("target_cpu", "llvm_target_cpu"), ("cgu", "cgu")):
        if f in fams:
            return area
    return "unknown"


def behaviour(rustc, src, workdir, cfg, common=COMMON, flags=None):
    """Compile+run one config -> dict(compile=res, run=(kind,out,rc)|None)."""
    exe = os.path.join(workdir, f"exe_{cfg}")
    res = compile_prog(rustc, src, exe, CONFIGS[cfg][0] if flags is None else flags, common=common)
    out = None
    if res["status"] == "ok":
        out = run_exe(exe)
        try:
            os.unlink(exe)
        except OSError:
            pass
    return {"compile": res, "run": out}


# RFC 3943 (rust-lang/rfcs#3943, "MIR move elimination", the semantics -Zmir-move-elimination
# opts into: options.rs "enable the experimental MIR semantics for the move elimination
# optimization") makes some custom MIR UB that Miri still accepts. rustlantis emits these
# constructs in almost every program, so a move-elimination difference on such a program says
# nothing. Textual and conservative (over-reports): only used to drop move-elimination configs.
_RL_FN = re.compile(r"^(?:pub )?fn (\w+)\(([^)]*)\)", re.M)
_RL_ADDR = re.compile(r"(?:addr_of(?:_mut)?!\(|&raw (?:const|mut) )(_\d+|RET)\b")
_RL_DEST = re.compile(r"^(?:Call\()?(_\d+|RET)(?:\.[\w.]+)?\s*=[^=]")
_RL_MOVE = re.compile(r"Move\((_\d+|RET)\)")


def rfc3943_hazards(text):
    """-> [(fn, kind, local)] for custom-MIR functions using constructs that are UB under RFC 3943."""
    out = []
    starts = [(m.start(), m.group(1), m.group(2)) for m in _RL_FN.finditer(text)]
    for i, (pos, name, params) in enumerate(starts):
        body = text[pos:starts[i + 1][0] if i + 1 < len(starts) else len(text)]
        if "mir! {" not in body:
            continue
        args = set(re.findall(r"(_\d+)\s*:", params))
        written, addressed, moved = set(), set(), set()
        for line in body.splitlines():
            line = line.strip()
            for loc in _RL_ADDR.findall(line):
                # "Any use of the local that doesn't initialize it, even taking its address, is UB
                # if the local is unallocated." Only a direct write allocates; arguments start
                # allocated. (Text order stands in for control flow.)
                if loc not in args and loc not in written:
                    out.append((name, "addr_of_unallocated", loc))
                addressed.add(loc)
            ms = _RL_MOVE.findall(line)
            # "it is illegal for the same local to be moved multiple times in the same MIR statement"
            if len(ms) != len(set(ms)):
                out.append((name, "moved_twice", ms[0]))
            d = _RL_DEST.match(line)
            # "We specifically forbid the destination place from having the same base local as a
            # bare-local moved argument."
            if d and line.startswith("Call(") and d.group(1) in ms:
                out.append((name, "call_dest_is_moved_arg", d.group(1)))
            moved.update(ms)
            if d:
                written.add(d.group(1))
        # "accessing (read or write) a variable after it has been moved is no longer allowed since
        # the allocation has been freed": pointers to a bare-moved local may dangle.
        out += [(name, "moved_while_addressed", loc) for loc in sorted(addressed & moved)]
    return out


def diff_kind(base_run, run):
    """None if equal, else stdout|exit|signal|timeout"""
    if base_run == run:
        return None
    k, out, rc = run
    if k != "exit":
        return k
    if rc != base_run[2]:
        return "exit"
    return "stdout"


def consteval_mismatch(out):
    """Const programs (gen.py const_main) print `c<k>:<cls> <v>` (the value CTFE computed for call k)
    and `r<k>:<cls> <v>` (the same call at run time). -> cls of the first pair that differs, or None.
    Both sides use the same Debug formatting, which prints every NaN as `NaN`."""
    c, r = {}, {}
    for line in out.decode("utf-8", "replace").splitlines():
        m = re.match(r"([cr])(\d+):(\w+) (.*)$", line)
        if m:
            (c if m.group(1) == "c" else r)[m.group(2)] = (m.group(3), m.group(4))
    for k, (cls, v) in sorted(c.items(), key=lambda kv: int(kv[0])):
        if k in r and r[k][1] != v:
            return cls
    return None


def _drop_lines(out):
    return [l for l in out.decode("utf-8", "replace").splitlines() if l.startswith("drop: ")]


def drop_count_bad(out):
    """Droppy programs (gen.py `droppy`) print `drop: <hash> <n>` after every call in main, where n is
    D values created (D::new, Clone) minus values dropped or forgotten since the last line. Every
    value the call made is dead by then and is dropped exactly once, so n != 0 means a value was
    leaked (n > 0) or dropped twice (n < 0). -> 'leak' | 'double' | None"""
    for l in _drop_lines(out):
        parts = l.split()
        if len(parts) == 3 and parts[2].lstrip("-").isdigit() and int(parts[2]) != 0:
            return "leak" if int(parts[2]) > 0 else "double"
    return None


def drop_only_diff(base_out, out):
    """True when two outputs differ only in their `drop:` lines (destructor order/values/counts)."""
    strip = lambda o: [l for l in o.decode("utf-8", "replace").splitlines() if not l.startswith("drop: ")]
    return base_out != out and strip(base_out) == strip(out) and _drop_lines(base_out) != _drop_lines(out)


def repro(build_dir):
    here = os.path.dirname(os.path.abspath(__file__))
    case = json.load(open(os.path.join(here, "case.json")))
    rustc = os.path.join(os.path.abspath(build_dir), "rustc")
    src = os.path.join(here, "prog.rs")
    common = case.get("common", COMMON)
    cflags = case.get("cfg_flags") or CONFIGS[case["config"]][0]
    import tempfile
    with tempfile.TemporaryDirectory() as wd:
        if case["mode"] == "ice":
            res = compile_prog(rustc, src, os.path.join(wd, "exe"), cflags, common=common)
            if res["status"] == "ok":
                return 0
            if res["status"] in ("ice", "timeout") and crash_signature(res) == case["signature"]:
                return 1
            return 2 if res["status"] in ("ice", "timeout") else 0
        b = behaviour(rustc, src, wd, "base", common)
        if b["compile"]["status"] != "ok" or b["run"][0] != "exit":
            return 2
        if case["mode"] == "dropcount":
            x = behaviour(rustc, src, wd, case["config"], common, cflags)
            if x["compile"]["status"] != "ok" or x["run"][0] != "exit":
                return 2
            return 1 if drop_count_bad(x["run"][1]) else 0
        if case["mode"] == "consteval":
            x = behaviour(rustc, src, wd, case["config"], common, cflags)
            if x["compile"]["status"] != "ok" or x["run"][0] != "exit":
                return 2
            return 1 if consteval_mismatch(x["run"][1]) else 0
        x = behaviour(rustc, src, wd, case["config"], common, cflags)
        if case["mode"] == "compile_error":
            return 1 if x["compile"]["status"] == "error" else 0
        if x["compile"]["status"] != "ok":
            return 2
        return 1 if diff_kind(b["run"], x["run"]) else 0


if __name__ == "__main__":
    if len(sys.argv) == 3 and sys.argv[1] == "repro":
        sys.exit(repro(sys.argv[2]))
    print(__doc__)
    sys.exit(2)
