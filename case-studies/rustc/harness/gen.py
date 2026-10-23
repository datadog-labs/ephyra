#!/usr/bin/env python3
# Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
# This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

"""Random generator of UB-free, panic-free, terminating *safe* Rust programs.

Usage: gen.py <seed>  -> program on stdout.

Every program is safe Rust (no `unsafe`), so it has no UB; all arithmetic is
wrapping/checked/saturating, every index is reduced modulo the length, there is
no recursion (functions only call earlier functions) and every loop is bounded,
so it cannot panic or hang. Its whole meaning is its stdout: main prints the
Debug form of every function result. Floats are printed with `{:?}`, which
prints every NaN as `NaN` (sign/payload are nondeterministic, RFC 3514).
"""
import random
import sys

INTS = ["i8", "i16", "i32", "i64", "i128", "u8", "u16", "u32", "u64", "u128", "usize", "isize"]
SIGNED = {"i8", "i16", "i32", "i64", "i128", "isize"}
BITS = {"i8": 8, "u8": 8, "i16": 16, "u16": 16, "i32": 32, "u32": 32, "i64": 64, "u64": 64,
        "i128": 128, "u128": 128, "isize": 64, "usize": 64}
FLOATS = ["f32", "f64"]
CONST_SHARE = 0.3

# Types are tuples:
# ('int', name) ('bool',) ('flt', name) ('arr', T, n) ('tup', (T, ...)) ('struct', name)
# ('enum', name) ('opt', T) ('box', T) ('vec', T)


def I(n):
    return ('int', n)


BOOL = ('bool',)


