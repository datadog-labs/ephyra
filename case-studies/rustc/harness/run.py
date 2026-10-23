#!/usr/bin/env python3
# Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
# This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

"""run.py <build-dir> <seed> <seconds> <out-dir>: one search process.

Each case: generate a safe Rust program (gen.py), compile it with the baseline config
(-Copt-level=0 -Zmir-opt-level=0) and a random sample of optimising configs (oracle.CONFIGS),
run every binary and compare stdout + exit status with the baseline.
Oracles:
  ice__*        any config crashes rustc (ICE, debug assertion, LLVM assertion, broken MIR, hang)
  miscompile__* an optimising config's binary behaves differently from the baseline's
  diff__compile_error__* an optimising config rejects a program the baseline accepts
  consteval__mismatch__* (const programs) CTFE and run-time evaluation of a call disagree
  drop__<leak|double>__base (droppy programs) a value is not dropped exactly once at the baseline;
                *_drop area suffix: a config's output differs from the baseline's only in destructor logs
"""
import json
import os
import random
import shutil
import subprocess
import sys
import time
from concurrent.futures import ThreadPoolExecutor

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import gen  # noqa: E402
import oracle  # noqa: E402
import rlfix  # noqa: E402
from oracle import CONFIGS  # noqa: E402

JOBS = int(os.environ.get("HARNESS_JOBS", "4"))
SAMPLE = int(os.environ.get("HARNESS_SAMPLE", "4"))
CAP = 3
# sampling weights: new / default-off passes and the default -O pipeline get more cases
WEIGHTS = {"O0m1": 1, "O0m2": 3, "O0m3": 2, "O0m4": 2, "O0m2me": 3, "O0m3me": 2, "O3me": 3,
           "O3m0": 1, "O1": 1, "O2": 2, "O3": 3, "Os": 1, "Oz": 1, "O3m4": 2, "O3inl": 2,
           "O0inl": 1, "cl0": 1, "cl3": 1,
           "O3dbg": 2, "O0m3dbg": 2, "O2medbg": 2, "O1strip": 1, "O3v3": 1, "O3v4": 2, "O3cgu1": 1}
# every case also gets one random -Zmir-enable-passes config ("toggle", oracle.toggle_flags)
TOGGLE = float(os.environ.get("HARNESS_TOGGLE", "1.0"))
# share of cases taken from rustlantis (custom-MIR programs) instead of gen.py, when available
RL_SHARE = float(os.environ.get("HARNESS_RUSTLANTIS", "0.08"))


