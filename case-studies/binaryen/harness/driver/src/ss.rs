// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! Generator of stack-switching programs (.wat): coroutines driven by
//! scheduler exports through `resume` / `resume_throw` / `cont.bind` with
//! handlers for several tags, suspends from callees and from nested
//! sub-schedulers (forwarded to the outer handler), exceptions thrown into
//! and out of continuations, abandoned and re-resumed continuations.
//!
//! `wasm-opt -ttf` creates `cont.new` but never `resume`, `suspend` or
//! `switch`, so binaryen's stack-switching runtime (ModuleRunner doResume,
//! visitSuspend) and the optimizer on resume/suspend code were never run.
//!
//! Every program is valid by construction, all values are i32 (no NaNs),
//! every loop is bounded, there is no recursion, and every suspend has a
//! handler in its export (continuations never outlive an export: binaryen's
//! fuzz-exec clears its continuation store after each export).

use crate::Rng;
use std::fmt::Write;

struct Gen<'a> {
    rng: &'a mut Rng,
    label: u32,
    /// Functions that may be called from coroutine context, by kind.
    n_co: usize,
    n_help: usize,
    n_nest: usize,
    /// The function whose body is being generated.
    cur: Ctx,
}

const LOCALS: &[&str] = &["$x", "$t", "$u"];

impl<'a> Gen<'a> {
    fn lbl(&mut self, p: &str) -> String {
        self.label += 1;
        format!("${p}{}", self.label)
    }

    fn konst(&mut self) -> String {
        let v: i32 = match self.rng.below(6) {
            0 => self.rng.below(4) as i32,
            1 => -(self.rng.below(3) as i32) - 1,
            2 => *self.rng.pick(&[i32::MIN, i32::MAX, 0x7fff, 0x8000, 255, 256, -256]),
            _ => self.rng.below(100) as i32,
        };
        format!("(i32.const {v})")
    }

    /// Helpers the current function may call (later ones only).
    fn helpers(&self) -> Vec<usize> {
        (0..self.n_help).filter(|i| self.cur.kind != Kind::Help || *i > self.cur.index).collect()
    }

    /// An i32 expression that may suspend in the middle of evaluating its
    /// parent: the interpreter must then restore the operands evaluated
    /// before it (and the call frames) when the continuation is resumed.
    fn suspending(&mut self, depth: u32) -> String {
        let hs = self.helpers();
        let d = depth.saturating_sub(1);
        match self.rng.below(if hs.is_empty() { 4 } else { 8 }) {
            0 | 1 => format!("(suspend $y {})", self.expr(d)),
            2 => format!("(suspend $y2 {} {})", self.expr(d), self.expr(d)),
            3 => {
                let l = *self.rng.pick(LOCALS);
                if self.rng.chance(0.5) {
                    "(suspend $y0)".to_string()
                } else {
                    format!("(local.tee {l} (suspend $y {}))", self.expr(d))
                }
            }
            4 | 5 => format!("(call $h{} {})", self.rng.pick(&hs), self.expr(d)),
            6 => format!("(call_ref $f {} (ref.func $h{}))", self.expr(d), self.rng.pick(&hs)),
            _ => format!("(call_indirect $tab (type $f) {} (i32.const {}))", self.expr(d), self.rng.pick(&hs)),
        }
    }

