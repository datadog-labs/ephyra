// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! Generator of deterministic, well-typed OCaml programs (modules H, A, Main).
//!
//! Soundness rules (manual/src/refman/expr.etex: evaluation order of
//! application arguments, tuple/record/constructor fields, `let ... and` is
//! unspecified): every expression that has a side effect or may raise is
//! let-bound in a statement sequence; sub-expressions (arguments, fields,
//! operands) are always *pure*: no effects, no exceptions, terminating.
//! Shift counts are masked (stdlib.mli: unspecified if m < 0 or m > int_size),
//! float->int conversions are clamped (unspecified out of range), nan is printed
//! as "nan" (sign/payload unspecified), `==` is never used on immutable values.

use std::fmt::Write;

/// Every decision is drawn from the ephyra `Source` of the current test, so a
/// program is a pure function of the source (ephyra's determinism contract).
pub struct Rng<'a>(&'a mut ephyra::Source);
impl<'a> Rng<'a> {
    pub fn next(&mut self) -> u64 {
        self.0.int(..)
    }
    pub fn below(&mut self, n: usize) -> usize {
        self.0.index(n)
    }
    pub fn range(&mut self, lo: usize, hi: usize) -> usize {
        self.0.int(lo..=hi)
    }
    pub fn chance(&mut self, p: f64) -> bool {
        self.0.bool_with(p)
    }
    pub fn pick<'b, T>(&mut self, xs: &'b [T]) -> &'b T {
        self.0.pick(xs)
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Ty {
    Int,
    Bool,
    Float,
    I32,
    I64,
    Nat,
    Str,
    Opt(Box<Ty>),
    Pair(Box<Ty>, Box<Ty>),
    List(Box<Ty>),
    Arr(Box<Ty>),
    Ref(Box<Ty>),
    R,
    FR,
    V,
    PV,
    Atom(Box<Ty>),
    /// H.ar: record with [@atomic] fields ai : int, ab : bool and a plain ap : int
    AR,
    /// int Atomic.Array.t (always length 4)
    AA,
    Fun(Box<Ty>, Box<Ty>),
}
use Ty::*;

fn bx(t: Ty) -> Box<Ty> {
    Box::new(t)
}

impl Ty {
    pub fn ml(&self) -> String {
        match self {
            Int => "int".into(),
            Bool => "bool".into(),
            Float => "float".into(),
            I32 => "int32".into(),
            I64 => "int64".into(),
            Nat => "nativeint".into(),
            Str => "string".into(),
            Opt(t) => format!("({}) option", t.ml()),
            Pair(a, b) => format!("({} * {})", a.ml(), b.ml()),
            List(t) => format!("({}) list", t.ml()),
            Arr(t) => format!("({}) array", t.ml()),
            Ref(t) => format!("({}) ref", t.ml()),
            R => "H.r".into(),
            FR => "H.fr".into(),
            V => "H.v".into(),
            PV => "H.pv".into(),
            Atom(t) => format!("({}) Atomic.t", t.ml()),
            AR => "H.ar".into(),
            AA => "(int Atomic.Array.t)".into(),
            Fun(a, b) => format!("({} -> {})", a.ml(), b.ml()),
        }
    }
    fn printer(&self) -> String {
        match self {
            Int => "H.p_int".into(),
            Bool => "H.p_bool".into(),
            Float => "H.p_float".into(),
            I32 => "H.p_i32".into(),
            I64 => "H.p_i64".into(),
            Nat => "H.p_nat".into(),
            Str => "H.p_str".into(),
            Opt(t) => format!("(H.p_opt {})", t.printer()),
            Pair(a, b) => format!("(H.p_pair {} {})", a.printer(), b.printer()),
            List(t) => format!("(H.p_list {})", t.printer()),
            Arr(t) => format!("(H.p_arr {})", t.printer()),
            Ref(t) => format!("(H.p_ref {})", t.printer()),
            R => "H.p_r".into(),
            FR => "H.p_fr".into(),
            V => "H.p_v".into(),
            PV => "H.p_pv".into(),
            Atom(t) => format!("(H.p_atom {})", t.printer()),
            AR => "H.p_ar".into(),
            AA => "H.p_aa".into(),
            Fun(..) => "(fun _ -> print_string \"<fun> \")".into(),
        }
    }
    /// Polymorphic compare is defined (no functional values inside).
    fn comparable(&self) -> bool {
        match self {
            Fun(..) | Atom(_) | AR | AA => false,
            Opt(t) | List(t) | Arr(t) | Ref(t) => t.comparable(),
            Pair(a, b) => a.comparable() && b.comparable(),
            _ => true,
        }
    }
    fn has_fun(&self) -> bool {
        !self.comparable()
    }
}