def main():
    build, seed, secs, out = sys.argv[1], int(sys.argv[2]), float(sys.argv[3]), sys.argv[4]
    build = os.path.abspath(build)
    out = os.path.abspath(out)
    rustc = os.path.join(build, "rustc")
    os.makedirs(os.path.join(out, "findings"), exist_ok=True)
    wroot = os.path.join(out, "work")
    os.makedirs(wroot, exist_ok=True)
    deadline = time.time() + secs - 5
    rng = random.Random(seed)
    stats = {"cases": 0, "failures": 0, "gen_invalid": 0, "base_bad_run": 0, "flaky": 0,
             "compiles": 0, "signatures": {}}
    t0 = time.time()

    def save_stats():
        stats["elapsed_s"] = round(time.time() - t0, 1)
        stats["cases_per_s"] = round(stats["cases"] / max(stats["elapsed_s"], 1e-3), 3)
        tmp = os.path.join(out, "stats.json.tmp")
        with open(tmp, "w") as f:
            json.dump(stats, f, indent=1)
        os.replace(tmp, os.path.join(out, "stats.json"))

    save_stats()
    pool = ThreadPoolExecutor(JOBS)
    names = list(WEIGHTS)
    rl = rustlantis_setup(build, wroot)
    i = 0
    while time.time() < deadline:
        gseed = seed * 10_000_000 + i
        i += 1
        wd = os.path.join(wroot, "c")
        shutil.rmtree(wd, ignore_errors=True)
        os.makedirs(wd)
        src = os.path.join(wd, "prog.rs")
        text = None
        if rl and rng.random() < RL_SHARE:
            text = rustlantis_prog(rl, gseed)
        no_me = False
        if text is not None:
            panicky, origin = False, "rustlantis"
            stats["rustlantis"] = stats.get("rustlantis", 0) + 1
            # make the program valid under the MIR semantics -Zmir-move-elimination opts into
            # (rlfix.py); if that isn't possible, don't run it with that flag
            fixed = rlfix.new_semantics(text)
            if fixed is None:
                no_me = True
                stats["rl_no_move_elim"] = stats.get("rl_no_move_elim", 0) + 1
            else:
                text = fixed
        else:
            g = gen.Gen(gseed)
            text = "#![allow(warnings, arithmetic_overflow, unconditional_panic)]\n" + g.program()
            panicky, origin = g.panicky, "gen"
            stats["const"] = stats.get("const", 0) + g.cmode
            stats["droppy"] = stats.get("droppy", 0) + g.droppy
        common = oracle.COMMON_OVF if panicky else oracle.COMMON
        stats["panicky"] = stats.get("panicky", 0) + panicky
        with open(src, "w") as f:
            f.write(text)
        sample = []
        while len(sample) < SAMPLE:
            c = rng.choices(names, [WEIGHTS[n] for n in names])[0]
            if c not in sample and not (panicky and c in oracle.PANIC_FREE_ONLY):
                sample.append(c)
        cfgmap = dict(CONFIGS)
        if rng.random() < TOGGLE:
            cfgmap["toggle"] = (oracle.toggle_flags(random.Random(gseed)), "pass_toggle")
            sample.append("toggle")
        if no_me:
            cfgmap = {c: v for c, v in cfgmap.items() if "-Zmir-move-elimination" not in v[0]}
            sample = [c for c in sample if c in cfgmap]
        cfgs = ["base"] + sample
        beh = lambda c: oracle.behaviour(rustc, src, wd, c, common, cfgmap[c][0])  # noqa: E731
        if origin == "rustlantis":
            # base first: a third of rustlantis programs hit a known ICE there (KNOWN_ICE_CUSTOM_MIR)
            results = {"base": beh("base")}
            if results["base"]["compile"]["status"] == "ice" and \
                    oracle.crash_signature(results["base"]["compile"]) in KNOWN_ICE_CUSTOM_MIR:
                stats["rl_known_ice"] = stats.get("rl_known_ice", 0) + 1
                stats["compiles"] += 1
                continue
            results.update(zip(sample, pool.map(beh, sample)))
        else:
            results = dict(zip(cfgs, pool.map(beh, cfgs)))
        stats["compiles"] += len(cfgs)
        if time.time() > deadline + 30:
            break   # don't count a case cut short by the time limit
        stats["cases"] += 1
        stats["last_seed"] = gseed
        failure = check(results, rustc, src, wd, pool, stats, common, panicky, cfgmap, origin)
        if failure:
            sig, mode, cfg, msg = failure
            stats["failures"] += 1
            stats["signatures"][sig] = stats["signatures"].get(sig, 0) + 1
            record(out, sig, mode, cfg, msg, text, gseed, build, stats, common, cfgmap[cfg][0])
        save_stats()
    save_stats()
    pool.shutdown(wait=False, cancel_futures=True)
    shutil.rmtree(wroot, ignore_errors=True)


# Filed upstream and hit by ~40% of rustlantis programs even at -Zmir-opt-level=0, which would
# drown everything else: rust-lang/rust#149191 (debug assertion "Post-projection ... layout should
# be non-Ref", codegen_ssa/src/mir/analyze.rs). Only skipped for rustlantis programs.
KNOWN_ICE_CUSTOM_MIR = {"ice__panic__codegen_ssa_analyze", "ice__debug_assert__codegen_ssa_analyze"}


def rustlantis_setup(build, wroot):
    """rustlantis `generate` (built by build.sh into <build>/rustlantis) needs config.toml in its cwd."""
    exe = os.path.join(build, "rustlantis", "generate")
    if not os.access(exe, os.X_OK):
        return None
    d = os.path.join(wroot, "rl")
    os.makedirs(d, exist_ok=True)
    shutil.copy(os.path.join(HERE, "rustlantis.toml"), os.path.join(d, "config.toml"))
    return (exe, d)


def rustlantis_prog(rl, gseed):
    exe, d = rl
    try:
        p = subprocess.run([exe, str(gseed)], cwd=d, capture_output=True, timeout=60)
    except subprocess.TimeoutExpired:
        return None
    if p.returncode != 0 or not p.stdout:
        return None
    return p.stdout.decode("utf-8", "replace")