    /// i32 expression; side effects only through suspending() (and traps).
    fn expr(&mut self, depth: u32) -> String {
        if depth > 0 && self.rng.chance(0.12) {
            return self.suspending(depth);
        }
        let leaf = depth == 0 || self.rng.chance(0.3);
        if leaf {
            return match self.rng.below(4) {
                0 => self.konst(),
                1 => "(global.get $g)".into(),
                _ => format!("(local.get {})", self.rng.pick(LOCALS)),
            };
        }
        match self.rng.below(9) {
            0..=3 => {
                let op = *self.rng.pick(&[
                    "add", "sub", "mul", "and", "or", "xor", "shl", "shr_u", "shr_s", "rotl", "eq", "ne",
                    "lt_u", "gt_s", "le_s", "ge_u",
                ]);
                format!("(i32.{op} {} {})", self.expr(depth - 1), self.expr(depth - 1))
            }
            4 => {
                let op = *self.rng.pick(&["eqz", "clz", "ctz", "popcnt", "extend8_s"]);
                format!("(i32.{op} {})", self.expr(depth - 1))
            }
            5 => format!("(i32.load (i32.and {} (i32.const 252)))", self.expr(depth - 1)),
            6 => format!("(select {} {} {})", self.expr(depth - 1), self.expr(depth - 1), self.expr(depth - 1)),
            7 if self.rng.chance(0.3) => {
                // Mostly a divisor that cannot be zero; traps end the export.
                let op = *self.rng.pick(&["div_s", "div_u", "rem_s", "rem_u"]);
                let d = self.expr(depth - 1);
                let d = if self.rng.chance(0.85) { format!("(i32.or {d} (i32.const 1))") } else { d };
                format!("(i32.{op} {} {d})", self.expr(depth - 1))
            }
            _ => format!("(local.get {})", self.rng.pick(LOCALS)),
        }
    }

    /// Statements of a coroutine-context body. `ctx` names the enclosing
    /// function, so that only later functions are called (no recursion).
    fn stmts(&mut self, out: &mut String, depth: u32, ctx: Ctx, n: usize) {
        for _ in 0..n {
            self.stmt(out, depth, ctx);
        }
    }

    fn stmt(&mut self, out: &mut String, depth: u32, ctx: Ctx) {
        let l = *self.rng.pick(LOCALS);
        let choice = self.rng.below(if depth == 0 { 9 } else { 13 });
        match choice {
            0 => writeln!(out, "(local.set {l} {})", self.expr(2)).unwrap(),
            1 => writeln!(out, "(global.set $g {})", self.expr(2)).unwrap(),
            2 => writeln!(out, "(i32.store (i32.and {} (i32.const 252)) {})", self.expr(1), self.expr(2)).unwrap(),
            3 => writeln!(out, "(call $log {})", self.expr(2)).unwrap(),
            4 => writeln!(out, "(local.set {l} (suspend $y {}))", self.expr(2)).unwrap(),
            5 => writeln!(out, "(local.set {l} (suspend $y2 {} {}))", self.expr(1), self.expr(1)).unwrap(),
            6 => {
                if self.rng.chance(0.3) {
                    writeln!(out, "(local.set {l} (suspend $y0))").unwrap()
                } else if self.rng.chance(0.15) {
                    writeln!(out, "(local.set {l} (suspend $stop {}))", self.expr(1)).unwrap()
                } else {
                    writeln!(out, "(local.set {l} (suspend $y {}))", self.expr(1)).unwrap()
                }
            }
            7 => {
                // Call a later helper, nested scheduler or a coroutine body
                // directly (its suspends then belong to our handler).
                let mut cands = Vec::new();
                for i in 0..self.n_help {
                    if ctx.kind != Kind::Help || i > ctx.index {
                        cands.push(format!("$h{i}"));
                    }
                }
                // Nested schedulers resume the last coroutine, so only
                // earlier coroutines may call them.
                for i in 0..self.n_nest {
                    if ctx.kind == Kind::Co && ctx.index + 1 < self.n_co {
                        cands.push(format!("$nest{i}"));
                    }
                }
                for i in 0..self.n_co {
                    if ctx.kind == Kind::Co && i > ctx.index {
                        cands.push(format!("$co{i}"));
                    }
                }
                if cands.is_empty() {
                    writeln!(out, "(call $log {})", self.expr(1)).unwrap();
                } else {
                    let f = self.rng.pick(&cands).clone();
                    writeln!(out, "(local.set {l} (call {f} {}))", self.expr(1)).unwrap();
                }
            }
            8 => {
                if ctx.in_try && self.rng.chance(0.7) || self.rng.chance(0.03) {
                    writeln!(out, "(if (i32.eqz (i32.and (global.get $g) (i32.const 3))) (then (throw $e {})))", self.expr(1))
                        .unwrap();
                } else {
                    writeln!(out, "(global.set $g (i32.add (global.get $g) (i32.const 1)))").unwrap();
                }
            }
            9 => {
                writeln!(out, "(if {}", self.expr(2)).unwrap();
                out.push_str("(then\n");
                let n = 1 + self.rng.below(3);
                self.stmts(out, depth - 1, ctx, n);
                out.push_str(")\n(else\n");
                let n = self.rng.below(3);
                self.stmts(out, depth - 1, ctx, n);
                out.push_str("))\n");
            }
            10 => {
                // Bounded loop with its own counter local.
                let c = format!("$c{depth}");
                let lp = self.lbl("loop");
                let bound = 1 + self.rng.below(4);
                writeln!(out, "(local.set {c} (i32.const 0))\n(loop {lp}").unwrap();
                let n = 1 + self.rng.below(4);
                self.stmts(out, depth - 1, ctx, n);
                writeln!(out, "(local.set {c} (i32.add (local.get {c}) (i32.const 1)))").unwrap();
                writeln!(out, "(br_if {lp} (i32.lt_u (local.get {c}) (i32.const {bound}))))").unwrap();
            }
            11 => {
                // Catch $e (also thrown into us by resume_throw at a suspend).
                let b = self.lbl("catch");
                writeln!(out, "(local.set {l} (block {b} (result i32)\n(try_table (catch $e {b})").unwrap();
                let n = 1 + self.rng.below(4);
                self.stmts(out, depth - 1, Ctx { in_try: true, ..ctx }, n);
                writeln!(out, ")\n{}))", self.konst()).unwrap();
            }
            12 if self.rng.chance(0.4) => {
                // br_table on a value that may come from a suspend.
                let (b1, b2) = (self.lbl("bt"), self.lbl("bt"));
                writeln!(out, "(block {b1}\n(block {b2}\n(br_table {b2} {b1} {b2} {})\n)", self.expr(2)).unwrap();
                let n = 1 + self.rng.below(2);
                self.stmts(out, depth - 1, ctx, n);
                out.push_str(")\n");
            }
            _ => {
                // Early exit from a block.
                let b = self.lbl("blk");
                writeln!(out, "(block {b}").unwrap();
                let n = 1 + self.rng.below(2);
                self.stmts(out, depth - 1, ctx, n);
                writeln!(out, "(br_if {b} {})", self.expr(1)).unwrap();
                let n = 1 + self.rng.below(2);
                self.stmts(out, depth - 1, ctx, n);
                out.push_str(")\n");
            }
        }
    }