class Gen:
    def __init__(self, seed):
        self.r = random.Random(seed)
        self.structs = {}   # name -> [(field, T)]
        self.enums = {}     # name -> {'variants': [(vname, kind, [(fname|None, T)])], 'repr': str|None, 'discr': [int|None]}
        self.fns = []       # dicts: name, params [(name, T, mode)], ret, cost
        self.out = []
        self.impls = []
        # panicky programs: compiled with -Coverflow-checks=on; plain arithmetic, unchecked indexing
        # and unwrap may panic (deterministically); main catches each call's panic and goes on.
        self.panicky = self.p(0.35)
        # const programs: every function is a `const fn`; main evaluates each call both in a
        # `const` item (CTFE, rustc_const_eval) and at runtime on black_box'ed arguments, and prints
        # both (oracle.consteval_mismatch compares them). Only the const-callable subset of the
        # generator is used (no closures, iterators, traits, Box/Vec, `for`, PartialEq on aggregates).
        self.cmode = not self.panicky and self.p(CONST_SHARE)
        # droppy programs: a type `D` whose Drop impl logs its payload (a running hash of the drop
        # sequence plus a live-object count, printed after every call in main as `drop: <h> <n>`).
        # Values of D are nested in structs/enums/tuples/Option/Box/arrays, moved out (whole or
        # partially, conditionally = drop flags), forgotten, dropped early and unwound through.
        # The Reference (Destructors): "When control flow leaves a drop scope all variables
        # associated to that scope are dropped in reverse order of declaration (for variables) or
        # creation (for temporaries)"; every value is dropped at most once, so n must be 0.
        self.droppy = not self.cmode and self.p(0.45)
        self.cost_budget = 200_000
        self.nvar = 0
        # small pools so values collide
        self.int_pool = [self.r.choice(INTS) for _ in range(3)] + ["i32", "u8", "usize"]

    # ---------- helpers ----------
    def ch(self, xs):
        return self.r.choice(xs)

    def p(self, prob):
        return self.r.random() < prob

    def fresh(self, base="v"):
        self.nvar += 1
        return f"{base}{self.nvar}"

    def rs(self, t):
        k = t[0]
        if k in ('int', 'flt'):
            return t[1]
        if k == 'bool':
            return 'bool'
        if k == 'arr':
            return f"[{self.rs(t[1])}; {t[2]}]"
        if k == 'tup':
            return "(" + "".join(self.rs(x) + ", " for x in t[1]) + ")"
        if k in ('struct', 'enum'):
            return t[1]
        if k == 'drop':
            return 'D'
        if k == 'opt':
            return f"Option<{self.rs(t[1])}>"
        if k == 'box':
            return f"Box<{self.rs(t[1])}>"
        if k == 'vec':
            return f"Vec<{self.rs(t[1])}>"
        raise ValueError(t)

    def is_copy(self, t):
        k = t[0]
        if k in ('int', 'flt', 'bool'):
            return True
        if k in ('box', 'vec', 'drop', 'moved'):
            return False
        if k == 'arr' or k == 'opt':
            return self.is_copy(t[1])
        if k == 'tup':
            return all(self.is_copy(x) for x in t[1])
        if k == 'struct':
            return all(self.is_copy(ft) for _, ft in self.structs[t[1]])
        if k == 'enum':
            return all(self.is_copy(ft) for _, _, fs in self.enums[t[1]]['variants'] for _, ft in fs)
        raise ValueError(t)

    def has_float(self, t):
        k = t[0]
        if k == 'flt':
            return True
        if k in ('arr', 'opt', 'box', 'vec'):
            return self.has_float(t[1])
        if k == 'tup':
            return any(self.has_float(x) for x in t[1])
        if k == 'struct':
            return any(self.has_float(ft) for _, ft in self.structs[t[1]])
        if k == 'enum':
            return any(self.has_float(ft) for _, _, fs in self.enums[t[1]]['variants'] for _, ft in fs)
        return False

    # ---------- types ----------
    def scalar_type(self):
        r = self.r.random()
        if self.cmode and r < 0.3:
            # const programs lean on what CTFE implements separately from codegen: soft floats
            # (rustc_apfloat) and 128-bit integers
            return ('flt', self.ch(FLOATS)) if self.p(0.6) else I(self.ch(["i128", "u128"]))
        if r < 0.72:
            return I(self.ch(self.int_pool) if self.p(0.7) else self.ch(INTS))
        if r < 0.87:
            return BOOL
        return ('flt', self.ch(FLOATS))

    def rand_type(self, depth=2, copy_only=False):
        if depth <= 0:
            return self.scalar_type()
        if self.droppy and not copy_only and self.p(0.22):
            return ('drop',)
        r = self.r.random()
        if r < 0.45:
            return self.scalar_type()
        if r < 0.55:
            return ('arr', self.rand_type(depth - 1, copy_only) if self.p(0.3) else I(self.ch(self.int_pool)), self.ch([1, 2, 3, 4, 8, 16, 32, 64]))
        if r < 0.63:
            return ('tup', tuple(self.rand_type(depth - 1, copy_only) for _ in range(self.r.randint(2, 3))))
        if r < 0.73 and self.structs:
            t = ('struct', self.ch(sorted(self.structs)))
            if not copy_only or self.is_copy(t):
                return t
        if r < 0.85 and self.enums:
            t = ('enum', self.ch(sorted(self.enums)))
            if not copy_only or self.is_copy(t):
                return t
        if r < 0.93:
            return ('opt', self.rand_type(depth - 1, copy_only))
        if not copy_only and not self.cmode:
            if self.p(0.5):
                return ('box', self.rand_type(depth - 1, copy_only))
            return ('vec', I(self.ch(self.int_pool)))
        return self.scalar_type()

    def make_types(self):
        for i in range(self.r.randint(1, 3)):
            name = f"S{i}"
            fields = [(f"f{j}", self.rand_type(1)) for j in range(self.r.randint(1, 4))]
            self.structs[name] = fields
        for i in range(self.r.randint(1, 3)):
            name = f"E{i}"
            nv = self.r.randint(2, 5)
            variants = []
            fieldless = self.p(0.35)
            for j in range(nv):
                if fieldless or self.p(0.3):
                    variants.append((f"V{j}", 'unit', []))
                elif self.p(0.6):
                    variants.append((f"V{j}", 'tuple', [(None, self.rand_type(1)) for _ in range(self.r.randint(1, 2))]))
                else:
                    variants.append((f"V{j}", 'named', [(f"x{k}", self.rand_type(1)) for k in range(self.r.randint(1, 2))]))
            repr_ = None
            discr = [None] * nv
            if self.p(0.5):
                repr_ = self.ch(["u8", "i8", "u16", "i32", "u32", "i64", "u64"])
                lo = -128 if repr_ in SIGNED else 0
                hi = 127 if repr_ in ("u8", "i8") else 1000
                used = set()
                for j in range(nv):
                    if self.p(0.6):
                        while True:
                            d = self.ch([lo, hi, 0, 1, 2, 4, 8, 16, 64, self.r.randint(lo, hi)])
                            if d not in used:
                                break
                        discr[j] = d
                # implicit discriminants are previous+1: check for collisions
                cur, seen, ok = -1, set(), True
                for j in range(nv):
                    cur = discr[j] if discr[j] is not None else cur + 1
                    if cur in seen or cur > hi or cur < lo:
                        ok = False
                    seen.add(cur)
                if not ok:
                    discr = [None] * nv
            self.enums[name] = {'variants': variants, 'repr': repr_, 'discr': discr}

    def emit_drop_type(self):
        attr = self.ch(["", "#[inline(never)] ", "#[inline(always)] "])
        self.out += [
            "use std::cell::Cell;",
            "thread_local! { static DH: Cell<u64> = const { Cell::new(0) }; static DN: Cell<i64> = const { Cell::new(0) }; }",
            "#[derive(Debug, PartialEq)]",
            "struct D(i64);",
            "impl D { " + attr + "fn new(x: i64) -> D { DN.with(|n| n.set(n.get() + 1)); D(x) } }",
            "impl Clone for D { fn clone(&self) -> D { D::new(self.0 ^ 256) } }",
            "impl Drop for D { " + attr + "fn drop(&mut self) { DH.with(|h| h.set(h.get().wrapping_mul(1000003) ^ (self.0 as u64))); DN.with(|n| n.set(n.get() - 1)); } }",
            "fn forget_d(d: D) { DN.with(|n| n.set(n.get() - 1)); std::mem::forget(d); }",
            "fn dlog() { let h = DH.with(|h| h.replace(0)); let n = DN.with(|n| n.replace(0)); println!(\"drop: {h} {n}\"); }",
        ]

    def move_stmt(self, env, d, pad, loop_depth):
        """Move a non-Copy local out (whole, partially, conditionally): drop elaboration / drop flags.
        Moved locals get type ('moved',), which nothing matches (the dict is shared by every scope's
        env list, so a move in a branch makes the local unusable afterwards everywhere)."""
        cands = [v for v in env if not self.is_copy(v['t']) and v['t'][0] != 'moved'
                 and not v['name'].startswith('(*') and v.get('ld', 0) == loop_depth]
        if not cands:
            return None
        v = self.ch(cands)
        t, name = v['t'], v['name']
        others = [e for e in env if e is not v]
        cond = lambda: self.expr(BOOL, others, d - 1)
        r = self.r.random()
        out = None
        keep = False
        if r < 0.15:
            out = [f"{pad}drop({name});"]
        elif r < 0.3:
            out = [f"{pad}if {cond()} {{ drop({name}); }}"]
            if v.get('mut') and self.p(0.6):
                # assignment to a maybe-moved place: the old value is dropped only if still there
                out = [f"{pad}if {cond()} {{ drop({name}); }} {name} = {self.expr(t, others, d - 1)};"]
                keep = True
        elif r < 0.45:
            w = self.fresh()
            out = [f"{pad}let {w}: {self.rs(t)} = if {cond()} {{ {name} }} else {{ {self.expr(t, others, d - 1)} }};"]
            others.append({'name': w, 't': t, 'mut': False, 'ld': loop_depth})
            env.append(others[-1])
        elif r < 0.55 and t == ('drop',):
            out = [f"{pad}if {cond()} {{ forget_d({name}); }}"]
        elif r < 0.75:
            # partial move out of a struct/tuple field, or out of a Box
            parts = []
            if t[0] == 'struct':
                parts = [(f"{name}.{f}", ft) for f, ft in self.structs[t[1]] if not self.is_copy(ft)]
            elif t[0] == 'tup':
                parts = [(f"{name}.{i}", x) for i, x in enumerate(t[1]) if not self.is_copy(x)]
            elif t[0] == 'box':
                parts = [(f"*{name}", t[1])]
            if not parts:
                return None
            src, pt = self.ch(parts)
            w = self.fresh()
            if self.p(0.3) and t[0] != 'box':
                # edition 2021 closures capture disjoint fields: a move closure takes only `src`
                out = [f"{pad}let {w}: {self.rs(pt)} = (move || {src})();"]
            elif self.p(0.4):
                out = [f"{pad}let {w}: {self.rs(pt)} = if {cond()} {{ {src} }} else {{ {self.expr(pt, others, d - 1)} }};"]
            else:
                out = [f"{pad}let {w}: {self.rs(pt)} = {src};"]
            env.append({'name': w, 't': pt, 'mut': self.p(0.5), 'ld': loop_depth})
            out[0] = out[0].replace(f"let {w}", f"let {'mut ' if env[-1]['mut'] else ''}{w}", 1)
        elif r < 0.82 and t[0] == 'enum':
            # by-value match moving one field out of a variant; the rest of the enum is dropped
            # later through the partially moved local (drop elaboration of a downcast)
            ms = [(vn, kind, i, fn_, ft) for vn, kind, fs in self.enums[t[1]]['variants']
                  for i, (fn_, ft) in enumerate(fs) if not self.is_copy(ft)]
            if not ms:
                return None
            vn, kind, i, fn_, ft = self.ch(ms)
            b, w = self.fresh("b"), self.fresh()
            if kind == 'tuple':
                nf = len(next(fs for v_, _, fs in self.enums[t[1]]['variants'] if v_ == vn))
                pat = f"{t[1]}::{vn}(" + ", ".join(b if j == i else "_" for j in range(nf)) + ")"
            else:
                pat = f"{t[1]}::{vn} {{ {fn_}: {b}, .. }}"
            benv = others + [{'name': b, 't': ft, 'mut': False}]
            out = [f"{pad}let {w}: {self.rs(ft)} = match {name} {{ {pat} => {self.expr(ft, benv, d - 1) if self.p(0.3) else b}, _ => {self.expr(ft, others, d - 1)} }};"]
            env.append({'name': w, 't': ft, 'mut': False, 'ld': loop_depth})
        elif r < 0.9 and t[0] == 'opt':
            # if let Some(w) = local { .. }: moves the local conditionally
            w = self.fresh()
            env2 = others + [{'name': w, 't': t[1], 'mut': False, 'ld': loop_depth}]
            body = self.stmts(env2, self.r.randint(1, 2), d - 1, len(pad) // 4 + 1, loop_depth)
            out = [f"{pad}if let Some({w}) = {name} {{"] + body + [f"{pad}}}"]
        elif self.p(0.3):
            # an FnOnce closure that owns the local: dropped with the closure if never called
            w, c = self.fresh(), self.fresh("c")
            res = self.ch([name, f"{{ let x = {name}; x }}", f"{{ drop({name}); {self.expr(t, self.cenv(others), d - 1)} }}"])
            if self.p(0.5):
                out = [f"{pad}let {w}: {self.rs(t)} = {{ let {c} = move || {res}; {c}() }};"]
            else:
                out = [f"{pad}let {w}: {self.rs(t)} = {{ let {c} = move || {res}; if {cond()} {{ {c}() }} else {{ {self.expr(t, others, d - 1)} }} }};"]
            env.append({'name': w, 't': t, 'mut': False, 'ld': loop_depth})
        else:
            w = self.fresh()
            out = [f"{pad}let mut {w}: {self.rs(t)} = {name};"]
            env.append({'name': w, 't': t, 'mut': True, 'ld': loop_depth})
            if v.get('mut') and self.p(0.5):
                out[0] += f" {name} = {self.expr(t, others, d - 1)};"
                keep = True
        if not keep:
            v['t'] = ('moved',)
            v['mut'] = False
        return out

    def emit_types(self):
        if self.droppy:
            self.emit_drop_type()
        for name, fields in self.structs.items():
            t = ('struct', name)
            derive = "Clone, Copy, Debug, PartialEq" if self.is_copy(t) else "Clone, Debug, PartialEq"
            self.out.append(f"#[derive({derive})]")
            self.out.append(f"struct {name} {{ " + " ".join(f"{f}: {self.rs(ft)}," for f, ft in fields) + " }")
        for name, e in self.enums.items():
            t = ('enum', name)
            derive = "Clone, Copy, Debug, PartialEq" if self.is_copy(t) else "Clone, Debug, PartialEq"
            self.out.append(f"#[derive({derive})]")
            if e['repr']:
                self.out.append(f"#[repr({e['repr']})]")
            vs = []
            for (vn, kind, fs), d in zip(e['variants'], e['discr']):
                s = vn
                if kind == 'tuple':
                    s += "(" + ", ".join(self.rs(ft) for _, ft in fs) + ")"
                elif kind == 'named':
                    s += " { " + ", ".join(f"{fn_}: {self.rs(ft)}" for fn_, ft in fs) + " }"
                if d is not None:
                    s += f" = {d}"
                vs.append(s)
            self.out.append(f"enum {name} {{ " + ", ".join(vs) + " }")

    def emit_trait(self):
        """trait Tr, impls for some types, generic/dyn helpers (monomorphisation, inlining, devirtualisation)."""
        self.impls = []
        if not self.p(0.6) or self.cmode:
            return
        cands = [('struct', n) for n in sorted(self.structs)] + [('enum', n) for n in sorted(self.enums)] + \
                [I(n) for n in sorted(set(self.int_pool))]
        self.r.shuffle(cands)
        self.impls = cands[:self.r.randint(1, len(cands))]
        self.out.append("trait Tr {")
        self.out.append("    fn k(&self) -> i64 { 7 }")
        self.out.append("    fn m(&self, x: i64) -> i64;")
        self.out.append("    fn d(&self, x: i64) -> i64 { self.m(x).wrapping_mul(3) ^ self.k() }")
        self.out.append("}")
        self.mult, self.cur_cost = 1, 0
        impls, self.impls = self.impls, []   # no trait calls inside impls: no recursion
        for t in impls:
            env = [{'name': 'x', 't': I('i64'), 'mut': False}, {'name': '(*self)', 't': t, 'mut': False}]
            self.out.append(f"impl Tr for {self.rs(t)} {{")
            if self.p(0.4):
                self.out.append(f"    fn k(&self) -> i64 {{ {self.int_lit('i64')} }}")
            attr = self.ch(["", "", "#[inline(never)] ", "#[inline(always)] "])
            self.out.append(f"    {attr}fn m(&self, x: i64) -> i64 {{ {self.expr(I('i64'), env, 2)} }}")
            if self.p(0.3):
                self.out.append(f"    fn d(&self, x: i64) -> i64 {{ {self.expr(I('i64'), env, 2)} }}")
            self.out.append("}")
        self.impls = impls
        self.out.append("fn tr_g<T: Tr>(t: &T, x: i64) -> i64 { t.m(x).wrapping_add(t.d(x ^ 1)) }")
        self.out.append("fn tr_dyn(t: &dyn Tr, x: i64) -> i64 { t.d(x).wrapping_sub(t.m(x)) }")
        self.out.append("fn tr_many<T: Tr + Clone>(t: &T, n: usize) -> i64 { (0..n).fold(t.k(), |a, i| a.wrapping_add(t.clone().m(i as i64))) }")

    def trait_call(self, env, d):
        t = self.ch(self.impls)
        v = self.expr(t, env, d - 1)
        x = self.expr(I('i64'), env, d - 1)
        r = self.r.random()
        if r < 0.25:
            return f"({v}).m({x})"
        if r < 0.45:
            return f"tr_g(&{v}, {x})"
        if r < 0.65:
            return f"tr_dyn(&{v}, {x})"
        if r < 0.8:
            return f"{{ let b: Box<dyn Tr> = Box::new({v}); b.d({x}) }}"
        if r < 0.9:
            return f"tr_many(&{v}, {self.r.randint(0, 6)})"
        return f"Tr::k(&{v})"

    def fieldless(self, ename):
        return all(k == 'unit' for _, k, _ in self.enums[ename]['variants'])

    # ---------- literals ----------
    def int_lit(self, n):
        r = self.r.random()
        if r < 0.15:
            return f"{n}::MAX" if self.p(0.6) else f"{n}::MIN"
        if r < 0.22:
            # powers of two +- 1: rounding boundaries of int->float casts (2^24, 2^53, ...), carries
            b = BITS[n] - (1 if n in SIGNED else 0)
            k = self.ch([b - 1, b - 2, 24, 25, 53, 54, 64, 65, self.r.randint(1, b)])
            v = (1 << min(k, b - 1)) + self.ch([-1, 0, 1, 3])
        elif r < 0.55:
            v = self.ch([0, 1, 2, 3, 7, 8, 15, 16, 31, 63, 100, 127, 255])
        elif r < 0.7 and n in SIGNED:
            v = -self.ch([1, 2, 3, 8, 100, 128])
        else:
            b = BITS[n]
            lo, hi = (-(1 << (b - 1)), (1 << (b - 1)) - 1) if n in SIGNED else (0, (1 << b) - 1)
            v = self.r.randint(lo, hi) if self.p(0.4) else self.r.randint(max(lo, -1000), min(hi, 1000))
        if n not in SIGNED and v < 0:
            v = -v
        b = BITS[n]
        if n in SIGNED:
            v = max(min(v, (1 << (b - 1)) - 1), -(1 << (b - 1)))
        else:
            v = min(v, (1 << b) - 1)
        return f"{v}{n}" if v >= 0 else f"({v}{n})"

    def flt_lit(self, n):
        v = self.ch(["0.0", "1.0", "-1.0", "0.5", "2.5", "-0.0", "1e10", "3.0e-5", "100.25",
                     f"{n}::INFINITY", f"{n}::NEG_INFINITY", f"{n}::NAN", f"{n}::MAX", f"{n}::MIN_POSITIVE",
                     # subnormals, halfway cases of float->int / f64->f32 rounding, integer-boundary values
                     "1e-310" if n == "f64" else "1e-40", "0.1", "2147483647.5", "9007199254740993.0",
                     "16777217.0", "-2.5", "4294967295.5", "1.7014118e38", f"{n}::EPSILON",
                     f"{self.r.randint(-50, 50)}.{self.r.randint(0, 99)}"])
        if "::" in v:
            return v
        return f"({v}{n})" if v.startswith("-") else f"{v}{n}"

    # ---------- environment ----------
    # env: list of dicts {name, t, mut, kind}; kind 'local' or 'ref' (for &mut params: name is deref place)
    def vars_of(self, env, t):
        return [v for v in env if v['t'] == t]

    def cenv(self, env):
        # what a `move` closure may capture: Copy locals (not derefs of reference params)
        return [v for v in env if self.is_copy(v['t']) and not v['name'].startswith('(*')]

    def read_var(self, v):
        # a read of a non-Copy place clones it
        if self.is_copy(v['t']):
            return v['name']
        return f"{v['name']}.clone()"

    # ---------- expressions ----------
    def expr(self, t, env, d):
        """An expression of type t."""
        cands = self.vars_of(env, t)
        if cands and self.p(0.45 if d > 0 else 0.85):
            return self.read_var(self.ch(cands))
        if d <= 0:
            return self.leaf(t, env)
        k = t[0]
        r = self.r.random()
        # generic forms valid for any type
        if r < 0.06:
            c = self.expr(BOOL, env, d - 1)
            return f"(if ({c}) {{ {self.expr(t, env, d - 1)} }} else {{ {self.expr(t, env, d - 1)} }})"
        if r < 0.10:
            return self.block_expr(t, env, d)
        if r < 0.15:
            e = self.from_projection(t, env, d)
            if e:
                return e
        if r < 0.20:
            e = self.match_expr(t, env, d)
            if e:
                return e
        if r < 0.25:
            e = self.call_expr(t, env, d)
            if e:
                return e
        if r < 0.27:
            return f"std::hint::black_box({self.expr(t, env, d - 1)})"
        if r < 0.30 and self.is_copy(t) and t[0] != 'flt' and not self.cmode:
            # closure capturing Copy locals by move, called right away or bound
            cenv = self.cenv(env)
            at = self.scalar_type()
            a = self.fresh("a")
            body = self.expr(t, cenv + [{'name': a, 't': at, 'mut': False}], d - 1)
            return f"(move |{a}: {self.rs(at)}| -> {self.rs(t)} {{ {body} }})({self.expr(at, env, d - 1)})"
        if k == 'int':
            return self.int_expr(t[1], env, d)
        if k == 'bool':
            return self.bool_expr(env, d)
        if k == 'flt':
            return self.flt_expr(t[1], env, d)
        if k == 'arr':
            return self.arr_expr(t, env, d)
        if k == 'tup':
            return "(" + "".join(self.expr(x, env, d - 1) + ", " for x in t[1]) + ")"
        if k == 'struct':
            fields = self.structs[t[1]]
            srcs = self.vars_of(env, t)
            if srcs and self.p(0.3) and len(fields) > 1:
                f, ft = self.ch(fields)
                return f"{t[1]} {{ {f}: {self.expr(ft, env, d - 1)}, ..{self.read_var(self.ch(srcs))} }}"
            return f"{t[1]} {{ " + " ".join(f"{f}: {self.expr(ft, env, d - 1)}," for f, ft in fields) + " }"
        if k == 'enum':
            return self.enum_ctor(t[1], env, d)
        if k == 'opt':
            return self.opt_expr(t, env, d)
        if k == 'box':
            return f"Box::new({self.expr(t[1], env, d - 1)})"
        if k == 'vec':
            return self.vec_expr(t, env, d)
        if k == 'drop':
            return f"D::new({self.expr(I('i64'), env, d - 1)})"
        raise ValueError(t)

    def leaf(self, t, env):
        cands = self.vars_of(env, t)
        if cands and self.p(0.7):
            return self.read_var(self.ch(cands))
        k = t[0]
        if k == 'int':
            # projections from variables are cheap leaves too
            if self.p(0.3):
                e = self.from_projection(t, env, 0)
                if e:
                    return e
            return self.int_lit(t[1])
        if k == 'bool':
            return self.ch(["true", "false"])
        if k == 'flt':
            return self.flt_lit(t[1])
        if k == 'arr':
            if self.is_copy(t[1]):
                return f"[{self.leaf(t[1], env)}; {t[2]}]"
            return "[" + ", ".join(self.leaf(t[1], env) for _ in range(t[2])) + "]"
        if k == 'tup':
            return "(" + "".join(self.leaf(x, env) + ", " for x in t[1]) + ")"
        if k == 'struct':
            return f"{t[1]} {{ " + " ".join(f"{f}: {self.leaf(ft, env)}," for f, ft in self.structs[t[1]]) + " }"
        if k == 'enum':
            return self.enum_ctor(t[1], env, 0)
        if k == 'opt':
            return f"None::<{self.rs(t[1])}>" if self.p(0.4) else f"Some({self.leaf(t[1], env)})"
        if k == 'box':
            return f"Box::new({self.leaf(t[1], env)})"
        if k == 'vec':
            return f"Vec::<{self.rs(t[1])}>::new()" if self.p(0.3) else f"vec![{', '.join(self.leaf(t[1], env) for _ in range(self.r.randint(1, 4)))}]"
        if k == 'drop':
            return f"D::new({self.int_lit('i64')})"
        raise ValueError(t)

    def enum_ctor(self, ename, env, d):
        vn, kind, fs = self.ch(self.enums[ename]['variants'])
        sub = (lambda ft: self.expr(ft, env, d - 1)) if d > 0 else (lambda ft: self.leaf(ft, env))
        if kind == 'unit':
            return f"{ename}::{vn}"
        if kind == 'tuple':
            return f"{ename}::{vn}(" + ", ".join(sub(ft) for _, ft in fs) + ")"
        return f"{ename}::{vn} {{ " + ", ".join(f"{fn_}: {sub(ft)}" for fn_, ft in fs) + " }"

    def any_int(self):
        return self.ch(self.int_pool) if self.p(0.7) else self.ch(INTS)

    def unwrap_or(self, opt, dflt):
        """`opt.unwrap_or(dflt)` (a match in const programs: Option::unwrap_or may drop, not const)."""
        if self.cmode:
            x = self.fresh("x")
            return f"(match {opt} {{ Some({x}) => {x}, None => {dflt} }})"
        return f"{opt}.unwrap_or({dflt})"

    def int_method(self, n, env, d):
        """Newer integer methods (each a const fn and an intrinsic or a short bit trick in core)."""
        sub = lambda: self.expr(I(n), env, d - 1)
        u32e = lambda: self.expr(I('u32'), env, d - 1)
        sg = n in SIGNED
        other = ('u' + n[1:]) if sg else ('i' + n[1:])   # same width, other signedness
        r = self.r.random()
        if r < 0.10:
            return self.unwrap_or(f"({sub()}).checked_isqrt()", sub()) if sg else f"({sub()}).isqrt()"
        if r < 0.20:
            m = self.ch(["checked_ilog2()", "checked_ilog10()", f"checked_ilog(({sub()}) | 2)"])
            return self.unwrap_or(f"({sub()}).{m}.map(|v| v as {n})" if not self.cmode else
                                  f"(match ({sub()}).{m} {{ Some(v) => Some(v as {n}), None => None }})", sub())
        if r < 0.28:
            return f"(({sub()}).abs_diff({sub()}) as {n})"
        if r < 0.36:
            return f"({sub()}).midpoint({sub()})"
        if r < 0.44:
            m = self.ch(["leading_ones", "trailing_ones"])
            return f"(({sub()}).{m}() as {n})"
        if r < 0.52:
            m = self.ch(["unbounded_shl", "unbounded_shr", "wrapping_pow", "saturating_pow"])
            return f"({sub()}).{m}(({u32e()}) % {BITS[n] + 8})"
        if r < 0.58:
            return self.unwrap_or(f"({sub()}).checked_pow(({u32e()}) % {BITS[n] + 2})", sub())
        if r < 0.66:
            bf = self.ch(["to_le_bytes", "to_be_bytes", "to_ne_bytes"])
            bt = self.ch(["from_le_bytes", "from_be_bytes", "from_ne_bytes"])
            return f"{n}::{bt}(({sub()}).{bf}())"
        if r < 0.72:
            m = "cast_unsigned" if sg else "cast_signed"
            return f"(({sub()}).{m}() as {n})"
        if r < 0.80:
            if sg:
                m = self.ch(["wrapping_add_unsigned", "wrapping_sub_unsigned", "saturating_add_unsigned",
                             "saturating_sub_unsigned"])
            else:
                m = self.ch(["wrapping_add_signed", "saturating_add_signed", "wrapping_sub_signed",
                             "saturating_sub_signed"])
            return f"({sub()}).{m}({self.expr(I(other), env, d - 1)})"
        if sg:
            m = self.ch(["unsigned_abs", "saturating_div", "checked_neg", "checked_abs", "checked_rem_euclid",
                         "saturating_neg", "saturating_abs"])
            if m == "unsigned_abs":
                return f"(({sub()}).unsigned_abs() as {n})"
            if m == "saturating_div":
                return f"({sub()}).saturating_div(({sub()}) | 1)"
            if m in ("saturating_neg", "saturating_abs"):
                return f"({sub()}).{m}()"
            if m == "checked_rem_euclid":
                return self.unwrap_or(f"({sub()}).checked_rem_euclid({sub()})", sub())
            return self.unwrap_or(f"({sub()}).{m}()", sub())
        m = self.ch(["div_ceil", "checked_next_multiple_of", "checked_next_power_of_two", "is_power_of_two"])
        if m == "div_ceil":
            return f"({sub()}).div_ceil(({sub()}) | 1)"
        if m == "checked_next_multiple_of":
            return self.unwrap_or(f"({sub()}).checked_next_multiple_of({sub()})", sub())
        if m == "is_power_of_two":
            return f"(({sub()}).is_power_of_two() as {n})"
        return self.unwrap_or(f"({sub()}).checked_next_power_of_two()", sub())

    def int_expr(self, n, env, d):
        sub = lambda: self.expr(I(n), env, d - 1)
        if self.panicky and self.p(0.12):
            return self.panicky_int(n, env, d)
        if self.p(0.1):
            return self.int_method(n, env, d)
        r = self.r.random()
        if r < 0.22:
            op = self.ch(["wrapping_add", "wrapping_sub", "wrapping_mul", "wrapping_add", "wrapping_sub",
                          "saturating_add", "saturating_sub", "saturating_mul", "min", "max"])
            if op in ("min", "max") and self.cmode:
                # Ord::min/max are not const: select by hand
                a, b = self.fresh("a"), self.fresh("b")
                return f"{{ let {a}: {n} = {sub()}; let {b}: {n} = {sub()}; if {a} {'<' if op == 'min' else '>'} {b} {{ {a} }} else {{ {b} }} }}"
            return f"({sub()}).{op}({sub()})"
        if r < 0.30:
            op = self.ch(["&", "|", "^"])
            return f"({sub()} {op} {sub()})"
        if r < 0.36:
            op = self.ch(["wrapping_div", "wrapping_rem", "wrapping_div_euclid", "wrapping_rem_euclid"])
            return f"({sub()}).{op}(({sub()}) | 1)"
        if r < 0.42:
            op = self.ch(["wrapping_shl", "wrapping_shr", "rotate_left", "rotate_right"])
            return f"({sub()}).{op}(({self.expr(I('u32'), env, d - 1)}) % {BITS[n] * 2})"
        if r < 0.47:
            op = self.ch(["count_ones", "leading_zeros", "trailing_zeros", "count_zeros"])
            return f"(({sub()}).{op}() as {n})"
        if r < 0.51:
            op = self.ch(["swap_bytes", "reverse_bits", "wrapping_neg", "to_be", "to_le"] +
                         (["wrapping_abs", "signum"] if n in SIGNED else []))
            return f"({sub()}).{op}()"
        if r < 0.55:
            op = self.ch(["checked_add", "checked_sub", "checked_mul", "checked_div", "checked_rem"])
            return self.unwrap_or(f"({sub()}).{op}({sub()})", sub())
        if r < 0.58:
            op = self.ch(["overflowing_add", "overflowing_sub", "overflowing_mul"])
            return f"(({sub()}).{op}({sub()}).{self.ch(['0', '1 as ' + n])})"
        if r < 0.62:
            return f"(!{sub()})"
        if r < 0.72:
            src = self.ch([I(self.any_int()), I(self.any_int()), BOOL, ('flt', self.ch(FLOATS))])
            if src == BOOL:
                return f"({self.expr(src, env, d - 1)} as {n})"
            return f"(({self.expr(src, env, d - 1)}) as {n})"
        if r < 0.76:
            # fieldless enum -> int
            fl = [e for e in self.enums if self.fieldless(e)]
            if fl:
                return f"(({self.expr(('enum', self.ch(fl)), env, d - 1)}) as {n})"
        if r < 0.84 and not self.cmode:
            if self.impls and self.p(0.3):
                return f"(({self.trait_call(env, d)}) as {n})"
            return self.iter_expr(n, env, d)
        if r < 0.88:
            vs = [v for v in env if v['t'][0] == 'vec']
            if vs:
                v = self.ch(vs)
                if self.p(0.5):
                    return f"({v['name']}.len() as {n})"
                if v['t'][1] == I(n):
                    return f"{v['name']}.get(({self.expr(I('usize'), env, d - 1)}) % 8).copied().unwrap_or({sub()})"
        if r < 0.92:
            bs = [v for v in env if v['t'] == ('box', I(n))]
            if bs:
                return f"(*{self.ch(bs)['name']})"
        if r < 0.96:
            return self.loop_expr(I(n), env, d)
        return self.int_lit(n)

    def panicky_int(self, n, env, d):
        sub = lambda: self.expr(I(n), env, d - 1)
        r = self.r.random()
        if r < 0.45:
            return f"({sub()} {self.ch(['+', '-', '*', '+', '-', '/', '%'])} {sub()})"
        if r < 0.55:
            return f"({sub()} {self.ch(['<<', '>>'])} ({self.expr(I('u32'), env, d - 1)} % {BITS[n] + 2}))"
        if r < 0.62 and n in SIGNED:
            return self.ch([f"(-{sub()})", f"({sub()}).abs()"])
        if r < 0.75:
            arrs = [v for v in env if v['t'][0] in ('arr', 'vec') and v['t'][1] == I(n)]
            if arrs:
                v = self.ch(arrs)
                return f"{v['name']}[({self.expr(I('usize'), env, d - 1)}) % {self.r.randint(1, 20)}]"
        if r < 0.85:
            return f"({self.expr(('opt', I(n)), env, d - 1)}).unwrap()"
        if r < 0.92:
            return f"({sub()}).pow(({self.expr(I('u32'), env, d - 1)}) % 9)"
        return f"[{sub()}, {sub()}].iter().{self.ch(['sum', 'product'])}::<{n}>()"

    def iter_expr(self, n, env, d):
        old = self.mult
        self.mult *= 13   # calls inside the closures run once per element
        try:
            return self._iter_expr(n, env, d)
        finally:
            self.mult = old

    def _iter_expr(self, n, env, d):
        """Iterator chains over arrays/vecs/ranges."""
        arrs = [v for v in env if v['t'][0] in ('arr', 'vec') and v['t'][1][0] == 'int']
        a = self.fresh("x")
        if arrs and self.p(0.7):
            v = self.ch(arrs)
            et = v['t'][1]
            src = f"{v['name']}.iter()"
            pat = f"&{a}"
        else:
            et = I('usize') if self.p(0.5) else I(n)
            lo = self.r.randint(0, 3)
            hi = lo + self.r.randint(0, 12)
            src = f"({lo}{et[1]}..{hi}{et[1]})" if self.p(0.7) else f"({lo}{et[1]}..={hi}{et[1]})"
            pat = a
        ienv = self.cenv(env) + [{'name': a, 't': et, 'mut': False}]
        chain = src
        if self.p(0.3):
            chain += ".rev()"
        for _ in range(self.r.randint(0, 2)):
            r = self.r.random()
            if r < 0.5:
                chain += f".skip({self.r.randint(0, 3)})"
            elif r < 0.7:
                chain += f".take({self.r.randint(0, 6)})"
            elif r < 0.85:
                chain += f".step_by({self.r.randint(1, 3)})"
            else:
                chain += f".filter(|&{'&' if pat.startswith('&') else ''}{a}| {self.expr(BOOL, ienv, d - 1)})"
        r = self.r.random()
        if r < 0.45:
            acc = self.fresh("acc")
            fenv = ienv + [{'name': acc, 't': I(n), 'mut': False}]
            return f"{chain}.fold({self.expr(I(n), env, d - 1)}, |{acc}, {pat}| {self.expr(I(n), fenv, d - 1)})"
        if r < 0.6:
            return f"({chain}.count() as {n})"
        if r < 0.75:
            return f"({chain}.map(|{pat}| {self.expr(I(n), ienv, d - 1)}).max().unwrap_or({self.int_lit(n)}))"
        if r < 0.85:
            return f"({chain}.position(|{pat}| {self.expr(BOOL, ienv, d - 1)}).map_or({self.int_lit(n)}, |p| p as {n}))"
        return f"({chain}.map(|{pat}| ({self.expr(I(n), ienv, d - 1)}) as {n}).min().unwrap_or({self.int_lit(n)}))"

    def loop_expr(self, t, env, d):
        old = self.mult
        self.mult *= 11
        try:
            return self._loop_expr(t, env, d)
        finally:
            self.mult = old

    def _loop_expr(self, t, env, d):
        """`loop` with a counter and break-with-value."""
        c = self.fresh("c")
        k = self.r.randint(1, 10)
        acc = self.fresh("acc")
        lenv = env + [{'name': c, 't': I('u32'), 'mut': False}, {'name': acc, 't': t, 'mut': False}]
        lab = self.fresh("'l")
        return (f"{{ let mut {c} = 0u32; let mut {acc} = {self.expr(t, env, d - 1)}; {lab}: loop {{ "
                f"if {c} >= {k} || ({self.expr(BOOL, lenv, d - 1)}) {{ break {lab} {acc}; }} "
                f"{acc} = {self.expr(t, lenv, d - 1)}; {c} += 1; }} }}")

    def bool_expr(self, env, d):
        r = self.r.random()
        if r < 0.3:
            n = self.any_int()
            op = self.ch(["<", "<=", ">", ">=", "==", "!="])
            return f"({self.expr(I(n), env, d - 1)} {op} {self.expr(I(n), env, d - 1)})"
        if r < 0.45:
            op = self.ch(["&&", "||", "^", "&", "|"])
            return f"({self.expr(BOOL, env, d - 1)} {op} {self.expr(BOOL, env, d - 1)})"
        if r < 0.5:
            return f"(!{self.expr(BOOL, env, d - 1)})"
        if r < 0.6:
            # equality of aggregates
            t = self.rand_type(1)
            if not self.has_float(t) and (not self.cmode or t[0] in ('int', 'bool')):
                return f"({self.expr(t, env, d - 1)} == {self.expr(t, env, d - 1)})"
        if r < 0.7 and self.enums:
            ename = self.ch(sorted(self.enums))
            vn, kind, fs = self.ch(self.enums[ename]['variants'])
            pat = f"{ename}::{vn}" + {"unit": "", "tuple": "(..)", "named": " { .. }"}[kind]
            return f"matches!({self.expr(('enum', ename), env, d - 1)}, {pat})"
        if r < 0.78:
            t = ('opt', self.rand_type(0))
            return f"{self.expr(t, env, d - 1)}.{self.ch(['is_some', 'is_none'])}()"
        if r < 0.85:
            f = self.ch(FLOATS)
            op = self.ch(["<", "<=", ">", ">=", "==", "!="])
            return f"({self.expr(('flt', f), env, d - 1)} {op} {self.expr(('flt', f), env, d - 1)})"
        if r < 0.9:
            f = self.ch(FLOATS)
            m = self.ch(['is_nan', 'is_finite', 'is_sign_negative', 'is_infinite'])
            if m == 'is_sign_negative':
                # the sign of a NaN is nondeterministic (RFC 3514): x86 SSE's default NaN is
                # negative, a const-folded one positive. Never observe it.
                return f"{{ let s: {f} = {self.expr(('flt', f), env, d - 1)}; !s.is_nan() && s.is_sign_negative() }}"
            return f"({self.expr(('flt', f), env, d - 1)}).{m}()"
        return self.ch(["true", "false"])

    def flt_expr(self, n, env, d):
        sub = lambda: self.expr(('flt', n), env, d - 1)
        r = self.r.random()
        if r < 0.4:
            return f"({sub()} {self.ch(['+', '-', '*', '/'])} {sub()})"
        if r < 0.5:
            return f"(-{sub()})"
        if r < 0.6:
            # sqrt is not a const fn
            m = self.ch(['sqrt', 'abs', 'floor', 'ceil', 'trunc', 'round', 'round_ties_even', 'fract', 'signum',
                         'recip'])
            if m == 'sqrt' and self.cmode:
                m = 'abs'
            return f"({sub()}).{m}()"
        if r < 0.65:
            return f"({sub()}).mul_add({sub()}, {sub()})"
        if r < 0.72:
            m = self.ch(['min', 'max', 'copysign'])
            if m == 'copysign':
                # copysign carries a NaN sign operand's (nondeterministic, RFC 3514) sign into a
                # non-NaN result; std's f32::copysign doc: "the result of copysign with sign being
                # a NaN might produce an unexpected or non-portable result". Never pass a NaN sign.
                return f"({sub()}).copysign({{ let s: {n} = {sub()}; if s.is_nan() {{ 1.0 }} else {{ s }} }})"
            # f32::min/max docs: "If the inputs compare equal (such as for the case of `+0.0` and
            # `-0.0`), either input may be returned non-deterministically." Adding +0.0 maps -0.0 to
            # +0.0 (and leaves every other value alone), so the result no longer depends on the choice.
            return f"(({sub()}).{m}({sub()}) + 0.0)"
        if r < 0.76:
            # IEEE remainder-free `%` (fmod, exact) and clamp with ordered constant bounds
            if self.p(0.6):
                return f"({sub()} % {sub()})"
            lo, hi = sorted(self.r.sample([-100.0, -1.5, -0.0, 0.0, 0.5, 1.0, 3.0, 1e6], 2))
            return f"({sub()}).clamp({lo}, {hi})"
        if r < 0.9:
            src = self.ch([I(self.any_int()), ('flt', self.ch(FLOATS))])
            return f"(({self.expr(src, env, d - 1)}) as {n})"
        return self.flt_lit(n)

    def arr_expr(self, t, env, d):
        et, n = t[1], t[2]
        r = self.r.random()
        if n > 16 and (r < 0.3 or r >= 0.9):
            r = 0.5   # no 64-element literals
        if r < 0.3:
            return "[" + ", ".join(self.expr(et, env, d - 1) for _ in range(n)) + "]"
        if r < 0.45 and self.is_copy(et):
            return f"[{self.expr(et, env, d - 1)}; {n}]"
        if self.cmode and 0.45 <= r < 0.75:
            r = 0.8   # array::from_fn / map take closures
        if r < 0.6:
            i = self.fresh("i")
            return f"core::array::from_fn::<_, {n}, _>(|{i}: usize| {self.expr(et, self.cenv(env) + [{'name': i, 't': I('usize'), 'mut': False}], d - 1)})"
        if r < 0.75:
            srcs = [v for v in env if v['t'][0] == 'arr' and v['t'][2] == n and self.is_copy(v['t'])]
            if srcs:
                s = self.ch(srcs)
                x = self.fresh("x")
                cenv = self.cenv(env) + [{'name': x, 't': s['t'][1], 'mut': False}]
                return f"{s['name']}.map(|{x}| {self.expr(et, cenv, d - 1)})"
        if r < 0.9 and self.is_copy(et):
            a = self.fresh("a")
            return (f"{{ let mut {a} = {self.expr(t, env, d - 1)}; "
                    f"{a}[(({self.expr(I('usize'), env, d - 1)}) % {n})] = {self.expr(et, env, d - 1)}; {a} }}")
        if self.is_copy(et) and self.p(0.5):
            a = self.fresh("a")
            op = self.ch(["reverse()", f"rotate_left({self.r.randint(0, n)})", f"swap({self.r.randrange(n)}, {self.r.randrange(n)})"])
            if self.cmode:
                op = self.ch(["reverse()", f"swap({self.r.randrange(n)}, {self.r.randrange(n)})"])
            elif et[0] == 'int' and self.p(0.4):
                op = self.ch(["sort()", "sort_unstable()"])
            return f"{{ let mut {a} = {self.expr(t, env, d - 1)}; {a}.{op}; {a} }}"
        return "[" + ", ".join(self.expr(et, env, d - 1) for _ in range(n)) + "]"

    def opt_expr(self, t, env, d):
        it = t[1]
        r = self.r.random()
        if r < 0.25:
            return f"None::<{self.rs(it)}>"
        if r < 0.55:
            return f"Some({self.expr(it, env, d - 1)})"
        if r < 0.65 and it[0] == 'int':
            op = self.ch(["checked_add", "checked_sub", "checked_mul", "checked_div", "checked_rem", "checked_shl"])
            arg = self.expr(I('u32') if op == 'checked_shl' else it, env, d - 1)
            return f"({self.expr(it, env, d - 1)}).{op}({arg})"
        if self.cmode and r >= 0.55:
            # get/filter/map/or are not const (closures, SliceIndex): match by hand
            x = self.fresh("x")
            return (f"(match {self.expr(t, env, d - 1)} {{ Some({x}) if {self.expr(BOOL, env + [{'name': x, 't': it, 'mut': False}], d - 1)} "
                    f"=> Some({x}), Some(_) => {self.expr(t, env, d - 1)}, None => None }})")
        if r < 0.75 and it[0] == 'int':
            arrs = [v for v in env if v['t'][0] in ('arr', 'vec') and v['t'][1] == it]
            if arrs:
                return f"{self.ch(arrs)['name']}.get(({self.expr(I('usize'), env, d - 1)}) % 20).copied()"
        if r < 0.85 and self.is_copy(it):
            x = self.fresh("x")
            cenv = self.cenv(env) + [{'name': x, 't': it, 'mut': False}]
            return f"{self.expr(t, env, d - 1)}.filter(|&{x}| {self.expr(BOOL, cenv, d - 1)})"
        if r < 0.95 and self.is_copy(it):
            x = self.fresh("x")
            st = self.rand_type(0)
            cenv = self.cenv(env) + [{'name': x, 't': st, 'mut': False}]
            return f"{self.expr(('opt', st), env, d - 1)}.map(|{x}| {self.expr(it, cenv, d - 1)})"
        return f"{self.expr(t, env, d - 1)}.or({self.expr(t, env, d - 1)})"

    def vec_expr(self, t, env, d):
        et = t[1]
        r = self.r.random()
        if r < 0.3:
            k = self.r.randint(0, 5)
            if k == 0:
                return f"Vec::<{self.rs(et)}>::new()"
            return f"vec![{', '.join(self.expr(et, env, d - 1) for _ in range(k))}]"
        if r < 0.6:
            i = self.fresh("i")
            cenv = self.cenv(env) + [{'name': i, 't': I('usize'), 'mut': False}]
            return f"(0..{self.r.randint(0, 9)}usize).map(|{i}| {self.expr(et, cenv, d - 1)}).collect::<Vec<_>>()"
        arrs = [v for v in env if v['t'][0] == 'arr' and v['t'][1] == et]
        if arrs:
            return f"{self.ch(arrs)['name']}.to_vec()"
        vs = [v for v in env if v['t'] == t]
        if vs:
            return f"{vs[0]['name']}.iter().rev().copied().collect::<Vec<_>>()"
        return f"vec![{self.expr(et, env, d - 1)}; {self.r.randint(0, 6)}]"

    def from_projection(self, t, env, d):
        """Read t out of a field/element/variant of some variable."""
        opts = []
        for v in env:
            vt = v['t']
            name = v['name']
            if vt[0] == 'arr' and vt[1] == t:
                if d > 0:
                    opts.append(f"{name}[(({self.expr(I('usize'), env, d - 1)}) % {vt[2]})]")
                opts.append(f"{name}[{self.r.randrange(vt[2])}]")
            elif vt[0] == 'tup':
                for i, x in enumerate(vt[1]):
                    if x == t:
                        opts.append(f"{name}.{i}")
            elif vt[0] == 'struct':
                for f, ft in self.structs[vt[1]]:
                    if ft == t:
                        opts.append(f"{name}.{f}")
            elif vt[0] == 'opt' and vt[1] == t and d > 0:
                opts.append(self.unwrap_or(self.read_var(v), self.expr(t, env, d - 1)))
            elif vt[0] == 'box' and vt[1] == t:
                opts.append(f"(*{name})")
            elif vt[0] == 'drop' and t == I('i64'):
                opts.append(f"{name}.0")
        if not opts:
            return None
        e = self.ch(opts)
        if not self.is_copy(t):
            e = f"{e}.clone()"
        return e

    def match_expr(self, t, env, d):
        r = self.r.random()
        if r < 0.55 and self.enums:
            ename = self.ch(sorted(self.enums))
            et = ('enum', ename)
            scrut_vars = self.vars_of(env, et)
            scrut = scrut_vars and self.p(0.7) and self.ch(scrut_vars)['name'] or self.expr(et, env, d - 1)
            arms = []
            guarded = False
            variants = list(self.enums[ename]['variants'])
            self.r.shuffle(variants)
            for vn, kind, fs in variants:
                if self.p(0.2) and len(arms) > 0:
                    continue   # covered by the wildcard
                aenv = list(env)
                binds = []
                for i, (fn_, ft) in enumerate(fs):
                    if self.p(0.7):
                        b = self.fresh("b")
                        binds.append(b)
                        # binding by reference when matching a place: use `ref` so non-Copy fields don't move
                        aenv.append({'name': b if self.is_copy(ft) else f"(*{b})", 't': ft, 'mut': False})
                    else:
                        binds.append("_")
                if kind == 'unit':
                    pat = f"{ename}::{vn}"
                elif kind == 'tuple':
                    pat = f"{ename}::{vn}(" + ", ".join(b if b == "_" else f"ref {b}" if not self.is_copy(ft) else b for b, (_, ft) in zip(binds, fs)) + ")"
                else:
                    pat = f"{ename}::{vn} {{ " + ", ".join(f"{fn_}: " + (b if b == "_" else f"ref {b}" if not self.is_copy(ft) else b) for b, (fn_, ft) in zip(binds, fs)) + " }"
                guard = ""
                if self.p(0.2):
                    guard = f" if ({self.expr(BOOL, aenv, d - 1)})"
                    guarded = True
                arms.append(f"{pat}{guard} => {self.expr(t, aenv, d - 1)},")
            if guarded or len(arms) < len(variants):
                arms.append(f"_ => {self.expr(t, env, d - 1)},")
            if not self.is_copy(et):
                scrut = f"&{scrut}" if not scrut.endswith(")") or scrut_vars else scrut
                # match on a reference; patterns auto-deref (default binding modes)
                arms = [a.replace("ref ", "") for a in arms]
                # with default binding modes all bindings are references; deref Copy ones
                return None if any(fs for _, _, fs in self.enums[ename]['variants']) else f"(match ({scrut}) {{ {' '.join(arms)} }})"
            return f"(match ({scrut}) {{ {' '.join(arms)} }})"
        if r < 0.8:
            n = self.any_int()
            scrut = self.expr(I(n), env, d - 1)
            arms = []
            used = set()
            for _ in range(self.r.randint(1, 4)):
                lit = self.int_lit(n)
                if lit in used:
                    continue
                used.add(lit)
                if self.p(0.3) and "::" not in lit and not lit.startswith("("):
                    hi = self.int_lit(n)
                    if "::" not in hi and not hi.startswith("("):
                        a, b = sorted([int(lit[:-len(n)]), int(hi[:-len(n)])])
                        arms.append(f"{a}{n}..={b}{n} => {self.expr(t, env, d - 1)},")
                        continue
                arms.append(f"{lit} => {self.expr(t, env, d - 1)},")
            arms.append(f"_ => {self.expr(t, env, d - 1)},")
            return f"(match ({scrut}) {{ {' '.join(arms)} }})"
        # match on a tuple of two enums / option
        if self.enums:
            e1, e2 = self.ch(sorted(self.enums)), self.ch(sorted(self.enums))
            if self.is_copy(('enum', e1)) and self.is_copy(('enum', e2)):
                arms = []
                for _ in range(self.r.randint(1, 4)):
                    v1 = self.ch(self.enums[e1]['variants'])
                    v2 = self.ch(self.enums[e2]['variants'])
                    p1 = f"{e1}::{v1[0]}" + {"unit": "", "tuple": "(..)", "named": " { .. }"}[v1[1]]
                    p2 = f"{e2}::{v2[0]}" + {"unit": "", "tuple": "(..)", "named": " { .. }"}[v2[1]]
                    if self.p(0.3):
                        p2 = "_"
                    arms.append(f"({p1}, {p2}) => {self.expr(t, env, d - 1)},")
                arms.append(f"_ => {self.expr(t, env, d - 1)},")
                return f"(match ({self.expr(('enum', e1), env, d - 1)}, {self.expr(('enum', e2), env, d - 1)}) {{ {' '.join(arms)} }})"
        return None

    def call_expr(self, t, env, d):
        cands = [f for f in self.fns if f['ret'] == t and not f['mutref']]
        if not cands:
            return None
        f = self.ch(cands)
        if f['cost'] * self.mult > self.cost_budget:
            return None
        self.cur_cost += f['cost'] * self.mult
        return f"{f['name']}({', '.join(self.arg(pt, mode, env, d) for _, pt, mode in f['params'])})"

    def arg(self, pt, mode, env, d):
        if mode == 'val':
            return self.expr(pt, env, d - 1)
        if mode == 'ref':
            vs = self.vars_of(env, pt)
            if vs and self.p(0.7):
                return f"&{self.ch(vs)['name']}"
            return f"&{self.expr(pt, env, d - 1)}"
        if mode == 'slice':
            vs = [v for v in env if v['t'][0] in ('arr', 'vec') and v['t'][1] == pt]
            if vs:
                v = self.ch(vs)
                if self.p(0.3):
                    lo = self.r.randint(0, 2)
                    return f"{v['name']}.get({lo}..).unwrap_or(&[])"
                return f"&{v['name']}[..]"
            return f"&[{', '.join(self.expr(pt, env, d - 1) for _ in range(self.r.randint(0, 4)))}][..]"
        raise ValueError(mode)

    def block_expr(self, t, env, d):
        env2 = list(env)
        stmts = []
        for _ in range(self.r.randint(1, 3)):
            vt = self.rand_type(1)
            name = self.fresh()
            stmts.append(f"let {name}: {self.rs(vt)} = {self.expr(vt, env2, d - 1)};")
            env2.append({'name': name, 't': vt, 'mut': False})
        return "{ " + " ".join(stmts) + f" {self.expr(t, env2, d - 1)} }}"

    # ---------- statements ----------
    def stmts(self, env, n, d, ind, loop_depth):
        out = []
        for _ in range(n):
            out += self.stmt(env, d, ind, loop_depth)
        return out

    def assignable(self, env):
        return [v for v in env if v.get('mut')]

    def stmt(self, env, d, ind, loop_depth):
        pad = "    " * ind
        r = self.r.random()
        muts = self.assignable(env)
        if self.droppy and self.p(0.25):
            out = self.move_stmt(env, d, pad, loop_depth)
            if out:
                return out
        if r < 0.05:
            # move chain: a temporary moved into a new local (or a tuple/struct), old name dead
            t = self.rand_type(2)
            a, b = self.fresh(), self.fresh()
            line = f"{pad}let {a}: {self.rs(t)} = {self.expr(t, env, d)}; let mut {b} = {a};"
            if self.p(0.4):
                c = self.fresh()
                line += f" let {c} = ({b}, {self.expr(BOOL, env, d - 1)}); let mut {b} = {c}.0;"
            env.append({'name': b, 't': t, 'mut': True, 'ld': loop_depth})
            return [line]
        if r < 0.32 or not muts:
            t = self.rand_type(2)
            name = self.fresh()
            mut = self.p(0.6)
            line = f"{pad}let {'mut ' if mut else ''}{name}: {self.rs(t)} = {self.expr(t, env, d)};"
            env.append({'name': name, 't': t, 'mut': mut, 'ld': loop_depth})
            return [line]
        if r < 0.55:
            v = self.ch(muts)
            return [pad + self.assign(v, env, d)]
        if r < 0.63:
            c = self.expr(BOOL, env, d - 1)
            then = self.stmts(list(env), self.r.randint(1, 3), d - 1, ind + 1, loop_depth)
            els = self.stmts(list(env), self.r.randint(0, 2), d - 1, ind + 1, loop_depth) if self.p(0.6) else []
            out = [f"{pad}if ({c}) {{"] + then
            if els:
                out += [f"{pad}}} else {{"] + els
            return out + [f"{pad}}}"]
        if r < 0.73 and loop_depth < 2 and self.cmode:
            r = 0.75   # no `for` in const fn (Iterator is not const): a while loop instead
        if r < 0.73 and loop_depth < 2:
            k = self.r.randint(0, 9)
            old = self.mult
            self.mult *= max(k, 1)
            i = self.fresh("i")
            env2 = list(env)
            kind = self.r.random()
            arrs = [v for v in env if v['t'][0] == 'arr' and self.is_copy(v['t'])]
            if kind < 0.6 or not arrs:
                head = f"for {i} in 0..{k}usize {{" if self.p(0.7) else f"for {i} in (0..{k}usize).rev() {{"
                env2.append({'name': i, 't': I('usize'), 'mut': False})
            else:
                a = self.ch(arrs)
                self.mult = old * a['t'][2]
                j = self.fresh("j")
                head = f"for ({j}, {i}) in {a['name']}.into_iter().enumerate() {{"
                env2.append({'name': i, 't': a['t'][1], 'mut': False})
                env2.append({'name': j, 't': I('usize'), 'mut': False})
            env_head = list(env2)
            body = self.stmts(env2, self.r.randint(1, 4), d - 1, ind + 1, loop_depth + 1)
            if self.p(0.25):
                body.insert(0 if self.p(0.5) else len(body), f"{pad}    if {self.expr(BOOL, env_head, d - 1)} {{ {self.ch(['break', 'continue'])}; }}")
            self.mult = old
            return [pad + head] + body + [f"{pad}}}"]
        if r < 0.78 and loop_depth < 2:
            c = self.fresh("w")
            k = self.r.randint(0, 8)
            old = self.mult
            self.mult *= max(k, 1)
            env2 = list(env) + [{'name': c, 't': I('u32'), 'mut': False}]
            body = self.stmts(env2, self.r.randint(1, 3), d - 1, ind + 1, loop_depth + 1)
            self.mult = old
            return [f"{pad}let mut {c} = 0u32;", f"{pad}while {c} < {k} {{"] + body + [f"{pad}    {c} += 1;", f"{pad}}}"]
        if r < 0.84:
            # swap / replace / take through &mut
            v = self.ch(muts)
            others = [o for o in muts if o['t'] == v['t'] and o['name'] != v['name'] and not o['name'].startswith('(*')]
            if others and not v['name'].startswith('(*') and self.p(0.5):
                return [f"{pad}std::mem::swap(&mut {v['name']}, &mut {self.ch(others)['name']});"]
            tmp = self.fresh()
            e = self.expr(v['t'], [o for o in env if o['name'] != v['name']], d - 1)
            env.append({'name': tmp, 't': v['t'], 'mut': False})
            return [f"{pad}let {tmp} = std::mem::replace(&mut {v['name']}, {e});"]
        if r < 0.9:
            # call a function that mutates through &mut
            cands = [f for f in self.fns if f['mutref']]
            if cands:
                f = self.ch(cands)
                mt = f['params'][0][1]
                targets = [m for m in muts if m['t'] == mt]
                if targets and f['cost'] * self.mult <= self.cost_budget:
                    self.cur_cost += f['cost'] * self.mult
                    tgt = self.ch(targets)
                    args = [self.arg(pt, mode, [e for e in env if e['name'] != tgt['name']], d) for _, pt, mode in f['params'][1:]]
                    ret = self.fresh()
                    env.append({'name': ret, 't': f['ret'], 'mut': False})
                    return [f"{pad}let {ret} = {f['name']}(&mut {tgt['name']}{''.join(', ' + a for a in args)});"]
            v = self.ch(muts)
            return [pad + self.assign(v, env, d)]
        if r < 0.95:
            # reborrow then write through it
            v = self.ch(muts)
            if v['name'].startswith('(*'):
                return [pad + self.assign(v, env, d)]
            rr = self.fresh("r")
            others = [e for e in env if e['name'] != v['name']]
            val = self.expr(v['t'], others + [{'name': f"(*{rr})", 't': v['t'], 'mut': False}], d - 1)
            return [f"{pad}{{ let {rr} = &mut {v['name']}; *{rr} = {val}; }}"]
        # vec push / place write
        vs = [m for m in muts if m['t'][0] == 'vec' and not m['name'].startswith('(*')]
        if vs and loop_depth == 0:
            v = self.ch(vs)
            return [f"{pad}{v['name']}.push({self.expr(v['t'][1], [e for e in env if e['name'] != v['name']], d - 1)});"]
        v = self.ch(muts)
        return [pad + self.assign(v, env, d)]

    def assign(self, v, env, d):
        t = v['t']
        name = v['name']
        r = self.r.random()
        if t[0] == 'arr' and r < 0.6:
            return f"{name}[(({self.expr(I('usize'), env, d - 1)}) % {t[2]})] = {self.expr(t[1], env, d - 1)};"
        if t[0] == 'struct' and r < 0.6:
            f, ft = self.ch(self.structs[t[1]])
            return f"{name}.{f} = {self.expr(ft, env, d - 1)};"
        if t[0] == 'tup' and r < 0.6:
            i = self.r.randrange(len(t[1]))
            return f"{name}.{i} = {self.expr(t[1][i], env, d - 1)};"
        if t[0] == 'int' and r < 0.5:
            op = self.ch(["wrapping_add", "wrapping_sub", "wrapping_mul", "rotate_left"])
            arg = self.expr(I('u32') if op == 'rotate_left' else t, env, d - 1)
            return f"{name} = {name}.{op}({arg});"
        if t[0] == 'int' and r < 0.6:
            return f"{name} {self.ch(['^=', '|=', '&='])} {self.expr(t, env, d - 1)};"
        if t[0] == 'opt' and r < 0.4:
            tmp = self.fresh()
            return f"if let Some({tmp}) = {name}.as_mut() {{ *{tmp} = {self.expr(t[1], [e for e in env if e['name'] != name], d - 1)}; }}"
        if t[0] == 'box' and r < 0.5:
            return f"*{name} = {self.expr(t[1], env, d - 1)};"
        if t[0] == 'drop' and r < 0.5:
            # the payload is observed by Drop: a store right before the value dies is not dead
            return f"{name}.0 = {self.expr(I('i64'), env, d - 1)};"
        return f"{name} = {self.expr(t, env, d)};"

    # ---------- functions ----------
    def make_fn(self, idx):
        name = f"f{idx}"
        params = []
        mutref = self.p(0.25)
        if mutref:
            params.append(("p0", self.rand_type(2), 'mutref'))
        for i in range(self.r.randint(0, 4)):
            pt = self.rand_type(2)
            mode = 'val'
            rr = self.r.random()
            if rr < 0.15:
                mode = 'ref'
            elif rr < 0.25 and not self.cmode:
                pt = I(self.ch(self.int_pool))
                mode = 'slice'
            params.append((f"p{len(params)}", pt, mode))
        ret = self.rand_type(2)
        env = []
        sig = []
        for pn, pt, mode in params:
            if mode == 'val':
                sig.append(f"{'mut ' if self.p(0.3) else ''}{pn}: {self.rs(pt)}")
                env.append({'name': pn, 't': pt, 'mut': sig[-1].startswith('mut')})
            elif mode == 'ref':
                sig.append(f"{pn}: &{self.rs(pt)}")
                env.append({'name': f"(*{pn})", 't': pt, 'mut': False})
            elif mode == 'mutref':
                sig.append(f"{pn}: &mut {self.rs(pt)}")
                env.append({'name': f"(*{pn})", 't': pt, 'mut': True})
            else:
                sig.append(f"{pn}: &[{self.rs(pt)}]")
                # slices: read through .len()/.get()/iter() via a local copy into a Vec
                v = self.fresh("sl")
                env.append({'name': v, 't': ('vec', pt), 'mut': False, 'init': f"let {v}: Vec<{self.rs(pt)}> = {pn}.to_vec();"})
        self.mult = 1
        self.cur_cost = 1
        body = []
        for e in env:
            if 'init' in e:
                body.append("    " + e.pop('init'))
        # slices: also index them directly with a guard
        for pn, pt, mode in params:
            if mode == 'slice':
                v = self.fresh()
                body.append(f"    let {v}: {self.rs(pt)} = if {pn}.is_empty() {{ {self.int_lit(pt[1])} }} else {{ {pn}[({self.expr(I('usize'), env, 1)}) % {pn}.len()] }};")
                env.append({'name': v, 't': pt, 'mut': False})
        body += self.stmts(env, self.r.randint(2, 9), 3, 1, 0)
        retexpr = self.expr(ret, env, 3)
        attr = self.ch(["", "", "", "#[inline(never)]\n", "#[inline]\n", "#[inline(always)]\n"])
        self.out.append(f"{attr}{'const ' if self.cmode else ''}fn {name}({', '.join(sig)}) -> {self.rs(ret)} {{")
        self.out += body
        self.out.append(f"    {retexpr}")
        self.out.append("}")
        self.fns.append({'name': name, 'params': params, 'ret': ret, 'cost': self.cur_cost + 1, 'mutref': mutref})

    def program(self):
        self.make_types()
        self.emit_types()
        self.emit_trait()
        nf = self.r.randint(3, 9)
        for i in range(nf):
            self.make_fn(i)
        self.mult = 1
        self.cur_cost = 0
        self.cost_budget = 2_000_000
        if self.cmode:
            return self.const_main()
        main = ["fn main() {"]
        if self.panicky:
            main.append("    std::panic::set_hook(Box::new(|_| {}));")
        env = []
        for f in self.fns:
            for _ in range(self.r.randint(1, 2)):
                args = []
                pre = []
                for pn, pt, mode in f['params']:
                    if mode == 'mutref':
                        v = self.fresh("m")
                        pre.append(f"    let mut {v}: {self.rs(pt)} = {self.expr(pt, env, 2)};")
                        args.append(f"&mut {v}")
                        mv = v
                    else:
                        a = self.arg(pt, mode, env, 2)
                        if self.p(0.4) and mode == 'val':
                            a = f"std::hint::black_box({a})"
                        args.append(a)
                if self.droppy:
                    main.append("    {")   # main's locals die before dlog()
                main += pre
                call = f"{f['name']}({', '.join(args)})"
                if self.panicky:
                    call = f"std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {call})).ok()"
                main.append(f"    println!(\"{{:?}}\", {call});")
                if f['mutref']:
                    main.append(f"    println!(\"{{:?}}\", {mv});")
                if self.droppy:
                    main.append("    }")
                    main.append("    dlog();")
        main.append("}")
        self.out += main
        return "\n".join(self.out) + "\n"


    def const_main(self):
        """main for const programs: each call evaluated by CTFE (`const K<k>`) and at run time on
        black_box'ed arguments. Lines `c<k>:<cls> <Debug>` / `r<k>:<cls> <Debug>` must agree."""
        main = ["fn main() {"]
        env = []
        k = 0
        for f in self.fns:
            for _ in range(self.r.randint(1, 2)):
                k += 1
                pre_c, pre_r, args_c, args_r = [], [], [], []
                rt = f['ret']
                mt = None
                for pn, pt, mode in f['params']:
                    if mode == 'mutref':
                        mt = pt
                        init = self.expr(pt, env, 2)
                        pre_c.append(f"let mut m: {self.rs(pt)} = {init};")
                        pre_r.append(f"let mut m: {self.rs(pt)} = std::hint::black_box({init});")
                        args_c.append("&mut m")
                        args_r.append("&mut m")
                    else:
                        a = self.arg(pt, mode, env, 2)
                        args_c.append(a)
                        args_r.append(f"std::hint::black_box({a})")
                call_c = f"{f['name']}({', '.join(args_c)})"
                call_r = f"{f['name']}({', '.join(args_r)})"
                if mt is not None:
                    ty = f"({self.rs(rt)}, {self.rs(mt)})"
                    val_c = f"{{ {' '.join(pre_c)} let r = {call_c}; (r, m) }}"
                    val_r = f"{{ {' '.join(pre_r)} let r = {call_r}; (r, m) }}"
                    cls_t = ('tup', (rt, mt))
                else:
                    ty, val_c, val_r, cls_t = self.rs(rt), call_c, call_r, rt
                cls = "flt" if self.has_float(cls_t) else "int128" if "128" in self.rs_deep(cls_t) else "int"
                main.append(f"    const K{k}: {ty} = {val_c};")
                main.append(f"    println!(\"c{k}:{cls} {{:?}}\", K{k});")
                main.append(f"    println!(\"r{k}:{cls} {{:?}}\", {val_r});")
        main.append("}")
        self.out.insert(0, "#![allow(long_running_const_eval)]")
        self.out += main
        return "\n".join(self.out) + "\n"

    def rs_deep(self, t):
        """Rust type text with struct/enum field types spelled out (to see what a value contains)."""
        k = t[0]
        if k == 'struct':
            return " ".join(self.rs_deep(ft) for _, ft in self.structs[t[1]])
        if k == 'enum':
            return " ".join(self.rs_deep(ft) for _, _, fs in self.enums[t[1]]['variants'] for _, ft in fs)
        if k in ('arr', 'opt', 'box', 'vec'):
            return self.rs_deep(t[1])
        if k == 'tup':
            return " ".join(self.rs_deep(x) for x in t[1])
        return self.rs(t)


def main():
    seed = int(sys.argv[1])
    sys.stdout.write("#![allow(warnings, arithmetic_overflow, unconditional_panic)]\n" + Gen(seed).program())


if __name__ == "__main__":
    main()