#[derive(Clone)]
struct Var {
    name: String,
    ty: Ty,
}
#[derive(Clone)]
struct Func {
    name: String,
    params: Vec<Ty>,
    ret: Ty,
}
#[derive(Clone, Default)]
struct Env {
    vars: Vec<Var>,
    funcs: Vec<Func>,
}
impl Env {
    fn with_var(&self, name: &str, ty: Ty) -> Env {
        let mut e = self.clone();
        e.vars.push(Var { name: name.into(), ty });
        e
    }
    fn vars_of(&self, ty: &Ty) -> Vec<&Var> {
        self.vars.iter().filter(|v| &v.ty == ty).collect()
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum RK {
    Fun,
    Knot,
    Kv,
    Lst,
    FRec,
    Lz,
    FunArr,
    FltArr,
    IntArr,
    Konst,
}

/// A generated program: module A's source and Main's test functions
/// (`(N, "let tN () =\n...()\n")`, run in order). Shrinking deletes tests,
/// top-level items of A and lines of tests, so ids need not be contiguous.
#[derive(Clone, PartialEq, Eq)]
pub struct Program {
    pub a: String,
    pub tests: Vec<(usize, String)>,
}

impl Program {
    pub fn main_src(&self) -> String {
        let mut s = String::new();
        for (_, t) in &self.tests {
            s.push_str(t);
            s.push('\n');
        }
        for (i, _) in &self.tests {
            let _ = writeln!(s, "let () = H.run \"t{i}\" t{i}");
        }
        s.push_str("let () = print_newline ()\n");
        s
    }
}

pub struct Gen<'a> {
    r: Rng<'a>,
    n: usize,
    /// remaining "size" budget for the current top-level function
    fuel: i64,
    /// > 0 while generating the body of an effect handler
    handler: u32,
}

/// Strings shared by literals and string patterns: common prefixes, lengths
/// around the 8-byte word boundary, embedded NULs (asmcomp/strmatch.ml
/// compares strings a word at a time).
const STR_POOL: &[&str] = &[
    "\"\"", "\"a\"", "\"b\"", "\"ab\"", "\"abc\"", "\"hello\"", "\"abcdefg\"", "\"abcdefgh\"", "\"abcdefgi\"",
    "\"abcdefghi\"", "\"abcdefghijklmnop\"", "\"abcdefghijklmnoq\"", "\"abcdefghijklmnopq\"", "\"\\000z\"",
    "\"\\000\\000\\000\\000\\000\\000\\000\\000\"", "\"q\"", "\"qq\"", "\"qqqqqqq\"", "\"qqqqqqqq\"", "\"yes\"", "\"no\"", "\"0\"", "\"-1\"", "\"42\"",
    "\"xyzzy-42\"", "\"hellohel\"", "\"\\255\\255\\255\\255\\255\\255\\255\"",
];

const SCALAR: &[Ty] = &[Int, Int, Int, Bool, Float, Float, I32, I64, I64, Nat, Str];

impl<'a> Gen<'a> {
    pub fn new(src: &'a mut ephyra::Source) -> Gen<'a> {
        Gen { r: Rng(src), n: 0, fuel: 0, handler: 0 }
    }
    fn fresh(&mut self, p: &str) -> String {
        self.n += 1;
        format!("{p}{}", self.n)
    }

    fn scalar(&mut self) -> Ty {
        self.r.pick(SCALAR).clone()
    }
    fn ty(&mut self, d: u32) -> Ty {
        if d == 0 || self.r.chance(0.55) {
            return match self.r.below(15) {
                14 => PV,
                0 => R,
                1 => FR,
                2 | 3 => V,
                _ => self.scalar(),
            };
        }
        match self.r.below(7) {
            0 => Opt(bx(self.ty(d - 1))),
            1 => Pair(bx(self.ty(d - 1)), bx(self.ty(d - 1))),
            2 => List(bx(self.ty(d - 1))),
            3 => Arr(bx(self.ty(d - 1))),
            4 => Ref(bx(self.ty(d - 1))),
            5 => self.fun_ty(),
            _ => Arr(bx(self.r.pick(&[Float, Int, I64]).clone())),
        }
    }
    fn fun_ty(&mut self) -> Ty {
        match self.r.below(4) {
            0 => Fun(bx(Int), bx(Int)),
            1 => Fun(bx(Float), bx(Float)),
            2 => Fun(bx(Int), bx(Float)),
            _ => Fun(bx(I64), bx(I64)),
        }
    }
    fn data_ty(&mut self, d: u32) -> Ty {
        loop {
            let t = self.ty(d);
            if t.comparable() {
                return t;
            }
        }
    }

    // ---------------------------------------------------------------- literals
    fn int_lit(&mut self) -> String {
        let v: i64 = match self.r.below(12) {
            0 => return "max_int".into(),
            1 => return "min_int".into(),
            2 => (1i64 << self.r.below(62)) - self.r.below(2) as i64,
            3 => -(1i64 << self.r.below(62)),
            4 | 5 => *self.r.pick(&[0, 1, -1, 2, 3, 7, 8, 63, 64, 255, 256, -2]),
            6 => self.r.next() as i64 >> 2,
            _ => self.r.below(41) as i64 - 20,
        };
        if v < 0 {
            format!("({v})")
        } else {
            format!("{v}")
        }
    }
    fn bint_lit(&mut self, t: &Ty) -> String {
        let (suf, m) = match t {
            I32 => ("l", "Int32"),
            I64 => ("L", "Int64"),
            _ => ("n", "Nativeint"),
        };
        let bits = if *t == I32 { 31 } else { 63 };
        let v: i64 = match self.r.below(10) {
            0 => return format!("{m}.max_int"),
            1 => return format!("{m}.min_int"),
            2 => (1i64 << self.r.below(bits)) - self.r.below(2) as i64,
            3 => *self.r.pick(&[0, 1, -1, 2, 7, 255, -256]),
            4 => {
                if bits == 31 {
                    (self.r.next() as i32) as i64
                } else {
                    self.r.next() as i64 >> 1
                }
            }
            _ => self.r.below(41) as i64 - 20,
        };
        if v < 0 {
            format!("({v}{suf})")
        } else {
            format!("{v}{suf}")
        }
    }
    fn float_lit(&mut self) -> String {
        let v: f64 = match self.r.below(14) {
            0 => return "Float.nan".into(),
            1 => return "Float.infinity".into(),
            2 => return "Float.neg_infinity".into(),
            3 => return "(-0.)".into(),
            4 => *self.r.pick(&[0.0, 1.0, -1.0, 0.5, 2.0, 1e308, 4.9e-324, 2.2250738585072014e-308, 9007199254740992.0, 3.5, -2.5]),
            5 => *self.r.pick(&[2147483647.0, -2147483648.0, 4294967296.5, 1e15, -1e15, 0.1]),
            6 => f64::from_bits(self.r.next()),
            _ => (self.r.below(2001) as f64 - 1000.0) / 8.0,
        };
        if v.is_nan() {
            return "Float.nan".into();
        }
        if v.is_infinite() {
            return if v > 0.0 { "Float.infinity".into() } else { "Float.neg_infinity".into() };
        }
        let s = format!("{v:e}");
        if v < 0.0 || s.starts_with('-') {
            format!("({s})")
        } else {
            s
        }
    }
    fn str_lit(&mut self) -> String {
        if self.r.chance(0.6) {
            self.r.pick(&["\"\"", "\"a\"", "\"ab\"", "\"hello\"", "\"\\000z\"", "\"xyzzy-42\""]).to_string()
        } else {
            self.r.pick(STR_POOL).to_string()
        }
    }

    // ---------------------------------------------------------------- pure expressions
    /// A pure (effect-free, non-raising, terminating) expression of type `t`.
    fn pure(&mut self, t: &Ty, env: &Env, d: u32) -> String {
        self.fuel -= 1;
        let d = if self.fuel < 0 { 0 } else { d };
        let vars = env.vars_of(t);
        if !vars.is_empty() && self.r.chance(if d == 0 { 0.7 } else { 0.3 }) {
            let n = self.r.pick(&vars).name.clone();
            return n;
        }
        if d > 0 && self.r.chance(0.06) {
            let e = self.pure(t, env, d - 1);
            return format!("(Sys.opaque_identity {e})");
        }
        if d > 0 && self.r.chance(0.07) {
            let c = self.pure(&Bool, env, d - 1);
            let a = self.pure(t, env, d - 1);
            let b = self.pure(t, env, d - 1);
            return format!("(if {c} then {a} else {b})");
        }
        if d > 0 && self.r.chance(0.05) {
            return self.pure_match(t, env, d);
        }
        if d > 0 && self.r.chance(0.05) {
            // pure let. Known bug (agents/harness/sizeexpr, also in 5.4.0): an unused
            // `let r = ref _ in e` directly inside an allocated block crashes the
            // closure backend (Selection.size_expr); refs are kept out of pure lets
            // so it does not dominate the failures.
            let mut ty = self.data_ty(1);
            while matches!(ty, Ref(_)) {
                ty = self.data_ty(1);
            }
            let x = self.fresh("p");
            let e = self.pure(&ty, env, d - 1);
            let env2 = env.with_var(&x, ty);
            let b = self.pure(t, &env2, d - 1);
            return format!("(let {x} = {e} in {b})");
        }
        if d > 0 && self.r.chance(0.08) {
            if let Some(e) = self.pure_read(t, env, d) {
                return e;
            }
        }
        match t {
            Int => self.pure_int(env, d),
            Bool => self.pure_bool(env, d),
            Float => self.pure_float(env, d),
            I32 | I64 | Nat => self.pure_bint(t, env, d),
            Str => self.pure_str(env, d),
            Opt(u) => {
                if self.r.chance(0.25) {
                    "None".into()
                } else {
                    format!("(Some {})", self.pure(u, env, d.saturating_sub(1)))
                }
            }
            Pair(a, b) => {
                if d > 0 && self.r.chance(0.2) {
                    let p = self.pure(&Pair(a.clone(), bx(Int)), env, d - 1);
                    if self.r.chance(0.5) {
                        let q = self.pure(b, env, d - 1);
                        return format!("(fst {p}, {q})");
                    }
                }
                let x = self.pure(a, env, d.saturating_sub(1));
                let y = self.pure(b, env, d.saturating_sub(1));
                format!("({x}, {y})")
            }
            List(u) => {
                let d1 = d.saturating_sub(1);
                match if d == 0 { self.r.below(2) } else { self.r.below(9) } {
                    0 => "[]".into(),
                    1 => format!("[{}]", self.pure(u, env, d1)),
                    2 | 3 => format!("({} :: {})", self.pure(u, env, d1), self.pure(t, env, d1)),
                    4 => format!("(List.rev {})", self.pure(t, env, d1)),
                    5 => format!("({} @ {})", self.pure(t, env, d1), self.pure(t, env, d1)),
                    6 => {
                        let x = self.fresh("x");
                        let body = self.pure(u, &env.with_var(&x, (**u).clone()), d1);
                        format!("(List.map (fun {x} -> {body}) {})", self.pure(t, env, d1))
                    }
                    7 if u.comparable() => {
                        let x = self.fresh("x");
                        let body = self.pure(&Bool, &env.with_var(&x, (**u).clone()), d1);
                        format!("(List.filter (fun {x} -> {body}) {})", self.pure(t, env, d1))
                    }
                    _ => format!("[{}; {}; {}]", self.pure(u, env, d1), self.pure(u, env, d1), self.pure(u, env, d1)),
                }
            }
            Arr(u) => {
                // arrays are never empty (H.get relies on it)
                let d1 = d.saturating_sub(1);
                match if d == 0 { self.r.below(2) } else { self.r.below(7) } {
                    0 => format!("[| {} |]", self.pure(u, env, d1)),
                    1 => {
                        let n = self.r.range(1, 9);
                        let e = self.pure(u, env, d1);
                        format!("(Array.make {n} {e})")
                    }
                    2 => format!("(Array.copy {})", self.pure(t, env, d1)),
                    3 => format!("(Array.append {} {})", self.pure(t, env, d1), self.pure(t, env, d1)),
                    4 => {
                        let x = self.fresh("x");
                        let body = self.pure(u, &env.with_var(&x, (**u).clone()), d1);
                        format!("(Array.map (fun {x} -> {body}) {})", self.pure(t, env, d1))
                    }
                    5 => {
                        let n = self.r.range(1, 12);
                        let x = self.fresh("i");
                        let body = self.pure(u, &env.with_var(&x, Int), d1);
                        format!("(Array.init {n} (fun {x} -> {body}))")
                    }
                    _ => format!("[| {}; {}; {} |]", self.pure(u, env, d1), self.pure(u, env, d1), self.pure(u, env, d1)),
                }
            }
            Ref(u) => format!("(ref {})", self.pure(u, env, d.saturating_sub(1))),
            R => {
                let d1 = d.saturating_sub(1);
                if d > 0 && self.r.chance(0.3) {
                    let b = self.pure(&R, env, d1);
                    let i = self.pure(&Int, env, d1);
                    return format!("{{ {b} with H.ri = {i} }}");
                }
                let (i, f, l) = (self.pure(&Int, env, d1), self.pure(&Float, env, d1), self.pure(&I64, env, d1));
                format!("{{ H.ri = {i}; rf = {f}; rl = {l} }}")
            }
            FR => {
                let d1 = d.saturating_sub(1);
                let (x, y) = (self.pure(&Float, env, d1), self.pure(&Float, env, d1));
                format!("{{ H.fx = {x}; fy = {y} }}")
            }
            V => {
                let d1 = d.saturating_sub(1);
                match self.r.below(6) {
                    0 => "H.V0".into(),
                    1 => "H.V1".into(),
                    2 => format!("(H.V2 {})", self.pure(&Int, env, d1)),
                    3 => format!("(H.V3 ({}, {}))", self.pure(&Float, env, d1), self.pure(&Int, env, d1)),
                    4 => format!("(H.V4 {})", self.pure(&I64, env, d1)),
                    _ => format!("(H.V5 {{ vx = {}; vy = {} }})", self.pure(&Int, env, d1), self.pure(&Float, env, d1)),
                }
            }
            PV => {
                let d1 = d.saturating_sub(1);
                match self.r.below(4) {
                    0 => "`A".into(),
                    1 => format!("(`B {})", self.pure(&Int, env, d1)),
                    2 => format!("(`C ({}, {}))", self.pure(&Float, env, d1), self.pure(&Int, env, d1)),
                    _ => format!("(`D {})", self.pure(&Str, env, d1)),
                }
            }
            Atom(u) => format!("(Atomic.make {})", self.pure(u, env, d.saturating_sub(1))),
            AR => {
                let d1 = d.saturating_sub(1);
                format!("{{ H.ai = {}; ab = {}; ap = {} }}", self.pure(&Int, env, d1), self.pure(&Bool, env, d1), self.pure(&Int, env, d1))
            }
            AA => format!("(Atomic.Array.make 4 {})", self.pure(&Int, env, d.saturating_sub(1))),
            Fun(a, b) => {
                let x = self.fresh("x");
                let body = self.pure(b, &env.with_var(&x, (**a).clone()), d.saturating_sub(1));
                format!("(fun {x} -> {body})")
            }
        }
    }

    /// Reads from mutable containers / projections that produce a `t`.
    fn pure_read(&mut self, t: &Ty, env: &Env, d: u32) -> Option<String> {
        let mut cands: Vec<String> = Vec::new();
        for v in &env.vars {
            match &v.ty {
                Ref(u) if **u == *t => cands.push(format!("!{}", v.name)),
                Arr(u) if **u == *t => {
                    let i = self.pure(&Int, env, d - 1);
                    cands.push(format!("(H.get {} {i})", v.name));
                }
                Pair(a, _) if **a == *t => cands.push(format!("(fst {})", v.name)),
                Pair(_, b) if **b == *t => cands.push(format!("(snd {})", v.name)),
                R if *t == Int => cands.push(format!("{}.H.ri", v.name)),
                R if *t == Float => cands.push(format!("{}.H.rf", v.name)),
                R if *t == I64 => cands.push(format!("{}.H.rl", v.name)),
                FR if *t == Float => cands.push(format!("{}.H.fx", v.name)),
                FR if *t == Float => cands.push(format!("{}.H.fy", v.name)),
                Atom(u) if **u == *t => cands.push(format!("(Atomic.get {})", v.name)),
                AR if *t == Int => {
                    cands.push(format!("(Atomic.Loc.get [%atomic.loc {}.H.ai])", v.name));
                    cands.push(format!("{}.H.ap", v.name));
                    cands.push(format!("{}.H.ai", v.name));
                }
                AR if *t == Bool => cands.push(format!("{}.H.ab", v.name)),
                AA if *t == Int => {
                    let i = self.pure_int(env, 0);
                    cands.push(format!("(Atomic.Array.get {} ({i} land 3))", v.name));
                }
                Opt(u) if **u == *t => {
                    let dflt = self.pure(t, env, d - 1);
                    cands.push(format!("(Option.value ~default:{dflt} {})", v.name))
                }
                _ => {}
            }
        }
        if cands.is_empty() {
            None
        } else {
            Some(self.r.pick(&cands).clone())
        }
    }

    fn pure_int(&mut self, env: &Env, d: u32) -> String {
        if d == 0 {
            return self.int_lit();
        }
        let d1 = d - 1;
        let p = |g: &mut Gen, t: &Ty| g.pure(t, env, d1);
        match self.r.below(30) {
            0..=2 => self.int_lit(),
            3 => format!("({} + {})", p(self, &Int), p(self, &Int)),
            4 => format!("({} - {})", p(self, &Int), p(self, &Int)),
            5 => format!("({} * {})", p(self, &Int), p(self, &Int)),
            6 => format!("({} / H.nz {})", p(self, &Int), p(self, &Int)),
            7 => format!("({} mod H.nz {})", p(self, &Int), p(self, &Int)),
            8 => {
                let k = *self.r.pick(&[1, 2, 3, 4, 7, 8, 10, 16, -1, -3, 1024i64, 1i64 << 32]);
                let op = self.r.pick(&["/", "mod", "*"]).to_string();
                let ks = if k < 0 { format!("({k})") } else { k.to_string() };
                format!("({} {op} {ks})", p(self, &Int))
            }
            9 => format!("({} land {})", p(self, &Int), p(self, &Int)),
            10 => format!("({} lor {})", p(self, &Int), p(self, &Int)),
            11 => format!("({} lxor {})", p(self, &Int), p(self, &Int)),
            12 => {
                let op = self.r.pick(&["lsl", "lsr", "asr"]).to_string();
                if self.r.chance(0.5) {
                    // boundary shift counts are where constant folding goes wrong
                    let mut k = if self.r.chance(0.5) { *self.r.pick(&[0, 1, 2, 31, 32, 61, 62, 63]) } else { self.r.below(64) };
                    // known bug (README): untagging `x asr 63` gives the tagged value; keep
                    // literal `asr 63` rare so it does not dominate.
                    if op == "asr" && k == 63 && self.r.chance(0.9) {
                        k = 62;
                    }
                    format!("({} {op} {k})", p(self, &Int))
                } else {
                    format!("({} {op} ({} land 63))", p(self, &Int), p(self, &Int))
                }
            }
            13 => format!("({} ({}))", self.r.pick(&["abs", "succ", "pred", "lnot", "(~-)"]), p(self, &Int)),
            14 => format!("({} {})", self.r.pick(&["Int.popcount", "Int.leading_zeros", "Int.trailing_zeros"]), p(self, &Int)),
            15 => format!("(H.f2i {})", p(self, &Float)),
            16 => {
                let t = self.r.pick(&[I32, I64, Nat]).clone();
                let m = bmod(&t);
                format!("({m}.to_int {})", p(self, &t))
            }
            17 => {
                let t = self.data_ty(1);
                let (a, b) = (p(self, &t), p(self, &t));
                format!("(compare {a} {b})")
            }
            18 => format!("(String.length {})", p(self, &Str)),
            19 => format!("(H.sget {} {})", p(self, &Str), p(self, &Int)),
            20 => {
                let t = self.data_ty(1);
                format!("(List.length {})", p(self, &List(bx(t))))
            }
            21 => {
                let t = self.data_ty(1);
                format!("(Array.length {})", p(self, &Arr(bx(t))))
            }
            22 => format!("(Bool.to_int {})", p(self, &Bool)),
            23 => format!("({} {} {})", self.r.pick(&["min", "max"]), p(self, &Int), p(self, &Int)),
            24 => {
                let t = self.r.pick(&[I32, I64, Nat]).clone();
                let m = bmod(&t);
                format!("({m}.unsigned_compare {} {})", p(self, &t), p(self, &t))
            }
            25 => {
                let x = self.fresh("x");
                let l = p(self, &List(bx(Int)));
                let init = p(self, &Int);
                let acc = self.fresh("acc");
                let env2 = env.with_var(&x, Int).with_var(&acc, Int);
                let body = self.pure(&Int, &env2, d1);
                format!("(List.fold_left (fun {acc} {x} -> {body}) {init} {l})")
            }
            26 => format!("(Int.shift_left {} ({} land 63))", p(self, &Int), p(self, &Int)),
            27 => format!("(Char.code (Char.unsafe_chr ({} land 255)))", p(self, &Int)),
            _ => format!("({} {} {})", p(self, &Int), self.r.pick(&["+", "-", "*", "land", "lxor"]), self.int_lit()),
        }
    }

    fn pure_bool(&mut self, env: &Env, d: u32) -> String {
        if d == 0 {
            return self.r.pick(&["true", "false"]).to_string();
        }
        let d1 = d - 1;
        match self.r.below(10) {
            0 => self.r.pick(&["true", "false"]).to_string(),
            1..=4 => {
                let t = if self.r.chance(0.6) { self.scalar() } else { self.data_ty(1) };
                let op = self.r.pick(&["=", "<>", "<", "<=", ">", ">="]).to_string();
                let (a, b) = (self.pure(&t, env, d1), self.pure(&t, env, d1));
                format!("({a} {op} {b})")
            }
            5 => format!("(not {})", self.pure(&Bool, env, d1)),
            6 => format!("({} && {})", self.pure(&Bool, env, d1), self.pure(&Bool, env, d1)),
            7 => format!("({} || {})", self.pure(&Bool, env, d1), self.pure(&Bool, env, d1)),
            8 => format!("({} {})", self.r.pick(&["Float.is_nan", "Float.is_integer", "Float.is_finite"]), self.pure(&Float, env, d1)),
            _ => {
                let t = self.r.pick(&[I32, I64, Nat, Float]).clone();
                let m = if t == Float { "Float" } else { bmod(&t) };
                format!("({m}.equal {} {})", self.pure(&t, env, d1), self.pure(&t, env, d1))
            }
        }
    }

    fn pure_float(&mut self, env: &Env, d: u32) -> String {
        if d == 0 {
            return self.float_lit();
        }
        let d1 = d - 1;
        let p = |g: &mut Gen, t: &Ty| g.pure(t, env, d1);
        match self.r.below(20) {
            0..=2 => self.float_lit(),
            3 => format!("({} +. {})", p(self, &Float), p(self, &Float)),
            4 => format!("({} -. {})", p(self, &Float), p(self, &Float)),
            5 => format!("({} *. {})", p(self, &Float), p(self, &Float)),
            6 => format!("({} /. {})", p(self, &Float), p(self, &Float)),
            7 => format!("(Float.of_int {})", p(self, &Int)),
            8 => {
                let t = self.r.pick(&[I32, I64, Nat]).clone();
                format!("({}.to_float {})", bmod(&t), p(self, &t))
            }
            9 => format!(
                "({} {})",
                self.r.pick(&["sqrt", "Float.abs", "(~-.)", "Float.round", "Float.trunc", "Float.floor", "Float.ceil", "Float.succ", "Float.pred"]),
                p(self, &Float)
            ),
            10 => format!("({} {} {})", self.r.pick(&["Float.min", "Float.max", "Float.rem", "Float.min_num", "Float.max_num"]), p(self, &Float), p(self, &Float)),
            11 => format!("(Float.fma {} {} {})", p(self, &Float), p(self, &Float), p(self, &Float)),
            12 => format!("(Int64.float_of_bits {})", p(self, &I64)),
            13 => format!("(Int32.float_of_bits {})", p(self, &I32)),
            14 => format!("(float_of_int {} *. {})", p(self, &Int), self.float_lit()),
            15 => {
                let x = self.fresh("x");
                let l = p(self, &Arr(bx(Float)));
                let init = p(self, &Float);
                let acc = self.fresh("acc");
                let env2 = env.with_var(&x, Float).with_var(&acc, Float);
                let body = self.pure(&Float, &env2, d1);
                format!("(Array.fold_left (fun {acc} {x} -> {body}) {init} {l})")
            }
            _ => format!("({} {} {})", p(self, &Float), self.r.pick(&["+.", "-.", "*.", "/."]), self.float_lit()),
        }
    }

    fn pure_bint(&mut self, t: &Ty, env: &Env, d: u32) -> String {
        if d == 0 {
            return self.bint_lit(t);
        }
        let d1 = d - 1;
        let m = bmod(t);
        let mask = if *t == I32 { 31 } else { 63 };
        let p = |g: &mut Gen, t: &Ty| g.pure(t, env, d1);
        match self.r.below(20) {
            0..=2 => self.bint_lit(t),
            3 => format!("({m}.{} {} {})", self.r.pick(&["add", "sub", "mul", "logand", "logor", "logxor"]), p(self, t), p(self, t)),
            4 => format!("({m}.{} {} {})", self.r.pick(&["add", "sub", "mul"]), p(self, t), p(self, t)),
            5 => {
                let op = self.r.pick(&["div", "rem", "unsigned_div", "unsigned_rem"]).to_string();
                let lit1 = self.bint_lit(&t.clone());
                format!("({m}.{op} {} ({m}.logor {} {}))", p(self, t), p(self, t), if self.r.chance(0.5) { format!("1{}", suffix(t)) } else { format!("({m}.logor {lit1} 1{})", suffix(t)) })
            }
            6 => {
                let op = self.r.pick(&["shift_left", "shift_right", "shift_right_logical"]).to_string();
                if self.r.chance(0.5) {
                    let k = if self.r.chance(0.5) { *self.r.pick(&[0, 1, mask - 1, mask, mask / 2, mask / 2 + 1]) } else { self.r.below(mask + 1) };
                    format!("({m}.{op} {} {k})", p(self, t))
                } else {
                    format!("({m}.{op} {} ({} land {mask}))", p(self, t), p(self, &Int))
                }
            }
            7 => format!("({m}.{} {})", self.r.pick(&["neg", "succ", "pred", "abs", "lognot"]), p(self, t)),
            8 => format!("({m}.of_int {})", p(self, &Int)),
            9 => {
                let f = match t {
                    I32 => "H.f2i32",
                    I64 => "H.f2i64",
                    _ => "H.f2nat",
                };
                format!("({f} {})", p(self, &Float))
            }
            10 => match t {
                I64 => format!("(H.bits64 {})", p(self, &Float)),
                I32 => format!("(H.bits32 {})", p(self, &Float)),
                _ => format!("(Nativeint.of_int32 {})", p(self, &I32)),
            },
            11 => match t {
                I64 => format!("(Int64.of_int32 {})", p(self, &I32)),
                I32 => format!("(Int64.to_int32 {})", p(self, &I64)),
                _ => format!("(Int64.to_nativeint {})", p(self, &I64)),
            },
            12 => match t {
                I64 => format!("(Int64.of_nativeint {})", p(self, &Nat)),
                I32 => format!("(Nativeint.to_int32 {})", p(self, &Nat)),
                _ => format!("(Nativeint.of_int {})", p(self, &Int)),
            },
            13 => format!("({m}.{} {} {})", self.r.pick(&["min", "max"]), p(self, t), p(self, t)),
            14 => {
                let x = self.fresh("x");
                let l = p(self, &List(bx(t.clone())));
                let init = p(self, t);
                let acc = self.fresh("acc");
                let env2 = env.with_var(&x, t.clone()).with_var(&acc, t.clone());
                let body = self.pure(t, &env2, d1);
                format!("(List.fold_left (fun {acc} {x} -> {body}) {init} {l})")
            }
            _ => format!("({m}.{} {} {})", self.r.pick(&["add", "sub", "mul", "logxor", "logand"]), p(self, t), self.bint_lit(t)),
        }
    }

    fn pure_str(&mut self, env: &Env, d: u32) -> String {
        if d == 0 {
            return self.str_lit();
        }
        let d1 = d - 1;
        match self.r.below(8) {
            0 | 1 => self.str_lit(),
            2 => format!("({} ^ {})", self.pure(&Str, env, d1), self.pure(&Str, env, d1)),
            3 => format!("(string_of_int {})", self.pure(&Int, env, d1)),
            4 => {
                let t = self.r.pick(&[I32, I64, Nat]).clone();
                format!("({}.to_string {})", bmod(&t), self.pure(&t, env, d1))
            }
            5 => format!("(H.ssub {} {})", self.pure(&Str, env, d1), self.pure(&Int, env, d1)),
            6 => format!("(String.make ({} land 7) 'q')", self.pure(&Int, env, d1)),
            _ => format!("(if {} then \"yes\" else \"no\")", self.pure(&Bool, env, d1)),
        }
    }

    /// `match` in pure position: always has a catch-all, guards are pure.
    fn pure_match(&mut self, t: &Ty, env: &Env, d: u32) -> String {
        let (scrut_ty, arms) = self.patterns(env, d - 1, true);
        let scrut = self.pure(&scrut_ty, env, d - 1);
        let mut s = format!("(match {scrut} with");
        for (pat, binds, guard) in arms {
            let mut env2 = env.clone();
            for (n, ty) in binds {
                env2.vars.push(Var { name: n, ty });
            }
            let g = if guard { format!(" when {}", self.pure(&Bool, &env2, d - 1)) } else { String::new() };
            let body = self.pure(t, &env2, d - 1);
            let _ = write!(s, " | {pat}{g} -> {body}");
        }
        s.push(')');
        s
    }

    /// Picks a scrutinee type and a list of arms (pattern, bound vars, has guard).
    #[allow(clippy::type_complexity)]
    fn patterns(&mut self, _env: &Env, _d: u32, catch_all: bool) -> (Ty, Vec<(String, Vec<(String, Ty)>, bool)>) {
        let mut arms: Vec<(String, Vec<(String, Ty)>, bool)> = Vec::new();
        let n = self.r.range(1, 5);
        // 12..: multi-column matrices, records and bools (lambda/matching.ml
        // splitting, record/field specialisation)
        let ty = match self.r.below(16) {
            0 => Int,
            1 | 2 => V,
            3 => Opt(bx(Int)),
            4 => List(bx(Int)),
            5 => Pair(bx(Int), bx(V)),
            6 => Str,
            7 => Pair(bx(Str), bx(Int)),
            8 => PV,
            9 => Arr(bx(Int)),
            10 => self.r.pick(&[I64, I32, Float, Nat]).clone(),
            11 => Pair(bx(PV), bx(Opt(bx(Str)))),
            12 => Pair(bx(Bool), bx(Pair(bx(Int), bx(Bool)))),
            13 => Pair(bx(V), bx(V)),
            // (matching on a fresh `ref` crashes closure ocamlopt, Selection.size_expr,
            // on ~1% of cases at weight 1/2: kept lower so it does not dominate)
            14 => self.r.pick(&[R, FR, R, FR, R, FR, Ref(bx(Int)), Ref(bx(Opt(bx(V))))]).clone(),
            _ => Pair(bx(Pair(bx(Opt(bx(Int))), bx(List(bx(Bool))))), bx(Int)),
        };
        let pd = if self.r.chance(0.2) { 3 } else { 2 };
        for _ in 0..n {
            let guard = self.r.chance(0.25);
            // Top-level arms are refutable; the only irrefutable arm is the final
            // catch-all. Known bug (agents/harness/matching, also in 5.4.0): an
            // irrefutable arm followed by a guarded or-pattern arm crashes
            // ocamlc ("impossible empty matrix in a Total match").
            let (mut p, mut b) = self.pattern(&ty, pd);
            while p == "_" || p.starts_with('m') {
                (p, b) = self.pattern(&ty, pd);
            }
            // (same bug, general form: a guarded or-pattern arm that is unreachable
            // because earlier arms are exhaustive) -> no guards on or-patterns.
            let guard = guard && !p.contains(" | ");
            arms.push((p, b, guard));
        }
        if catch_all || self.r.chance(0.85) {
            arms.push(("_".into(), vec![], false));
        }
        (ty, arms)
    }

    fn int_pat(&mut self) -> String {
        let k = if self.r.chance(0.15) {
            // sparse constants: switch.ml chooses between jump tables and binary trees
            *self.r.pick(&[63, 64, 100, 255, 256, 1000, -100, 4611686018427387903, -4611686018427387904, 1 << 32])
        } else {
            self.r.below(7) as i64 - 2
        };
        if k < 0 {
            format!("({k})")
        } else {
            k.to_string()
        }
    }
    fn pattern(&mut self, t: &Ty, d: u32) -> (String, Vec<(String, Ty)>) {
        if self.r.chance(0.12) || d == 0 {
            if self.r.chance(0.5) {
                let x = self.fresh("m");
                return (x.clone(), vec![(x, t.clone())]);
            }
            return ("_".into(), vec![]);
        }
        match t {
            Int => {
                if self.r.chance(0.3) {
                    (format!("({} | {})", self.int_pat(), self.int_pat()), vec![])
                } else {
                    (self.int_pat(), vec![])
                }
            }
            V => match self.r.below(8) {
                0 => ("H.V0".into(), vec![]),
                1 => ("H.V1".into(), vec![]),
                2 => ("(H.V0 | H.V1)".into(), vec![]),
                3 => {
                    let (p, b) = self.pattern(&Int, d - 1);
                    (format!("(H.V2 {p})"), b)
                }
                4 => {
                    let x = self.fresh("m");
                    let (p, b) = self.pattern(&Int, d - 1);
                    let mut b = b;
                    b.push((x.clone(), Float));
                    (format!("(H.V3 ({x}, {p}))"), b)
                }
                5 => {
                    let x = self.fresh("m");
                    (format!("(H.V4 {x})"), vec![(x, I64)])
                }
                6 => {
                    let (p, b) = self.pattern(&Int, d - 1);
                    (format!("(H.V5 {{ vx = {p}; _ }})"), b)
                }
                _ => {
                    // or-pattern binding the same variable on both sides
                    let x = self.fresh("m");
                    (format!("(H.V2 {x} | H.V3 (_, {x}) | H.V5 {{ vx = {x}; _ }})"), vec![(x, Int)])
                }
            },
            Str => {
                if self.r.chance(0.3) {
                    (format!("({} | {})", self.r.pick(STR_POOL), self.r.pick(STR_POOL)), vec![])
                } else {
                    (self.r.pick(STR_POOL).to_string(), vec![])
                }
            }
            I32 | I64 | Nat => {
                let suf = suffix(t);
                let k = *self.r.pick(&["0", "1", "(-1)", "2", "7", "255", "(-20)"]);
                let k = if k.starts_with('(') { format!("({}{suf})", &k[1..k.len() - 1]) } else { format!("{k}{suf}") };
                (k, vec![])
            }
            Float => (self.r.pick(&["0.", "1.", "(-1.)", "0.5", "2.", "3.5", "(-2.5)"]).to_string(), vec![]),
            Bool => match self.r.below(5) {
                0 => ("(true | false)".into(), vec![]),
                1 | 2 => ("true".into(), vec![]),
                _ => ("false".into(), vec![]),
            },
            R => {
                // record patterns: some fields, the rest elided
                let (p, mut b) = self.pattern(&Int, d - 1);
                let mut fs = vec![format!("H.ri = {p}")];
                if self.r.chance(0.4) {
                    let (q, b2) = self.pattern(&I64, d - 1);
                    b.extend(b2);
                    fs.push(format!("rl = {q}"));
                }
                if self.r.chance(0.3) {
                    let x = self.fresh("m");
                    b.push((x.clone(), Float));
                    fs.push(format!("rf = {x}"));
                }
                (format!("{{ {}; _ }}", fs.join("; ")), b)
            }
            FR => {
                let (p, b) = self.pattern(&Float, d - 1);
                if self.r.chance(0.5) {
                    (format!("{{ H.fx = {p}; _ }}"), b)
                } else {
                    let x = self.fresh("m");
                    let mut b = b;
                    b.push((x.clone(), Float));
                    (format!("{{ H.fy = {p}; fx = {x} }}"), b)
                }
            }
            Ref(u) => {
                let (p, b) = self.pattern(u, d - 1);
                (format!("{{ contents = {p} }}"), b)
            }
            PV => match self.r.below(6) {
                0 => ("`A".into(), vec![]),
                1 => {
                    let (p, b) = self.pattern(&Int, d - 1);
                    (format!("(`B {p})"), b)
                }
                2 => {
                    let x = self.fresh("m");
                    let (p, mut b) = self.pattern(&Int, d - 1);
                    b.push((x.clone(), Float));
                    (format!("(`C ({x}, {p}))"), b)
                }
                3 => {
                    let (p, b) = self.pattern(&Str, d - 1);
                    (format!("(`D {p})"), b)
                }
                4 => {
                    let x = self.fresh("m");
                    (format!("(`B {x} | `C (_, {x}))"), vec![(x, Int)])
                }
                _ => ("(`A | `D _)".into(), vec![]),
            },
            Arr(u) => {
                let n = self.r.below(4);
                let mut ps = vec![];
                let mut b = vec![];
                for _ in 0..n {
                    let (p, b2) = self.pattern(u, d - 1);
                    ps.push(p);
                    b.extend(b2);
                }
                (format!("[| {} |]", ps.join("; ")), b)
            }
            Opt(u) => {
                if self.r.chance(0.3) {
                    ("None".into(), vec![])
                } else {
                    let (p, b) = self.pattern(u, d - 1);
                    (format!("(Some {p})"), b)
                }
            }
            List(u) => match self.r.below(4) {
                0 => ("[]".into(), vec![]),
                1 => {
                    let (p, b) = self.pattern(u, d - 1);
                    (format!("[{p}]"), b)
                }
                2 => {
                    let (p, mut b) = self.pattern(u, d - 1);
                    let x = self.fresh("m");
                    b.push((x.clone(), t.clone()));
                    (format!("({p} :: {x})"), b)
                }
                _ => {
                    let (p, mut b) = self.pattern(u, d - 1);
                    let (q, b2) = self.pattern(u, d - 1);
                    b.extend(b2);
                    (format!("({p} :: {q} :: _)"), b)
                }
            },
            Pair(a, c) => {
                let (p, mut b) = self.pattern(a, d - 1);
                let (q, b2) = self.pattern(c, d - 1);
                b.extend(b2);
                (format!("({p}, {q})"), b)
            }
            _ => ("_".into(), vec![]),
        }
    }

    // ---------------------------------------------------------------- statements
    /// A block (sequence of statements ending in an expression of type `t`),
    /// parenthesised.
    fn block(&mut self, t: &Ty, env: &Env, depth: u32, nstmts: usize) -> String {
        let mut env = env.clone();
        let mut s = String::from("(");
        let n = if self.fuel < 0 { 0 } else { self.r.range(0, nstmts) };
        for _ in 0..n {
            self.stmt(&mut s, &mut env, depth);
        }
        if *t == Ty::Int && self.r.chance(0.03) {
            s.push_str("raise (H.E1 3))");
            return s;
        }
        let e = if *t == Ty::Bool && self.r.chance(0.0) { "true".into() } else { self.pure(t, &env, 3) };
        s.push_str(&e);
        s.push(')');
        s
    }

    fn obs(&mut self, s: &mut String, env: &Env) {
        // observe a variable or an expression
        let use_var = !env.vars.is_empty() && self.r.chance(0.6);
        if use_var {
            let v = self.r.pick(&env.vars).clone();
            let _ = writeln!(s, "{} {};", v.ty.printer(), v.name);
        } else {
            let t = self.ty(1);
            let e = self.pure(&t, env, 3);
            let _ = writeln!(s, "{} {};", t.printer(), e);
        }
    }

    fn stmt(&mut self, s: &mut String, env: &mut Env, depth: u32) {
        self.fuel -= 2;
        let deep = depth > 0 && self.fuel > 0;
        let k = if self.handler > 0 && self.r.chance(0.15) { 104 } else { self.r.below(128) };
        match k {
            124..=126 if deep => self.gadt_fun(s, env, depth),
            127 => {
                // a minor collection moves young blocks: a value the native code does
                // not register as a root reads stale memory afterwards
                s.push_str("Gc.minor ();\n");
            }
            114..=117 if deep => self.tmc_fun(s, env, depth),
            118..=123 if deep => self.let_rec_values(s, env),
            100..=103 if deep => {
                // effect handler around a block (H.handle_*: fixed handlers in h.ml)
                let t = if self.r.chance(0.5) { Int } else { self.data_ty(1) };
                let h = *self.r.pick(&["H.handle_cont", "H.handle_disc", "H.handle_shallow"]);
                self.handler += 1;
                let b = self.block(&t, env, depth - 1, 5);
                self.handler -= 1;
                let x = self.fresh("hd");
                let _ = writeln!(s, "let {x} = {h} (fun () -> {b}) in");
                env.vars.push(Var { name: x, ty: t });
            }
            110..=113 => {
                // immediate object capturing the environment (translclass, closures)
                let o = self.fresh("o");
                let f = self.fresh("f");
                let n = self.fresh("n");
                let t = self.r.pick(&[Int, Float, I64]).clone();
                let init = self.pure(&t, env, 2);
                let envm = env.with_var(&f, t.clone()).with_var(&n, t.clone());
                let upd = self.pure(&t, &envm, 3);
                let get = self.pure(&t, &env.with_var(&f, t.clone()), 2);
                let _ = writeln!(
                    s,
                    "let {o} = object (self) val mutable {f} = {init} method get = {get} method step ({n} : {}) = {f} <- {upd}; self#get end in",
                    t.ml()
                );
                let reps = self.r.range(1, 3);
                for _ in 0..reps {
                    let x = self.fresh("ob");
                    let a = self.pure(&t, env, 2);
                    let _ = writeln!(s, "let {x} = {o}#step {a} in");
                    env.vars.push(Var { name: x, ty: t.clone() });
                }
            }
            104..=109 if self.handler > 0 || self.r.chance(0.05) => {
                // perform (Effect.Unhandled outside a handler is specified behaviour)
                let x = self.fresh("ef");
                if self.r.chance(0.7) {
                    let e = self.pure(&Int, env, 2);
                    let _ = writeln!(s, "let {x} = Effect.perform (H.Eff {e}) in");
                    env.vars.push(Var { name: x, ty: Int });
                } else {
                    let e = self.pure(&Float, env, 2);
                    let _ = writeln!(s, "let {x} = Effect.perform (H.Eff2 {e}) in");
                    env.vars.push(Var { name: x, ty: Float });
                }
            }
            0..=15 => {
                // let-bound pure value
                let t = self.ty(2);
                let x = self.fresh("v");
                let e = self.pure(&t, env, 3);
                let _ = writeln!(s, "let {x} = {e} in");
                env.vars.push(Var { name: x, ty: t });
            }
            16..=27 => self.obs(s, env),
            28..=35 => {
                // mutation
                let muts: Vec<Var> = env
                    .vars
                    .iter()
                    .filter(|v| matches!(v.ty, Ref(_) | Arr(_) | R | FR | Atom(_)))
                    .cloned()
                    .collect();
                if muts.is_empty() || self.r.chance(0.35) {
                    let t = self.data_ty(1);
                    let x = self.fresh("r");
                    let e = self.pure(&t, env, 2);
                    let (kind, ty) = match self.r.below(3) {
                        0 => (format!("ref {e}"), Ref(bx(t))),
                        1 => (format!("Array.make {} {e}", self.r.range(1, 6)), Arr(bx(t))),
                        _ => {
                            let u = self.r.pick(&[Int, Bool]).clone();
                            let e = self.pure(&u, env, 2);
                            (format!("Atomic.make {e}"), Atom(bx(u)))
                        }
                    };
                    let _ = writeln!(s, "let {x} = {kind} in");
                    env.vars.push(Var { name: x, ty });
                } else {
                    let v = self.r.pick(&muts).clone();
                    match &v.ty {
                        Ref(u) => {
                            let e = self.pure(u, env, 3);
                            let _ = writeln!(s, "{} := {e};", v.name);
                        }
                        Arr(u) => {
                            let i = self.pure(&Int, env, 2);
                            let e = self.pure(u, env, 3);
                            let _ = writeln!(s, "H.set {} {i} {e};", v.name);
                        }
                        R => {
                            if self.r.chance(0.5) {
                                let e = self.pure(&Int, env, 3);
                                let _ = writeln!(s, "{}.H.ri <- {e};", v.name);
                            } else {
                                let e = self.pure(&I64, env, 3);
                                let _ = writeln!(s, "{}.H.rl <- {e};", v.name);
                            }
                        }
                        FR => {
                            let e = self.pure(&Float, env, 3);
                            let _ = writeln!(s, "{}.H.fx <- {e};", v.name);
                        }
                        Atom(u) => self.atomic_op(s, env, &v.name, u),
                        _ => unreachable!(),
                    }
                }
            }
            36..=43 => self.call(s, env),
            44..=49 if deep => {
                let c = self.pure(&Bool, env, 3);
                let a = self.block(&Ty::Bool, env, depth - 1, 4);
                let _ = a;
                let a = self.unit_block(env, depth - 1, 4);
                if self.r.chance(0.5) {
                    let b = self.unit_block(env, depth - 1, 4);
                    let _ = writeln!(s, "if {c} then {a} else {b};");
                } else {
                    let _ = writeln!(s, "if {c} then {a};");
                }
            }
            50..=54 if deep => {
                let i = self.fresh("i");
                let lo = self.pure(&Int, env, 2);
                let hi = self.pure(&Int, env, 2);
                let dir = if self.r.chance(0.8) { "to" } else { "downto" };
                let (lo, hi) = if dir == "to" {
                    (format!("({lo} land 3)"), format!("({hi} land 7)"))
                } else {
                    (format!("({lo} land 7)"), format!("({hi} land 3)"))
                };
                let env2 = env.with_var(&i, Int);
                let b = self.unit_block(&env2, depth - 1, 4);
                let _ = writeln!(s, "for {i} = {lo} {dir} {hi} do {b} done;");
            }
            55..=57 if deep => {
                let c = self.fresh("c");
                let n = self.r.range(1, 8);
                let env2 = env.with_var(&c, Ref(bx(Int)));
                let cond = self.pure(&Bool, &env2, 2);
                // the body must not see the counter (it could reset it: infinite loop)
                let b = self.unit_block(env, depth - 1, 4);
                let _ = writeln!(s, "let {c} = ref 0 in\nwhile !{c} < {n} && {cond} do incr {c}; {b} done;");
            }
            58..=64 if deep => {
                // match statement (may be non-exhaustive -> Match_failure, ordered)
                let t = if self.r.chance(0.5) { Ty::Int } else { self.data_ty(1) };
                let (sty, arms) = self.patterns(env, 2, false);
                let scrut = self.pure(&sty, env, 3);
                let x = self.fresh("w");
                let mut m = format!("let {x} = (match {scrut} with\n");
                for (pat, binds, guard) in arms {
                    let mut env2 = env.clone();
                    for (n, ty) in binds {
                        env2.vars.push(Var { name: n, ty });
                    }
                    let g = if guard { format!(" when {}", self.pure(&Bool, &env2, 2)) } else { String::new() };
                    let body = self.block(&t, &env2, depth - 1, 3);
                    let _ = writeln!(m, " | {pat}{g} -> {body}");
                }
                m.push_str(") in\n");
                s.push_str(&m);
                env.vars.push(Var { name: x, ty: t });
            }
            65..=69 if deep => {
                // try
                let t = self.data_ty(1);
                let x = self.fresh("w");
                let b = self.block(&t, env, depth - 1, 4);
                let mut h = String::new();
                let mut hs = vec!["H.E0".to_string(), "Division_by_zero".into(), "Not_found".into(), "(Invalid_argument _)".into(), "(Failure _)".into(), "(Match_failure _)".into()];
                let n = self.r.range(1, 3);
                for _ in 0..n {
                    let i = self.r.below(hs.len() + 1);
                    if i == hs.len() {
                        let e = self.fresh("e");
                        let env2 = env.with_var(&e, Int);
                        let body = self.pure(&t, &env2, 2);
                        let _ = write!(h, " | H.E1 {e} -> {body}");
                    } else {
                        let p = hs.remove(i);
                        let body = self.pure(&t, env, 2);
                        let _ = write!(h, " | {p} -> {body}");
                    }
                }
                if self.r.chance(0.3) {
                    let body = self.pure(&t, env, 2);
                    let _ = write!(h, " | _ -> {body}");
                }
                let _ = writeln!(s, "let {x} = try {b} with{h} in");
                env.vars.push(Var { name: x, ty: t });
            }
            70..=72 => {
                // conditional raise
                let c = self.pure(&Bool, env, 2);
                let e = match self.r.below(3) {
                    0 => "H.E0".to_string(),
                    1 => format!("(H.E1 {})", self.pure(&Int, env, 2)),
                    _ => format!("(H.E2 ({}, {}))", self.pure(&Str, env, 1), self.pure(&Float, env, 2)),
                };
                let _ = writeln!(s, "if {c} then raise {e};");
            }
            73..=76 => {
                // risky op, alone in its let: may raise, ordered
                let x = self.fresh("q");
                let (e, t) = match self.r.below(6) {
                    0 => (format!("{} / {}", self.pure(&Int, env, 2), self.pure(&Int, env, 2)), Int),
                    1 => (format!("{} mod {}", self.pure(&Int, env, 2), self.pure(&Int, env, 2)), Int),
                    2 => (format!("Int64.div {} {}", self.pure(&I64, env, 2), self.pure(&I64, env, 2)), I64),
                    3 => {
                        let a = self.pure(&Arr(bx(Float)), env, 2);
                        (format!("{a}.({} land 7)", self.pure(&Int, env, 2)), Float)
                    }
                    4 => (format!("List.hd {}", self.pure(&List(bx(Int)), env, 2)), Int),
                    _ => (format!("Option.get {}", self.pure(&Opt(bx(I64)), env, 2)), I64),
                };
                let _ = writeln!(s, "let {x} = {e} in");
                env.vars.push(Var { name: x, ty: t });
            }
            77..=83 if deep => self.local_fun(s, env, depth),
            84..=87 if deep => self.local_loop(s, env, depth),
            88..=90 if self.r.chance(0.4) => self.atomic_loc_op(s, env),
            88..=90 => {
                // atomics
                let ats: Vec<Var> = env.vars.iter().filter(|v| matches!(v.ty, Atom(_))).cloned().collect();
                if ats.is_empty() {
                    let x = self.fresh("at");
                    let u = self.r.pick(&[Int, Int, Bool, Ref(bx(Int))]).clone();
                    let e = self.pure(&u, env, 2);
                    let _ = writeln!(s, "let {x} = Atomic.make {e} in");
                    env.vars.push(Var { name: x, ty: Atom(bx(u)) });
                } else {
                    let v = self.r.pick(&ats).clone();
                    if let Atom(u) = &v.ty {
                        self.atomic_op(s, env, &v.name, u);
                    }
                }
            }
            91..=93 if deep => {
                // lazy
                let t = self.data_ty(1);
                let x = self.fresh("lz");
                let b = self.block(&t, env, depth - 1, 3);
                let y = self.fresh("v");
                let _ = writeln!(s, "let {x} = lazy {b} in");
                let extra = self.unit_block(env, 0, 2);
                let _ = writeln!(s, "{extra};\nlet {y} = Lazy.force {x} in");
                env.vars.push(Var { name: y, ty: t });
            }
            _ => {
                // higher-order stdlib call with a possibly effectful function value
                let funs: Vec<Var> = env.vars.iter().filter(|v| matches!(v.ty, Fun(..))).cloned().collect();
                if let Some(f) = funs.first().map(|_| self.r.pick(&funs).clone()) {
                    if let Fun(a, b) = &f.ty {
                        let x = self.fresh("h");
                        match self.r.below(3) {
                            0 => {
                                let l = self.pure(&List((*a).clone()), env, 2);
                                let _ = writeln!(s, "let {x} = List.map {} {l} in", f.name);
                                env.vars.push(Var { name: x, ty: List((*b).clone()) });
                            }
                            1 => {
                                let l = self.pure(&Arr((*a).clone()), env, 2);
                                let _ = writeln!(s, "let {x} = Array.map {} {l} in", f.name);
                                env.vars.push(Var { name: x, ty: Arr((*b).clone()) });
                            }
                            _ => {
                                let e = self.pure(a, env, 2);
                                let _ = writeln!(s, "let {x} = {} {e} in", f.name);
                                env.vars.push(Var { name: x, ty: (**b).clone() });
                            }
                        }
                    }
                } else {
                    self.obs(s, env);
                }
            }
        }
    }

    fn unit_block(&mut self, env: &Env, depth: u32, n: usize) -> String {
        let mut env = env.clone();
        let mut s = String::from("(");
        let k = if self.fuel < 0 { 1 } else { self.r.range(1, n) };
        for _ in 0..k {
            self.stmt(&mut s, &mut env, depth);
        }
        s.push_str("())");
        s
    }

    fn atomic_op(&mut self, s: &mut String, env: &mut Env, a: &str, u: &Ty) {
        self.atomic_op_at(s, env, "Atomic", a, u, true)
    }

    /// An atomic operation on an atomic location: `m` is the module
    /// (`Atomic`, `Atomic.Loc`, `Atomic.Array`), `a` the location argument(s)
    /// (`x`, `[%atomic.loc r.H.ai]`, `arr (i land 3)`); `incr` whether the
    /// module has incr/decr (Atomic.Array has not).
    fn atomic_op_at(&mut self, s: &mut String, env: &mut Env, m: &str, a: &str, u: &Ty, incr: bool) {
        let x = self.fresh("ao");
        match self.r.below(8) {
            0 => {
                let _ = writeln!(s, "let {x} = {m}.get {a} in");
                env.vars.push(Var { name: x, ty: u.clone() });
            }
            1 => {
                let e = self.pure(u, env, 2);
                let _ = writeln!(s, "{m}.set {a} {e};");
            }
            2 => {
                let e = self.pure(u, env, 2);
                let _ = writeln!(s, "let {x} = {m}.exchange {a} {e} in");
                env.vars.push(Var { name: x, ty: u.clone() });
            }
            3 | 4 => {
                // compare_and_set uses physical equality: only immediates or refs
                // (mutable, so physical equality is well defined)
                let exp = if self.r.chance(0.4) { format!("({m}.get {a})") } else { self.pure(u, env, 1) };
                let exp = if matches!(u, Ref(_)) && !exp.starts_with('(') {
                    match env.vars_of(u).first() {
                        Some(_) => self.r.pick(&env.vars_of(u)).name.clone(),
                        None => format!("({m}.get {a})"),
                    }
                } else {
                    exp
                };
                let nw = self.pure(u, env, 2);
                let _ = writeln!(s, "let {x} = {m}.compare_and_set {a} {exp} {nw} in");
                env.vars.push(Var { name: x, ty: Bool });
            }
            5 if *u == Int => {
                let e = self.pure(&Int, env, 2);
                let _ = writeln!(s, "let {x} = {m}.fetch_and_add {a} {e} in");
                env.vars.push(Var { name: x, ty: Int });
            }
            6 if *u == Int && incr => {
                let f = self.r.pick(&["incr", "decr"]);
                let _ = writeln!(s, "{m}.{f} {a};");
            }
            7 if *u == Int || *u == Bool => {
                // update with a pure, total function (no concurrent writer: f runs once)
                let y = self.fresh("x");
                let mut env2 = env.clone();
                env2.vars.push(Var { name: y.clone(), ty: u.clone() });
                let e = self.pure(u, &env2, 2);
                let _ = writeln!(s, "{m}.update (fun {y} -> {e}) {a};");
            }
            _ => {
                let _ = writeln!(s, "{} ({m}.get {a});", u.printer());
            }
        }
    }

    /// Atomic record fields and atomic arrays (trunk 5.4+/5.6 features).
    fn atomic_loc_op(&mut self, s: &mut String, env: &mut Env) {
        let locs: Vec<Var> = env.vars.iter().filter(|v| matches!(v.ty, AR | AA)).cloned().collect();
        if locs.is_empty() || self.r.chance(0.25) {
            let x = self.fresh("al");
            if self.r.chance(0.5) {
                let (i, b, p) = (self.pure(&Int, env, 2), self.pure(&Bool, env, 2), self.pure(&Int, env, 2));
                let _ = writeln!(s, "let {x} = {{ H.ai = {i}; ab = {b}; ap = {p} }} in");
                env.vars.push(Var { name: x, ty: AR });
            } else {
                let e = self.pure(&Int, env, 2);
                if self.r.chance(0.5) {
                    let _ = writeln!(s, "let {x} = Atomic.Array.make 4 {e} in");
                } else {
                    let j = self.fresh("j");
                    let mut env2 = env.clone();
                    env2.vars.push(Var { name: j.clone(), ty: Int });
                    let e = self.pure(&Int, &env2, 2);
                    let _ = writeln!(s, "let {x} = Atomic.Array.init 4 (fun {j} -> {e}) in");
                }
                env.vars.push(Var { name: x, ty: AA });
            }
            return;
        }
        let v = self.r.pick(&locs).clone();
        match v.ty {
            AR => match self.r.below(5) {
                0 => {
                    let e = self.pure(&Int, env, 2);
                    let _ = writeln!(s, "{}.H.ap <- {e};", v.name);
                }
                1 => {
                    // plain write to an atomic field (allowed: a release store)
                    let e = self.pure(&Int, env, 2);
                    let _ = writeln!(s, "{}.H.ai <- {e};", v.name);
                }
                2 => {
                    let a = format!("[%atomic.loc {}.H.ab]", v.name);
                    self.atomic_op_at(s, env, "Atomic.Loc", &a, &Bool, true);
                }
                _ => {
                    let a = format!("[%atomic.loc {}.H.ai]", v.name);
                    self.atomic_op_at(s, env, "Atomic.Loc", &a, &Int, true);
                }
            },
            _ => {
                // indices always in bounds (land 3, length 4); unsafe_ variants too
                let i = self.pure_int(env, 1);
                let pre = if self.r.chance(0.3) { "unsafe_" } else { "" };
                let a = format!("{} (({i}) land 3)", v.name);
                let mut op = String::new();
                self.atomic_op_at(&mut op, env, "Atomic.Array", &a, &Int, false);
                if pre.is_empty() {
                    s.push_str(&op);
                } else {
                    s.push_str(&op.replacen("Atomic.Array.", "Atomic.Array.unsafe_", 1).replace("Atomic.Array.unsafe_length", "Atomic.Array.length"));
                }
            }
        }
    }

    fn call(&mut self, s: &mut String, env: &mut Env) {
        if env.funcs.is_empty() {
            return self.obs(s, env);
        }
        let f = self.r.pick(&env.funcs).clone();
        let args: Vec<String> = f.params.iter().map(|t| self.pure(t, env, 2)).collect();
        let x = self.fresh("c");
        let attr = if self.r.chance(0.15) { "[@inlined never]" } else if self.r.chance(0.1) { "[@inlined]" } else { "" };
        let call = format!("({}{attr}) {}", f.name, args.join(" "));
        if self.r.chance(0.2) {
            // match with exception branch
            let e = self.fresh("e");
            let dflt = self.pure(&f.ret, env, 1);
            let _ = writeln!(
                s,
                "let {x} = match {call} with r -> r | exception H.E1 {e} -> (H.p_int {e}; {dflt}) | exception Division_by_zero -> {dflt} in"
            );
        } else {
            let _ = writeln!(s, "let {x} = {call} in");
        }
        env.vars.push(Var { name: x, ty: f.ret.clone() });
    }

    /// A local function matching on a GADT witness (`H.w`) together with a
    /// payload of the witnessed type, then 1-3 calls (some through the
    /// existential `H.gb`). Shapes follow testsuite/tests/basic/gadt_immediate_fields.ml
    /// and typeopt/partial_gadt.ml: tuple, curried `match w, x`, constructor or
    /// record of a local type mentioning the abstract type, GADT pattern on a
    /// parameter (partial: Match_failure, alone in its let). Arms observe the
    /// payload after a `Gc.minor ()` (a field read wrongly classified as
    /// immediate is not a GC root).
    fn gadt_fun(&mut self, s: &mut String, env: &mut Env, depth: u32) {
        let cons: [(&str, Ty); 5] = [("WI", Int), ("WS", Str), ("WF", Float), ("WL", List(bx(Int))), ("WP", Pair(bx(Int), bx(Float)))];
        let name = self.fresh("gf");
        let shape = self.r.below(7);
        // arms: a subset in random order; partial when some are missing (no wildcard)
        let mut idx: Vec<usize> = (0..5).collect();
        for i in (1..idx.len()).rev() {
            let j = self.r.below(i + 1);
            idx.swap(i, j);
        }
        let keep = if shape == 5 { 1 } else if self.r.chance(0.6) { 5 } else { self.r.range(1, 4) };
        idx.truncate(keep);
        let wild = keep < 5 && shape != 5 && self.r.chance(0.4);
        let attr = *self.r.pick(&["", "", "[@inline never]", "[@inline]"]);
        let mut arms = String::new();
        for &c in &idx {
            let (cn, ty) = &cons[c];
            let x = self.fresh("gx");
            let env2 = env.with_var(&x, ty.clone());
            let gc = if self.r.chance(0.5) { "Gc.minor (); " } else { "" };
            let body = self.block(&Int, &env2, depth - 1, 3);
            // rebound at its refined type: lets in the body that derive from it would
            // otherwise have the ambiguous type `a` (escaping its equation)
            let src = if matches!(shape, 4 | 6) { "gp" } else { x.as_str() };
            let arm_body = format!("(let {x} : {} = {src} in {gc}{} {x}; {body})", ty.ml(), ty.printer());
            let pat = match shape {
                0 => format!("({x}, H.{cn})"),
                1 => format!("H.{cn}, {x}"),
                2 => format!("M.C ({x}, H.{cn})"),
                3 => format!("{{ M.gw = H.{cn}; gv = {x} }}"),
                _ => format!("H.{cn}"),
            };
            match shape {
                5 => arms = format!("({x} : a) : int = {arm_body}"),
                _ => {
                    let _ = writeln!(arms, "  | {pat} -> {arm_body}");
                }
            }
        }
        if wild {
            let d = self.pure(&Int, env, 2);
            let _ = writeln!(arms, "  | _ -> {d}");
        }
        let def = match shape {
            0 => format!("let{attr} {name} = fun (type a) (p : a * a H.w) : int -> match p with\n{arms} in\n"),
            1 => format!("let{attr} {name} = fun (type a) (gp : a) (w : a H.w) : int -> match w, gp with\n{arms} in\n"),
            2 => format!(
                "let{attr} {name} = fun (type a) (gp : a) (w : a H.w) : int ->\n let module M = struct type u = C of a * a H.w end in\n match Sys.opaque_identity (M.C (gp, w)) with\n{arms} in\n"
            ),
            3 => format!(
                "let{attr} {name} = fun (type a) (gp : a) (w : a H.w) : int ->\n let module M = struct type u = {{ gv : a; gw : a H.w }} end in\n match Sys.opaque_identity {{ M.gv = gp; gw = w }} with\n{arms} in\n"
            ),
            4 => format!("let{attr} {name} = fun (type a) (w : a H.w) (gp : a) : int -> match w with\n{arms} in\n"),
            5 => format!("let{attr} {name} (type a) (H.{} : a H.w) {arms} in\n", cons[idx[0]].0),
            _ => format!("let{attr} {name} : type a. a H.w -> a -> int = fun w gp -> match w with\n{arms} in\n"),
        };
        s.push_str(&def);
        let ncalls = self.r.range(1, 3);
        for _ in 0..ncalls {
            // mostly a constructor the function handles
            let c = if self.r.chance(0.8) { idx[self.r.below(idx.len())] } else { self.r.below(5) };
            let (cn, ty) = &cons[c];
            let v = if *ty == Str && self.r.chance(0.6) { format!("(H.fresh_str {})", self.pure(&Int, env, 2)) } else { self.pure(ty, env, 2) };
            let call = |a: &str, w: &str| -> String {
                match shape {
                    0 => format!("{name} ({a}, {w})"),
                    1 | 2 | 3 => format!("{name} {a} {w}"),
                    _ => format!("{name} {w} {a}"),
                }
            };
            let x = self.fresh("g");
            if self.r.chance(0.3) {
                let e = format!("(match Sys.opaque_identity (H.G ({v}, H.{cn})) with H.G (ga, gw) -> {})", call("ga", "gw"));
                let _ = writeln!(s, "let {x} = {e} in");
            } else {
                let _ = writeln!(s, "let {x} = {} in", call(&v, &format!("H.{cn}")));
            }
            env.vars.push(Var { name: x, ty: Int });
        }
    }

    fn local_fun(&mut self, s: &mut String, env: &mut Env, depth: u32) {
        let name = self.fresh("lf");
        let np = self.r.range(1, 3);
        let mut params = vec![];
        let mut env2 = env.clone();
        let mut ps = String::new();
        for _ in 0..np {
            let t = if self.r.chance(0.5) { self.scalar() } else { self.ty(1) };
            let p = self.fresh("a");
            let _ = write!(ps, " ({p} : {})", t.ml());
            env2.vars.push(Var { name: p, ty: t.clone() });
            params.push(t);
        }
        let ret = if self.r.chance(0.6) { self.scalar() } else { self.data_ty(1) };
        let attr = *self.r.pick(&["", "", "", "[@inline never]", "[@inline]", "[@inline always]"]);
        let body = self.block(&ret, &env2, depth - 1, 5);
        let _ = writeln!(s, "let{attr} {name}{ps} : {} = {body} in", ret.ml());
        if params.len() == 1 && matches!((&params[0], &ret), (Int, Int) | (Float, Float) | (Int, Float) | (I64, I64)) {
            env.vars.push(Var { name: name.clone(), ty: Fun(bx(params[0].clone()), bx(ret.clone())) });
        }
        env.funcs.push(Func { name, params, ret });
    }

    /// `[@tail_mod_cons]` list builder (lambda/tmc.ml): the recursive call is
    /// under a constructor, sometimes under `if`/`match`/`let`, with ordered
    /// effects before it. Bounded by the counter.
    fn tmc_fun(&mut self, s: &mut String, env: &mut Env, depth: u32) {
        let name = self.fresh("tm");
        let n = self.fresh("n");
        let k = self.fresh("k");
        let t = if self.r.chance(0.5) { Int } else { self.data_ty(1) };
        let kt = if self.r.chance(0.5) { Int } else { self.scalar() };
        let env2 = env.with_var(&n, Int).with_var(&k, kt.clone());
        let pre = if self.r.chance(0.5) { format!("{}; ", self.unit_block(&env2, depth.saturating_sub(1).min(1), 2)) } else { String::new() };
        let e = self.pure(&t, &env2, 2);
        let e2 = self.pure(&t, &env2, 2);
        let k1 = self.pure(&kt, &env2, 2);
        let c = self.pure(&Bool, &env2, 2);
        let rec_ = |m: &str, kk: &str| format!("{name} ({n} - {m}) {kk}");
        let body = match self.r.below(5) {
            0 => format!("{e} :: {}", rec_("1", &k1)),
            1 => format!("if {c} then {e} :: {} else {e2} :: {e} :: {}", rec_("1", &k1), rec_("1", &k)),
            2 => format!("if {c} then {} else {e} :: {}", rec_("1", &k1), rec_("2", &k)),
            3 => {
                let x = self.fresh("x");
                let env3 = env2.with_var(&x, t.clone());
                let e3 = self.pure(&t, &env3, 2);
                format!("let {x} = {e} in {e3} :: {x} :: {}", rec_("1", &k1))
            }
            _ => {
                let (sty, arms) = self.patterns(&env2, 1, true);
                let scrut = self.pure(&sty, &env2, 2);
                let mut m = format!("match {scrut} with");
                for (i, (pat, binds, guard)) in arms.into_iter().enumerate() {
                    let mut env3 = env2.clone();
                    for (bn, ty) in binds {
                        env3.vars.push(Var { name: bn, ty });
                    }
                    let g = if guard { format!(" when {}", self.pure(&Bool, &env3, 1)) } else { String::new() };
                    let v = self.pure(&t, &env3, 2);
                    let arm = if i % 2 == 0 { format!("{v} :: {}", rec_("1", &k1)) } else { format!("[{v}] @ {}", rec_("1", &k)) };
                    let _ = write!(m, " | {pat}{g} -> {arm}");
                }
                format!("({m})")
            }
        };
        let attr = if self.r.chance(0.85) { "[@tail_mod_cons]" } else { "" };
        let _ = writeln!(
            s,
            "let{attr} rec {name} ({n} : int) ({k} : {}) : {} = if {n} <= 0 || {n} > 40 then [] else begin {pre}{body} end in",
            kt.ml(),
            List(bx(t.clone())).ml()
        );
        let x = self.fresh("v");
        let cnt = self.pure(&Int, env, 2);
        let ka = self.pure(&kt, env, 2);
        let _ = writeln!(s, "let {x} = {name} ({cnt} land 31) {ka} in");
        env.vars.push(Var { name: x, ty: List(bx(t.clone())) });
        env.funcs.push(Func { name, params: vec![Int, kt], ret: List(bx(t)) });
    }

    /// Recursive value definitions (lambda/value_rec_compiler.ml): a cyclic
    /// list, mutually recursive closures, lazy and tuple values referring to
    /// them. Only finite projections of the cyclic values are observed.
    fn let_rec_values(&mut self, s: &mut String, env: &mut Env) {
        if self.r.chance(0.65) {
            let (d, obs) = self.rec_group(env);
            let _ = writeln!(s, "{d} in");
            for (name, e, ty) in obs {
                if name != e {
                    let _ = writeln!(s, "let {name} = {e} in");
                }
                env.vars.push(Var { name, ty });
            }
            return;
        }
        let t = self.data_ty(1);
        let (f, g, cyc, lz, pr, x, y) = (self.fresh("rf"), self.fresh("rg"), self.fresh("cy"), self.fresh("lz"), self.fresh("pr"), self.fresh("x"), self.fresh("y"));
        let ex = env.with_var(&x, Int);
        let base = self.pure(&Int, &ex, 2);
        let k = self.pure(&Int, &ex, 1);
        let ey = env.with_var(&y, Int);
        let gb = self.pure(&Int, &ey, 1);
        let elems: Vec<String> = (0..self.r.range(1, 3)).map(|_| self.pure(&t, env, 2)).collect();
        let mut d = format!(
            "let rec {f} = (fun ({x} : int) -> if {x} <= 0 || {x} > 30 then {base} else {g} ({x} - 1) + {k})\nand {g} = (fun ({y} : int) -> {f} ({y} - 1) lxor {gb})\nand {cyc} = {} :: {cyc}",
            elems.join(" :: ")
        );
        let with_lz = self.r.chance(0.6);
        if with_lz {
            let _ = write!(d, "\nand {lz} = lazy ({f} 7 + List.length (H.take 3 {cyc}))");
        }
        let with_pr = self.r.chance(0.5);
        if with_pr {
            let _ = write!(d, "\nand {pr} = ({g}, Some {cyc})");
        }
        let _ = writeln!(s, "{d} in");
        let (v1, v2) = (self.fresh("v"), self.fresh("v"));
        let a = self.pure(&Int, env, 2);
        let n = self.pure(&Int, env, 1);
        let _ = writeln!(s, "let {v1} = {f} ({a} land 31) in\nlet {v2} = H.take ({n} land 15) {cyc} in");
        env.vars.push(Var { name: v1, ty: Int });
        env.vars.push(Var { name: v2, ty: List(bx(t.clone())) });
        if with_lz {
            let v3 = self.fresh("v");
            let _ = writeln!(s, "let {v3} = Lazy.force {lz} in");
            env.vars.push(Var { name: v3, ty: Int });
        }
        if with_pr {
            let v4 = self.fresh("v");
            let _ = writeln!(s, "let {v4} = (match {pr} with (h, Some l) -> h (List.length (H.take 4 l)) | (_, None) -> 0) in");
            env.vars.push(Var { name: v4, ty: Int });
        }
        env.vars.push(Var { name: f.clone(), ty: Fun(bx(Int), bx(Int)) });
    }

    // ------------------------------------------------ generic recursive definitions
    /// A random `let rec` group in the shapes `typing/value_rec_check.ml`
    /// accepts (checked against ocamlc: recursive names only as bare fields
    /// or under `fun`/`lazy`; wrappers: lets of pure values, aliases of other
    /// recursive names, irrefutable array/tuple/record/lazy patterns, local
    /// modules/exceptions/refs, local (rec) functions, nested `let rec`),
    /// aimed at `lambda/value_rec_compiler.ml` (sizing, function lifting,
    /// backpatching of regular/float/lazy blocks). Every recursive function
    /// is guarded (`x <= 0 || x > 10`) and only calls with `x - 1`; lazies
    /// force only earlier lazies, functions never force: no Undefined, no
    /// unbounded recursion, nothing raises. Returns the definition (without
    /// `in`) and observations `(name, expr, ty)` of finite projections.
    fn rec_group(&mut self, env: &Env) -> (String, Vec<(String, String, Ty)>) {
        let n = self.r.range(1, 4);
        let mut rb: Vec<(String, RK)> = Vec::new();
        for _ in 0..n {
            let k = *self.r.pick(&[RK::Fun, RK::Fun, RK::Fun, RK::Knot, RK::Knot, RK::Kv, RK::Kv, RK::Lst, RK::FRec, RK::Lz, RK::Lz, RK::FunArr, RK::FltArr, RK::IntArr, RK::Konst]);
            rb.push((self.fresh("rv"), k));
        }
        let mut defs = vec![];
        for i in 0..n {
            let rhs = self.rec_rhs(env, &rb, i, 0);
            defs.push(format!("{} = {rhs}", rb[i].0));
        }
        let d = format!("let rec {}", defs.join("\nand "));
        let mut obs = vec![];
        for (name, k) in rb.iter() {
            let o = self.fresh("ro");
            let a = self.pure(&Int, env, 1);
            let b = self.pure(&Int, env, 1);
            match k {
                RK::Fun => {
                    obs.push((o, format!("{name} ({a} land 15)"), Int));
                    obs.push((name.clone(), name.clone(), Fun(bx(Int), bx(Int))));
                }
                RK::Knot => {
                    obs.push((o, format!("H.ksum ({a} land 7) {name} + {name}.H.kg ({b} land 15)"), Int));
                    obs.push((self.fresh("ro"), format!("{name}.H.kr"), FR));
                }
                RK::Kv => obs.push((o, format!("H.kvsum ({a} land 7) {name}"), Int)),
                RK::Lst => obs.push((o, format!("H.take ({a} land 15) {name}"), List(bx(Int)))),
                RK::FRec => obs.push((o, name.clone(), FR)),
                RK::Lz => obs.push((o, format!("Lazy.force {name}"), Int)),
                RK::FunArr => obs.push((o, format!("(H.get {name} {a}) ({b} land 15) + Array.length {name}"), Int)),
                RK::FltArr => obs.push((o, name.clone(), Arr(bx(Float)))),
                RK::IntArr => obs.push((o, name.clone(), Arr(bx(Int)))),
                RK::Konst => obs.push((o, name.clone(), Int)),
            }
        }
        (d, obs)
    }

    /// Right-hand side for binding `me` of kind `rb[me].1`; `depth` limits
    /// nested `let rec`s.
    fn rec_rhs(&mut self, env: &Env, rb: &[(String, RK)], me: usize, depth: u32) -> String {
        let k = rb[me].1;
        let mut env = env.clone();
        let mut rb: Vec<(String, RK)> = rb.to_vec();
        let mut pre = String::new();
        let nw = if self.r.chance(0.4) { 0 } else { self.r.range(1, 3) };
        for _ in 0..nw {
            let w = self.rec_wrapper(&mut env, &mut rb, me, k);
            pre.push_str(&w);
            pre.push(' ');
        }
        let core = if depth == 0 && matches!(k, RK::Knot | RK::Kv | RK::Lst) && self.r.chance(0.2) {
            // nested recursive definition of the same kind, returned
            let y = self.fresh("ry");
            rb.push((y.clone(), k));
            let me2 = rb.len() - 1;
            let inner = self.rec_rhs(&env, &rb, me2, depth + 1);
            format!("let rec {y} = {inner} in {y}")
        } else {
            self.rec_core(&env, &rb, me, k)
        };
        format!("({pre}{core})")
    }

    fn rec_wrapper(&mut self, env: &mut Env, rb: &mut Vec<(String, RK)>, me: usize, k: RK) -> String {
        let a = self.fresh("w");
        // a Konst binding is Dynamic for value_rec_check: no recursive names in it
        let n = if k == RK::Konst { 8 } else { 16 };
        match self.r.below(n) {
            // refutable constant patterns whose scrutinee always matches: the
            // failure branch is the Unreachable case of the sizing/splitting
            12 => {
                let e = self.pure(&Int, env, 2);
                env.vars.push(Var { name: a.clone(), ty: Int });
                if self.r.chance(0.5) {
                    format!("let ((0 | 1 | 2 | 3) as {a}) = ({e}) land 3 in")
                } else {
                    format!("let ((0 | 1 | 2 | 3 | 4 | 5 | 6 | 7) as {a}) = ({e}) land 7 in")
                }
            }
            13 => {
                let e = self.pure(&Int, env, 2);
                env.vars.push(Var { name: a.clone(), ty: Str });
                format!("let ((\"a\" | \"ab\" | \"abcdefghi\") as {a}) = (match ({e}) land 3 with 0 -> \"a\" | 1 -> \"ab\" | _ -> \"abcdefghi\") in")
            }
            14 => {
                let e = self.pure(&Int, env, 2);
                format!("let ('a'..'d') = Char.chr (97 + (({e}) land 3)) in")
            }
            15 => {
                let (e1, e2) = (self.pure(&Int, env, 2), self.pure(&Int, env, 2));
                env.vars.push(Var { name: a.clone(), ty: Int });
                format!("let ([| {a} |] | [| _; {a} |] | [| _; _; {a} |]) = Array.make (1 + (({e1}) land 1)) {e2} in")
            }
            0 | 1 => {
                let t = if self.r.chance(0.6) { self.scalar() } else { self.data_ty(1) };
                let e = self.pure(&t, env, 2);
                env.vars.push(Var { name: a.clone(), ty: t });
                format!("let {a} = {e} in")
            }
            2 => {
                let b = self.fresh("w");
                let (e1, e2) = (self.pure(&Int, env, 2), self.pure(&Int, env, 2));
                env.vars.push(Var { name: a.clone(), ty: Int });
                if self.r.chance(0.5) {
                    format!("let [| {a} |] = [| {e1} |] in")
                } else {
                    env.vars.push(Var { name: b.clone(), ty: Int });
                    format!("let [| {a}; {b} |] = [| {e1}; {e2} |] in")
                }
            }
            3 => {
                let b = self.fresh("w");
                let t1 = self.scalar();
                let t2 = self.data_ty(1);
                let (e1, e2) = (self.pure(&t1, env, 2), self.pure(&t2, env, 2));
                env.vars.push(Var { name: a.clone(), ty: t1 });
                env.vars.push(Var { name: b.clone(), ty: t2 });
                format!("let ({a}, {b}) = ({e1}, {e2}) in")
            }
            4 => {
                let e = self.pure(&FR, env, 2);
                env.vars.push(Var { name: a.clone(), ty: Float });
                format!("let {{ H.fy = {a}; _ }} = {e} in")
            }
            5 => {
                let m = self.fresh("M");
                let t = self.scalar();
                let e = self.pure(&t, env, 2);
                env.vars.push(Var { name: format!("{m}.c"), ty: t });
                format!("let module {m} = struct let c = {e} end in")
            }
            6 => {
                let x = self.fresh("X");
                format!("let exception {x} of int in")
            }
            7 => {
                let t = self.scalar();
                let e = self.pure(&t, env, 2);
                env.vars.push(Var { name: a.clone(), ty: t });
                format!("let lazy {a} = lazy {e} in")
            }
            8 => {
                // alias of another recursive name (should_inline / Lvar)
                let cands: Vec<(String, RK)> = rb.iter().enumerate().filter(|(i, (_, kk))| *i != me && *kk != RK::Konst && *kk != RK::Lz).map(|(_, x)| x.clone()).collect();
                if cands.is_empty() {
                    return String::new();
                }
                let (n, kk) = self.r.pick(&cands).clone();
                rb.push((a.clone(), kk));
                format!("let {a} = {n} in")
            }
            9 => {
                let e = self.pure(&Int, env, 2);
                format!("ignore {e};")
            }
            10 => {
                let e = self.pure(&Int, env, 2);
                env.vars.push(Var { name: a.clone(), ty: Ref(bx(Int)) });
                if k == RK::Fun || self.r.chance(0.3) {
                    format!("let {a} = ref {e} in")
                } else {
                    // not captured: becomes a mutable variable (Lmutlet, Lassign)
                    let e2 = self.pure(&Int, env, 1);
                    format!("let {a} = ref {e} in {a} := !{a} + {e2};")
                }
            }
            _ => {
                // local recursive function, callable from the core
                let y = self.fresh("y");
                let body = self.rec_fun_body(&y, env, rb, usize::MAX, Some(&a));
                rb.push((a.clone(), RK::Fun));
                format!("let rec {a} ({y} : int) = {body} in")
            }
        }
    }

    /// Guarded body of a recursive `int -> int` function with parameter `x`.
    fn rec_fun_body(&mut self, x: &str, env: &Env, rb: &[(String, RK)], _me: usize, selfname: Option<&str>) -> String {
        let ex = env.with_var(x, Int);
        let base = self.pure(&Int, &ex, 2);
        let mut step = self.pure(&Int, &ex, 2);
        for _ in 0..self.r.range(1, 2) {
            let c = if let (Some(s), true) = (selfname, self.r.chance(0.3)) {
                format!("{s} ({x} - 1)")
            } else {
                match self.rec_call(x, rb) {
                    Some(c) => c,
                    None => continue,
                }
            };
            let op = *self.r.pick(&["+", "lxor", "-", "*"]);
            step = format!("({step}) {op} ({c})");
        }
        format!("if {x} <= 0 || {x} > 10 then {base} else {step}")
    }

    fn rec_call(&mut self, x: &str, rb: &[(String, RK)]) -> Option<String> {
        let cands: Vec<&(String, RK)> = rb.iter().filter(|(_, k)| *k != RK::Lz).collect();
        if cands.is_empty() {
            return None;
        }
        let (n, k) = self.r.pick(&cands);
        Some(match k {
            RK::Fun => format!("{n} ({x} - 1)"),
            RK::Knot => {
                if self.r.chance(0.5) {
                    format!("{n}.H.kg ({x} - 1)")
                } else {
                    format!("H.ksum 3 {n}.H.kn")
                }
            }
            RK::Kv => format!("H.kvsum 3 {n}"),
            RK::Lst => format!("List.length (H.take 4 {n})"),
            RK::FunArr => format!("(H.get {n} {x}) ({x} - 1)"),
            RK::FltArr => format!("H.f2i (H.get {n} {x})"),
            RK::IntArr => format!("H.get {n} {x}"),
            RK::FRec => format!("H.f2i {n}.H.fy"),
            RK::Konst => n.clone(),
            RK::Lz => unreachable!(),
        })
    }

    /// An inline function value (for record/variant/array fields).
    fn rec_inline_fun(&mut self, env: &Env, rb: &[(String, RK)]) -> String {
        let x = self.fresh("x");
        let body = self.rec_fun_body(&x, env, rb, usize::MAX, None);
        format!("(fun ({x} : int) -> {body})")
    }

    fn rec_pick(&mut self, rb: &[(String, RK)], k: RK) -> Option<String> {
        let c: Vec<&String> = rb.iter().filter(|(_, kk)| *kk == k).map(|(n, _)| n).collect();
        if c.is_empty() {
            None
        } else {
            Some(self.r.pick(&c).to_string())
        }
    }

    fn rec_fun_field(&mut self, env: &Env, rb: &[(String, RK)]) -> String {
        match self.rec_pick(rb, RK::Fun) {
            Some(f) if self.r.chance(0.6) => f,
            _ if self.r.chance(0.2) => "H.knot0.H.kg".into(),
            _ => self.rec_inline_fun(env, rb),
        }
    }

    fn rec_core(&mut self, env: &Env, rb: &[(String, RK)], me: usize, k: RK) -> String {
        match k {
            RK::Fun => {
                let x = self.fresh("x");
                match self.r.below(4) {
                    0 => {
                        let h = self.fresh("h");
                        let body = self.rec_fun_body(&x, env, rb, me, None);
                        format!("let {h} ({x} : int) = {body} in {h}")
                    }
                    1 => {
                        let h = self.fresh("h");
                        let body = self.rec_fun_body(&x, env, rb, me, Some(&h));
                        format!("let rec {h} ({x} : int) = {body} in {h}")
                    }
                    2 => {
                        let c = self.pure(&Int, env, 1);
                        let body = self.rec_fun_body(&x, env, rb, me, None);
                        format!("function 0 -> {c} | {x} -> {body}")
                    }
                    _ => {
                        let body = self.rec_fun_body(&x, env, rb, me, None);
                        format!("fun ({x} : int) -> {body}")
                    }
                }
            }
            RK::Knot => {
                let kn = if self.r.chance(0.85) { self.rec_pick(rb, RK::Knot).unwrap() } else { "H.knot0".into() };
                let v = self.pure(&Int, env, 2);
                if self.r.chance(0.25) {
                    format!("{{ H.knot0 with H.kn = {kn}; kv = {v} }}")
                } else {
                    let kr = match self.rec_pick(rb, RK::FRec) {
                        Some(r) if self.r.chance(0.7) => r,
                        _ => self.pure(&FR, env, 1),
                    };
                    let kg = self.rec_fun_field(env, rb);
                    format!("{{ H.kv = {v}; kr = {kr}; kn = {kn}; kg = {kg} }}")
                }
            }
            RK::Kv => {
                let kn = if self.r.chance(0.85) { self.rec_pick(rb, RK::Kv).unwrap() } else { "H.K0".into() };
                let v = self.pure(&Int, env, 2);
                if self.r.chance(0.5) {
                    format!("H.K1 ({v}, {kn})")
                } else {
                    let f = self.rec_fun_field(env, rb);
                    format!("H.K2 {{ k2 = {v}; k2n = {kn}; k2f = {f} }}")
                }
            }
            RK::Lst => {
                let tl = if self.r.chance(0.85) { self.rec_pick(rb, RK::Lst).unwrap() } else { "[]".into() };
                let els: Vec<String> = (0..self.r.range(1, 3)).map(|_| self.pure(&Int, env, 2)).collect();
                format!("{} :: {tl}", els.join(" :: "))
            }
            RK::FRec => {
                let (a, b) = (self.pure(&Float, env, 2), self.pure(&Float, env, 2));
                format!("{{ H.fx = {a}; fy = {b} }}")
            }
            RK::Lz => {
                // forces only lazies defined before `me` (no cycles)
                let mut body = self.pure(&Int, env, 2);
                for _ in 0..self.r.range(0, 2) {
                    let earlier: Vec<String> = rb.iter().take(me).filter(|(_, k)| *k == RK::Lz).map(|(n, _)| n.clone()).collect();
                    let c = if !earlier.is_empty() && self.r.chance(0.4) {
                        format!("Lazy.force {}", self.r.pick(&earlier))
                    } else {
                        let c = self.r.range(0, 12).to_string();
                        match self.rec_call(&c, rb) {
                            Some(e) => e,
                            None => continue,
                        }
                    };
                    body = format!("({body}) + ({c})");
                }
                // an application, never a bare constant/identifier (those are
                // not delayed: Typeopt.classify_lazy_argument)
                body = format!("({body}) + 1");
                if self.r.chance(0.4) {
                    let t = self.fresh("t");
                    format!("let {t} = lazy ({body}) in {t}")
                } else {
                    format!("lazy ({body})")
                }
            }
            RK::FunArr => {
                let els: Vec<String> = (0..self.r.range(1, 3)).map(|_| self.rec_fun_field(env, rb)).collect();
                format!("[| {} |]", els.join("; "))
            }
            RK::FltArr => {
                let els: Vec<String> = (0..self.r.range(1, 3)).map(|_| self.pure(&Float, env, 2)).collect();
                format!("[| {} |]", els.join("; "))
            }
            RK::IntArr => {
                let els: Vec<String> = (0..self.r.range(1, 3)).map(|_| self.pure(&Int, env, 2)).collect();
                format!("[| {} |]", els.join("; "))
            }
            RK::Konst => self.pure(&Int, env, 2),
        }
    }

    fn local_loop(&mut self, s: &mut String, env: &mut Env, depth: u32) {
        let name = self.fresh("lp");
        let n = self.fresh("n");
        let acc = self.fresh("acc");
        let t = if self.r.chance(0.5) { self.scalar() } else { self.data_ty(1) };
        let env2 = env.with_var(&n, Int).with_var(&acc, t.clone());
        let b = self.unit_block(&env2, depth - 1, 3);
        let step = self.pure(&t, &env2, 3);
        let init = self.pure(&t, env, 2);
        let cnt = self.pure(&Int, env, 2);
        let x = self.fresh("v");
        let _ = writeln!(
            s,
            "let rec {name} {n} ({acc} : {}) = if {n} <= 0 then {acc} else begin {b}; {name} ({n} - 1) {step} end in\nlet {x} = {name} ({cnt} land 15) {init} in",
            t.ml()
        );
        env.vars.push(Var { name: x, ty: t });
    }

    /// A functor applied twice, first-class modules and a class hierarchy in A
    /// (lambda/translmod.ml, translclass.ml); their functions join A's.
    fn modules(&mut self, a: &mut String, env: &mut Env) {
        self.fuel = 40;
        let fm = self.fresh("F");
        let g = self.fresh("fg");
        let h = self.fresh("fh");
        let p = self.fresh("a");
        let mut fenv = env.clone();
        fenv.vars.push(Var { name: "X.k".into(), ty: Int });
        fenv.vars.push(Var { name: "X.f".into(), ty: Fun(bx(Int), bx(Int)) });
        fenv.vars.push(Var { name: p.clone(), ty: Int });
        let gattr = *self.r.pick(&["", "", "[@inline never]", "[@inline]"]);
        let gb = self.block(&Int, &fenv, 1, 4);
        fenv.funcs.push(Func { name: g.clone(), params: vec![Int], ret: Int });
        self.fuel = 30;
        let hb = self.block(&Int, &fenv, 1, 3);
        let _ = writeln!(a, "module {fm} (X : H.S) = struct\n  let{gattr} {g} ({p} : int) : int = {gb}\n  let {h} ({p} : int) : int = {hb}\nend");
        let mut insts = vec![];
        for _ in 0..2 {
            let m = self.fresh("M");
            let x = self.fresh("x");
            self.fuel = 20;
            let k = self.int_lit();
            let fb = self.pure(&Int, &Env::default().with_var(&x, Int), 2);
            if self.r.chance(0.5) {
                let _ = writeln!(a, "module {m} = {fm} (struct let k = {k} let f {x} = {fb} end)");
            } else {
                let _ = writeln!(a, "module {m}X = struct let k = {k} let f {x} = {fb} end\nmodule {m} = {fm} ({m}X)");
            }
            for fname in [&g, &h] {
                env.funcs.push(Func { name: format!("{m}.{fname}"), params: vec![Int], ret: Int });
            }
            env.vars.push(Var { name: format!("{m}.{g}"), ty: Fun(bx(Int), bx(Int)) });
            insts.push(m);
        }
        // first-class modules: unpacked at a dynamically chosen instance
        let fc = self.fresh("fc");
        let q = self.fresh("a");
        let _ = writeln!(
            a,
            "let {fc} ({q} : int) : int = let module Y = (val (if {q} land 1 = 0 then (module struct let k = {q} let f = {i0}.{g} end : H.S) else (module struct let k = 1 let f = {i1}.{h} end : H.S))) in Y.f (Y.k + 1)",
            i0 = insts[0],
            i1 = insts[1]
        );
        env.funcs.push(Func { name: fc, params: vec![Int], ret: Int });
        // classes with inheritance, overriding and super calls
        if self.r.chance(0.7) {
            let (c, d, fld, n) = (self.fresh("cl"), self.fresh("cd"), self.fresh("fl"), self.fresh("n"));
            self.fuel = 20;
            let e0 = env.with_var(&n, Int).with_var(&fld, Int);
            let step = self.pure(&Int, &e0, 2);
            let get = self.pure(&Int, &env.with_var(&fld, Int), 2);
            let get2 = self.pure(&Int, &env.with_var("s", Int).with_var(&fld, Int), 2);
            let _ = writeln!(
                a,
                "class {c} (init : int) = object (self)\n  val mutable {fld} = init\n  method get = {get}\n  method step ({n} : int) = {fld} <- {step}; self#get\nend\nclass {d} init = object\n  inherit {c} init as super\n  method! get = let s = super#get in {get2}\nend"
            );
            let mk = self.fresh("mk");
            let (x, y) = (self.fresh("a"), self.fresh("a"));
            let _ = writeln!(
                a,
                "let {mk} ({x} : int) ({y} : int) : int = let o = if {x} land 1 = 0 then new {c} {x} else (new {d} {x} :> {c}) in let _ = o#step {y} in o#step ({y} + 1)"
            );
            env.funcs.push(Func { name: mk, params: vec![Int, Int], ret: Int });
        }
    }

    // ---------------------------------------------------------------- program
    pub fn program(&mut self) -> Program {
        let mut a = String::new();
        let mut env = Env::default();
        // module-level state of A
        let globals: Vec<(Ty, String)> = vec![
            (Ref(bx(Int)), "ref 0".into()),
            (Ref(bx(Float)), "ref 0.5".into()),
            (Ref(bx(I64)), "ref 0L".into()),
            (Arr(bx(Int)), "[| 1; 2; 3; 4 |]".into()),
            (Arr(bx(Float)), "[| 1.5; -2.; 0. |]".into()),
            (R, "{ H.ri = 1; rf = 2.; rl = 3L }".into()),
            (FR, "{ H.fx = 1.; fy = 2. }".into()),
            (Atom(bx(Int)), "Atomic.make 0".into()),
            (Atom(bx(Bool)), "Atomic.make false".into()),
            (AR, "{ H.ai = 0; ab = false; ap = 0 }".into()),
            (AA, "Atomic.Array.make 4 0".into()),
        ];
        for (i, (t, init)) in globals.iter().enumerate() {
            if self.r.chance(0.7) {
                let _ = writeln!(a, "let g{i} : {} = {init}", t.ml());
                env.vars.push(Var { name: format!("g{i}"), ty: t.clone() });
            }
        }
        // A's functions
        let nf = self.r.range(2, 6);
        let modpos = if self.r.chance(0.6) { Some(self.r.below(nf)) } else { None };
        let recpos = if self.r.chance(0.35) { Some(self.r.below(nf)) } else { None };
        for fi in 0..nf {
            if modpos == Some(fi) {
                self.modules(&mut a, &mut env);
            }
            if recpos == Some(fi) {
                // top-level recursive definitions (translmod + value_rec_compiler,
                // constants lifted by closure/flambda)
                self.fuel = 40;
                let (d, obs) = self.rec_group(&env);
                let _ = writeln!(a, "{d}\n");
                for (name, e, ty) in obs {
                    if name != e {
                        let _ = writeln!(a, "let {name} = {e}");
                    }
                    env.vars.push(Var { name, ty });
                }
                a.push('\n');
            }
            self.fuel = 60;
            let name = self.fresh("fa");
            let np = self.r.range(1, 3);
            let mut params = vec![];
            let mut env2 = env.clone();
            let mut ps = String::new();
            for _ in 0..np {
                let t = if self.r.chance(0.6) { self.scalar() } else { self.ty(1) };
                let p = self.fresh("a");
                let _ = write!(ps, " ({p} : {})", t.ml());
                env2.vars.push(Var { name: p, ty: t.clone() });
                params.push(t);
            }
            let ret = if self.r.chance(0.6) { self.scalar() } else { self.data_ty(1) };
            let attr = *self.r.pick(&["", "", "", "[@inline never]", "[@inline]"]);
            let body = self.block(&ret, &env2, 2, 6);
            let _ = writeln!(a, "let{attr} {name}{ps} : {} =\n{body}\n", ret.ml());
            if np == 1 && matches!((&params[0], &ret), (Int, Int) | (Float, Float) | (Int, Float) | (I64, I64)) {
                env.vars.push(Var { name: name.clone(), ty: Fun(bx(params[0].clone()), bx(ret.clone())) });
            }
            env.funcs.push(Func { name, params, ret });
        }
        // Main's view of A
        let mut menv = Env::default();
        for v in &env.vars {
            menv.vars.push(Var { name: format!("A.{}", v.name), ty: v.ty.clone() });
        }
        for f in &env.funcs {
            menv.funcs.push(Func { name: format!("A.{}", f.name), params: f.params.clone(), ret: f.ret.clone() });
        }
        let nt = self.r.range(6, 14);
        let mut tests = vec![];
        for i in 0..nt {
            self.fuel = 120;
            let mut s = format!("let t{i} () =\n");
            let mut tenv = menv.clone();
            let mut body = String::new();
            let k = self.r.range(3, 10);
            for _ in 0..k {
                self.stmt(&mut body, &mut tenv, 3);
            }
            // observe everything bound at the end
            for v in tenv.vars.iter().filter(|v| !v.name.starts_with("A.fa")) {
                let _ = writeln!(body, "{} {};", v.ty.printer(), v.name);
            }
            s.push_str(&body);
            s.push_str("()\n");
            tests.push((i, s));
        }
        Program { a, tests }
    }
}

fn bmod(t: &Ty) -> &'static str {
    match t {
        I32 => "Int32",
        I64 => "Int64",
        _ => "Nativeint",
    }
}
fn suffix(t: &Ty) -> &'static str {
    match t {
        I32 => "l",
        I64 => "L",
        _ => "n",
    }
}

#[allow(dead_code)]
fn unused(t: &Ty) -> bool {
    t.has_fun()
}