    fn locals_decl() -> &'static str {
        "(local $t i32) (local $u i32) (local $c0 i32) (local $c1 i32) (local $c2 i32) (local $c3 i32)"
    }

    /// Coroutine body / helper / nested scheduler: (param $x i32) (result i32).
    fn func_body(&mut self, ctx: Ctx) -> String {
        self.cur = ctx;
        let mut b = String::new();
        let n = 2 + self.rng.below(5);
        self.stmts(&mut b, 3, ctx, n);
        let hs = self.helpers();
        if !hs.is_empty() && self.rng.chance(0.15) {
            // A tail call: the suspended frame is the callee's alone.
            write!(b, "(return_call $h{} {})", self.rng.pick(&hs), self.expr(2)).unwrap();
        } else {
            write!(b, "{}", self.expr(2)).unwrap();
        }
        b
    }

    /// A sub-scheduler, called from coroutine context: resumes a later
    /// coroutine handling only some tags; the rest go to the outer handler,
    /// which then captures the frames of both.
    fn nest(&mut self, i: usize) -> String {
        let co = self.n_co - 1;
        let handle_y2 = self.rng.chance(0.4);
        let mut s = String::new();
        writeln!(s, "(func $nest{i} (param $x i32) (result i32)\n(local $k (ref null $c)) (local $n i32) (local $t i32) (local $u i32)").unwrap();
        writeln!(s, "(local.set $k (cont.new $c (ref.func $co{co})))").unwrap();
        writeln!(s, "(block $done\n(loop $l").unwrap();
        writeln!(s, "(local.set $n (i32.add (local.get $n) (i32.const 1)))").unwrap();
        writeln!(s, "(br_if $done (i32.gt_u (local.get $n) (i32.const {})))", 3 + self.rng.below(10)).unwrap();
        if handle_y2 {
            s.push_str("(block $h2 (result i32 i32 (ref $c))\n");
        }
        s.push_str("(block $h (result i32 (ref $c))\n");
        let ons = if handle_y2 { "(on $y $h) (on $y2 $h2)" } else { "(on $y $h)" };
        writeln!(s, "(local.set $x (resume $c {ons} (local.get $x) (local.get $k)))").unwrap();
        s.push_str("(br $done))\n");
        writeln!(s, "(local.set $k)\n(local.set $x (i32.add (i32.const {})))", self.rng.below(50)).unwrap();
        if self.rng.chance(0.5) {
            s.push_str("(call $log (local.get $x))\n");
        }
        if self.rng.chance(0.3) {
            // Forward to the outer scheduler as well.
            s.push_str("(local.set $x (suspend $y (local.get $x)))\n");
        }
        s.push_str("(br $l)");
        if handle_y2 {
            s.push_str(")\n(local.set $k)\n(local.set $t)\n(local.set $x (i32.xor (local.get $t)))\n(br $l)");
        }
        s.push_str("))\n(local.get $x))\n");
        s
    }

    /// An export that drives one or two coroutines to completion.
    fn driver(&mut self, j: usize) -> String {
        let a = self.rng.below(self.n_co);
        let two = self.n_co > 1 && self.rng.chance(0.4);
        let bind = self.rng.chance(0.3);
        let catch_outer = self.rng.chance(0.3);
        let reresume = self.rng.chance(0.15);
        let bound = 5 + self.rng.below(40);
        let mut s = String::new();
        writeln!(s, "(func $run{j} (export \"run{j}\") (result i32)").unwrap();
        s.push_str("(local $k (ref null $c)) (local $k2 (ref null $c)) (local $kt (ref null $c)) (local $kold (ref null $c))\n");
        s.push_str("(local $v i32) (local $n i32) (local $mode i32) (local $m i32) (local $p i32)\n");
        writeln!(s, "(local.set $v {})", self.konst()).unwrap();
        let k0 = if bind {
            // First run through a cont.bind'ed (cont $f0); its suspensions
            // hand back plain (cont $f) continuations.
            s.push_str("(local.set $mode (i32.const 2))\n");
            format!("(cont.bind $c $c0 {} (cont.new $c (ref.func $co{a})))", self.konst())
        } else {
            writeln!(s, "(local.set $k (cont.new $c (ref.func $co{a})))").unwrap();
            "(ref.null $c0)".to_string()
        };
        if two {
            let mut b = self.rng.below(self.n_co);
            if b == a {
                b = (a + 1) % self.n_co;
            }
            writeln!(s, "(local.set $k2 (cont.new $c (ref.func $co{b})))").unwrap();
        }
        if catch_outer {
            s.push_str("(local.set $v (block $caught (result i32)\n(try_table (catch $e $caught)\n");
        }
        s.push_str("(block $done\n(loop $l\n");
        s.push_str("(local.set $n (i32.add (local.get $n) (i32.const 1)))\n");
        writeln!(s, "(br_if $done (i32.gt_u (local.get $n) (i32.const {bound})))").unwrap();
        s.push_str("(local.set $kold (local.get $k))\n(local.set $m (local.get $mode))\n(local.set $mode (i32.const 0))\n");
        s.push_str("(block $hstop (result i32 (ref $c))\n(block $h0 (result (ref $c))\n(block $h2 (result i32 i32 (ref $c))\n(block $h (result i32 (ref $c))\n");
        let ons = "(on $y $h) (on $y2 $h2) (on $y0 $h0) (on $stop $hstop)";
        writeln!(
            s,
            "(local.set $v (if (result i32) (i32.eq (local.get $m) (i32.const 1))\n(then (resume_throw $c $e {ons} (local.get $p) (local.get $k)))\n(else (if (result i32) (i32.eq (local.get $m) (i32.const 2))\n(then (resume $c0 {ons} {k0}))\n(else (resume $c {ons} (local.get $v) (local.get $k)))))))"
        )
        .unwrap();
        s.push_str("(call $log (local.get $v))\n");
        if two {
            s.push_str("(br_if $done (ref.is_null (local.get $k2)))\n(local.set $k (local.get $k2))\n(local.set $k2 (ref.null $c))\n(br $l))\n");
        } else {
            s.push_str("(br $done))\n");
        }
        // $h: [i32 k]
        writeln!(s, "(local.set $k)\n(local.set $v (i32.{} (i32.const {})))", self.rng.pick(&["add", "xor", "mul", "sub"]), self.rng.below(100)).unwrap();
        if self.rng.chance(0.6) {
            s.push_str("(call $log (local.get $v))\n");
        }
        if self.rng.chance(0.6) {
            // State the suspended code may have read before suspending.
            writeln!(s, "(global.set $g (i32.add (global.get $g) (i32.const {})))", 1 + self.rng.below(9)).unwrap();
        }
        if two {
            s.push_str("(if (i32.eqz (ref.is_null (local.get $k2))) (then (local.set $kt (local.get $k)) (local.set $k (local.get $k2)) (local.set $k2 (local.get $kt))))\n");
        }
        if reresume {
            writeln!(s, "(if (i32.eq (local.get $n) (i32.const {})) (then (local.set $k (local.get $kold))))", 1 + self.rng.below(4)).unwrap();
        }
        s.push_str("(br $l))\n");
        // $h2: [i32 i32 k]
        s.push_str("(local.set $k)\n(local.set $p)\n(local.set $v)\n");
        writeln!(s, "(local.set $mode (i32.and (local.get $p) (i32.const {})))", self.rng.pick(&[0, 1, 1, 3])).unwrap();
        s.push_str("(call $log (i32.add (local.get $v) (local.get $p)))\n");
        if self.rng.chance(0.5) {
            s.push_str("(i32.store (i32.const 0) (i32.add (i32.load (i32.const 0)) (local.get $v)))\n");
        }
        s.push_str("(br $l))\n");
        // $h0: [k]
        writeln!(s, "(local.set $k)\n(local.set $v (i32.add (local.get $n) (i32.const {})))\n(br $l))", self.rng.below(10)).unwrap();
        // $hstop: [i32 k] -- abandon this coroutine
        s.push_str("(drop)\n(local.set $v)\n(call $log (local.get $v))\n");
        if two {
            s.push_str("(local.set $k (local.get $k2))\n(local.set $k2 (ref.null $c))\n(br_if $l (i32.eqz (ref.is_null (local.get $k))))\n");
        }
        s.push_str("))\n");
        if catch_outer {
            s.push_str(")\n(local.get $v)))\n(call $log (local.get $v))\n");
        }
        s.push_str("(call $log (global.get $g))\n(call $log (i32.load (i32.const 0)))\n");
        s.push_str("(local.get $v))\n");
        s
    }
}