def check(results, rustc, src, wd, pool, stats, common, panicky, cfgmap, origin):
    """-> (signature, mode, config, message) or None"""
    if origin == "rustlantis":
        for c, r in list(results.items()):
            if r["compile"]["status"] == "ice" and oracle.crash_signature(r["compile"]) in KNOWN_ICE_CUSTOM_MIR:
                stats["rl_known_ice"] = stats.get("rl_known_ice", 0) + 1
                if c == "base":
                    return None
                del results[c]
    for c, r in results.items():
        if r["compile"]["status"] in ("ice", "timeout"):
            if r["compile"]["status"] == "timeout" and results["base"]["compile"]["status"] == "ok" \
                    and results["base"]["compile"]["secs"] > oracle.COMPILE_TIMEOUT / 4:
                continue   # slow program, not a hang
            sig = oracle.crash_signature(r["compile"])
            return sig, "ice", c, first_lines(r["compile"]["stderr"])
    base = results["base"]
    if base["compile"]["status"] != "ok":
        stats["gen_invalid"] += 1
        stats.setdefault("gen_invalid_example", first_lines(base["compile"]["stderr"], 6))
        return None
    if base["run"][0] != "exit" or base["run"][2] not in ((0, 101) if panicky else (0,)):
        stats["base_bad_run"] += 1
        stats.setdefault("base_bad_run_examples", []).append([stats.get("last_seed"), base["run"][0], base["run"][2]])
        stats["base_bad_run_examples"] = stats["base_bad_run_examples"][-5:]
        return None
    for c, r in results.items():
        if r["compile"]["status"] == "error":
            return (f"diff__compile_error__{cfgmap[c][1]}", "compile_error", c,
                    first_lines(r["compile"]["stderr"]))
    # CTFE vs run time (const programs): a call's const value and its run-time value differ in the
    # unoptimised baseline itself -> consteval__mismatch__<value class>
    cls = oracle.consteval_mismatch(base["run"][1])
    if cls:
        return (f"consteval__mismatch__{cls}", "consteval", "base",
                "a const item (CTFE) and the same call at run time print different values at the baseline")
    # destructor bookkeeping (droppy programs): a value leaked or dropped twice in the unoptimised
    # baseline itself (drop elaboration / MIR building / unwinding) -> drop__<leak|double>__base
    dk = oracle.drop_count_bad(base["run"][1])
    if dk:
        return (f"drop__{dk}__base", "dropcount", "base",
                f"a `drop: <hash> <n>` line has n != 0 at the baseline ({dk})")
    diffs = {c: oracle.diff_kind(base["run"], r["run"]) for c, r in results.items() if c != "base"}
    diffs = {c: k for c, k in diffs.items() if k}
    if not diffs:
        return None
    # Localise: evaluate every config so the area doesn't depend on the random sample.
    rest = [c for c in cfgmap if c not in results and not (panicky and c in oracle.PANIC_FREE_ONLY)]
    for c, r in zip(rest, pool.map(lambda c: oracle.behaviour(rustc, src, wd, c, common, cfgmap[c][0]), rest)):
        stats["compiles"] += 1
        if r["compile"]["status"] == "ok":
            k = oracle.diff_kind(base["run"], r["run"])
            if k:
                diffs[c] = k
    if origin == "rustlantis":
        me = [c for c in diffs if "-Zmir-move-elimination" in cfgmap[c][0]]
        if me and oracle.rfc3943_hazards(open(src).read()):
            # UB under the semantics -Zmir-move-elimination opts into (oracle.rfc3943_hazards)
            stats["rl_rfc3943_skip"] = stats.get("rl_rfc3943_skip", 0) + 1
            for c in me:
                del diffs[c]
            if not diffs:
                return None
    area = oracle.classify_diff(set(diffs), cfgmap)
    # representative config: first (in CONFIGS order) differing config of the deciding family
    fam_of_area = {"mir_opt": "mir", "mir_move_elim": "move_elim", "llvm": "llvm", "mir_inline": "inline",
                   "mir_llvm": "mir_llvm", "cranelift": "cranelift", "mir_cranelift": "mir_cranelift",
                   "mir_pass_toggle": "pass_toggle", "debuginfo": "debuginfo", "llvm_target_cpu": "target_cpu",
                   "cgu": "cgu"}
    rep = next((c for c in cfgmap if c in diffs and cfgmap[c][1] == fam_of_area.get(area)),
               next(c for c in cfgmap if c in diffs))
    kind = diffs[rep]
    if origin == "rustlantis":
        # rustlantis (patched to use raw pointers only) can still go out of bounds or read uninit
        # memory: only report a difference when Miri finds no UB and prints what the baseline printed.
        verdict = miri_gate(src, base["run"][1])
        stats[f"rl_miri_{verdict}"] = stats.get(f"rl_miri_{verdict}", 0) + 1
        if verdict != "clean":
            return None
        area += "_custom_mir"
    if kind == "stdout" and oracle.drop_only_diff(base["run"][1], results[rep]["run"][1] if rep in results
                                                   else oracle.behaviour(rustc, src, wd, rep, common, cfgmap[rep][0])["run"][1]):
        # only the destructor log differs: own area, so drop bugs don't hide behind value bugs
        area += "_drop"
    msg = f"{rep} differs from base ({kind}); all differing configs: {sorted(diffs)}; flags: {cfgmap[rep][0]}"
    return f"miscompile__{kind}__{area}", "miscompile", rep, msg