impl<'a> Gen<'a> {
    /// Symmetric coroutines with `switch`: each side does some work, then
    /// switches to the other, passing a value and receiving the other's
    /// continuation back. The first to return ends the export's resume.
    fn switch_pair(&mut self, out: &mut String, j: usize) {
        for side in ["a", "b"] {
            let rounds = 1 + self.rng.below(5);
            writeln!(out, "(func $sw{j}{side} (type $fs) (param $x i32) (param $k (ref null $cs)) (result i32)\n(local $t i32) (local $u i32) (local $n i32)").unwrap();
            writeln!(out, "(loop $l\n(local.set $n (i32.add (local.get $n) (i32.const 1)))").unwrap();
            self.cur = Ctx { kind: Kind::Co, index: usize::MAX, in_try: false };
            // Work between switches (no suspends to other tags: the only
            // handler is the switch one).
            for _ in 0..1 + self.rng.below(3) {
                let l = *self.rng.pick(&["$x", "$t", "$u"]);
                match self.rng.below(4) {
                    0 => writeln!(out, "(local.set {l} {})", self.pure(2)).unwrap(),
                    1 => writeln!(out, "(global.set $g {})", self.pure(2)).unwrap(),
                    2 => writeln!(out, "(i32.store (i32.and {} (i32.const 252)) {})", self.pure(1), self.pure(1)).unwrap(),
                    _ => writeln!(out, "(call $log {})", self.pure(2)).unwrap(),
                }
            }
            let v = self.pure(1);
            writeln!(out, "(switch $cs $sw (i32.add {v} (global.get $g)) (local.get $k))\n(local.set $k)\n(local.set $x)").unwrap();
            writeln!(out, "(call $log (i32.add (local.get $x) (i32.const {})))", if side == "a" { 1000 } else { 2000 }).unwrap();
            writeln!(out, "(br_if $l (i32.lt_u (local.get $n) (i32.const {rounds}))))\n(i32.add (local.get $x) (local.get $t)))").unwrap();
            writeln!(out, "(elem declare func $sw{j}{side})").unwrap();
        }
        writeln!(out, "(func $runsw{j} (export \"runsw{j}\") (result i32)\n(local $v i32)").unwrap();
        writeln!(out, "(local.set $v (resume $cs (on $sw switch) {} (cont.new $cs (ref.func $sw{j}b)) (cont.new $cs (ref.func $sw{j}a))))", self.konst()).unwrap();
        out.push_str("(call $log (local.get $v))\n(call $log (global.get $g))\n(local.get $v))\n");
    }

    /// An expression that never suspends or calls.
    fn pure(&mut self, depth: u32) -> String {
        loop {
            let e = self.expr(depth);
            if !e.contains("suspend") && !e.contains("call") {
                return e;
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Co,
    Help,
}

#[derive(Clone, Copy)]
struct Ctx {
    kind: Kind,
    index: usize,
    in_try: bool,
}

/// One random stack-switching module in the text format.
pub fn module(rng: &mut Rng) -> String {
    let n_co = 1 + rng.below(4);
    let n_help = rng.below(3);
    let n_nest = rng.below(3);
    let n_drv = 1 + rng.below(4);
    let mut g = Gen { rng, label: 0, n_co, n_help, n_nest, cur: Ctx { kind: Kind::Co, index: 0, in_try: false } };
    let mut m = String::from(
        "(module\n\
         (import \"fuzzing-support\" \"log-i32\" (func $log (param i32)))\n\
         (type $f (func (param i32) (result i32)))\n\
         (type $c (cont $f))\n\
         (type $f0 (func (result i32)))\n\
         (type $c0 (cont $f0))\n\
         (tag $y (param i32) (result i32))\n\
         (tag $y2 (param i32 i32) (result i32))\n\
         (tag $y0 (result i32))\n\
         (tag $stop (param i32) (result i32))\n\
         (tag $e (param i32))\n\
         (rec (type $fs (func (param i32 (ref null $cs)) (result i32))) (type $cs (cont $fs)))\n\
         (tag $sw (result i32))\n\
         (global $g (mut i32) (i32.const 0))\n\
         (memory 1)\n",
    );
    for i in 0..n_co {
        let body = g.func_body(Ctx { kind: Kind::Co, index: i, in_try: false });
        writeln!(m, "(func $co{i} (type $f) (param $x i32) (result i32) {}\n{body})", Gen::locals_decl()).unwrap();
        writeln!(m, "(elem declare func $co{i})").unwrap();
    }
    if n_help > 0 {
        let names: Vec<String> = (0..n_help).map(|i| format!("$h{i}")).collect();
        writeln!(m, "(table $tab funcref (elem {}))", names.join(" ")).unwrap();
    }
    for i in 0..n_help {
        let body = g.func_body(Ctx { kind: Kind::Help, index: i, in_try: false });
        writeln!(m, "(func $h{i} (param $x i32) (result i32) {}\n{body})", Gen::locals_decl()).unwrap();
    }
    for i in 0..n_nest {
        let s = g.nest(i);
        m.push_str(&s);
    }
    for j in 0..n_drv {
        let s = g.driver(j);
        m.push_str(&s);
    }
    // binaryen's interpreter cannot run `switch` yet (ModuleRunner
    // visitStackSwitch), so such modules only get the wasmtime oracles.
    if g.rng.chance(0.25) {
        let mut s = String::new();
        g.switch_pair(&mut s, 0);
        m.push_str(&s);
    }
    m.push_str(")\n");
    m
}