MIRI_TC = os.environ.get("HARNESS_MIRI_TC",
                         "/agent-run/work/ref/rustup/toolchains/nightly-x86_64-unknown-linux-gnu")
MIRI_SYSROOT = os.environ.get("HARNESS_MIRI_SYSROOT", "/agent-run/work/ref/cache/miri")


def miri_gate(src, base_out):
    """-> clean | ub | differs | unavailable. Tree Borrows: raw pointers carry no tags there, and the
    programs have no references, so it only checks bounds/init/validity (Stacked Borrows would flag
    raw pointers invalidated by direct writes to their pointee, which no optimisation exploits)."""
    miri = os.path.join(MIRI_TC, "bin", "miri")
    if not os.access(miri, os.X_OK):
        return "unavailable"
    env = dict(os.environ, LD_LIBRARY_PATH=os.path.join(MIRI_TC, "lib"))
    for extra in (["-Zmiri-tree-borrows"],):
        try:
            p = subprocess.run([miri, "--sysroot", MIRI_SYSROOT, "-Awarnings"] + extra + [src],
                               capture_output=True, timeout=300, env=env, stdin=subprocess.DEVNULL)
        except subprocess.TimeoutExpired:
            return "unavailable"
        if p.returncode != 0:
            return "ub" if b"Undefined Behavior" in p.stderr else "unavailable"
        if p.stdout != base_out:
            return "differs"
    return "clean"


def first_lines(s, n=12):
    lines = [l for l in s.splitlines() if l.strip()]
    keep = [l for l in lines if "panicked" in l or "error" in l or "Assertion" in l or "broken MIR" in l]
    return "\n".join((keep or lines)[:n])[:2000]


def record(out, sig, mode, cfg, msg, text, gseed, build, stats, common, cfg_flags):
    sdir = os.path.join(out, "findings", sig)
    os.makedirs(sdir, exist_ok=True)
    n = len([d for d in os.listdir(sdir) if d.isdigit()])
    if n >= CAP:
        return
    d = os.path.join(sdir, str(n))
    os.makedirs(d)
    with open(os.path.join(d, "prog.rs"), "w") as f:
        f.write(text)
    shutil.copy(os.path.join(HERE, "oracle.py"), os.path.join(d, "oracle.py"))
    case = {"mode": mode, "config": cfg, "signature": sig, "common": common, "flags": common + cfg_flags,
            "cfg_flags": cfg_flags, "base_flags": common + CONFIGS["base"][0], "gen_seed": gseed}
    with open(os.path.join(d, "case.json"), "w") as f:
        json.dump(case, f, indent=1)
    with open(os.path.join(d, "repro.sh"), "w") as f:
        f.write("#!/bin/sh\n# usage: repro.sh <build-dir>   (exit 1 = reproduces, 0 = not, 2 = undecided)\n"
                f"# {mode}: rustc {' '.join(case['flags'])} prog.rs  vs  {' '.join(case['base_flags'])}\n"
                'cd "$(dirname "$0")" && exec python3 ./oracle.py repro "$1"\n')
    os.chmod(os.path.join(d, "repro.sh"), 0o755)
    with open(os.path.join(d, "finding.json"), "w") as f:
        json.dump({"signature": sig, "oracle": mode, "message": msg, "seed": gseed, "config": cfg}, f, indent=1)
    # determinism check: the repro must fail on the same build
    rc = subprocess.run(["sh", os.path.join(d, "repro.sh"), build], capture_output=True).returncode
    if rc != 1:
        stats["flaky"] += 1
        with open(os.path.join(d, "UNSTABLE"), "w") as f:
            f.write(f"repro.sh exit {rc} right after the search found it\n")


if __name__ == "__main__":
    main()
