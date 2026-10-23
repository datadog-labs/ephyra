// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! Typed random OCaml (OxCaml) program generator.
//!
//! Programs are valid by construction for the rules in dossier §3:
//! - expressions are pure and never raise (division/shift/conversion guarded),
//!   so the unspecified evaluation order of arguments, tuples, records and
//!   constructor fields (R1-R3) cannot be observed;
//! - effects (raise, global counter, mutation) only happen in statements,
//!   which are sequenced with `let ... in` / `;`;
//! - shift counts are masked (R5), float->int conversions clamp (R6),
//!   floats are observed through `hf`, which canonicalises NaN (R9);
//! - unboxed values are never passed to polymorphic compare/hash (R10);
//! - local (stack) values never escape: values derived from them are tracked
//!   ("tainted") and only flow into `local_` parameters, matches, observers
//!   and immediates, which cross modes.
//!
//! Every program is a list of `test_<i>` functions, each with one focus
//! feature; the program prints one line per test, so a difference localises
//! to the test and its focus (which becomes the signature's area).

use ephyra::Source;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ty {
    Int,
    Bool,
    Float,
    I64,
    I32,
    Fu,   // float#
    I64u, // int64#
    V0,   // A0 | B0 of int | C0 of float * int | D0 of int * v0
    V1,   // constant constructors only (immediate)
    V2,   // P2 of int64 | Q2 of float# * int | R2 | S2 of int32 * float
    R0,   // mixed record { mutable f0:int; f1:float; f2:float#; mutable f3:int64# }
    R1,   // { g0:int; mutable g1:int; g2:v1 }
    R2,   // flat float record { h0:float; mutable h1:float }
    Pair, // int * int
    Str,
    Fun1, // int -> int
    IArr, // int array
    FArr, // float array
    IRef, // int ref
}
use Ty::*;

impl Ty {
    /// Values of these types cross modes: never local.
    fn immediate(self) -> bool {
        matches!(self, Int | Bool | Fu | I64u | V1)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Focus {
    Int,
    Float,
    Boxint,
    Unboxed,
    Variant,
    Record,
    Closure,
    Exn,
    Local,
    Loop,
    Str,
    Array,
}

pub const FOCI: [Focus; 12] = [
    Focus::Int,
    Focus::Float,
    Focus::Boxint,
    Focus::Unboxed,
    Focus::Variant,
    Focus::Record,
    Focus::Closure,
    Focus::Exn,
    Focus::Local,
    Focus::Loop,
    Focus::Str,
    Focus::Array,
];

impl Focus {
    pub fn name(self) -> &'static str {
        match self {
            Focus::Int => "int_arith",
            Focus::Float => "float",
            Focus::Boxint => "boxed_int",
            Focus::Unboxed => "unboxed",
            Focus::Variant => "variant_match",
            Focus::Record => "record",
            Focus::Closure => "closure",
            Focus::Exn => "exn",
            Focus::Local => "local",
            Focus::Loop => "loop_mutable",
            Focus::Str => "string",
            Focus::Array => "array",
        }
    }
    /// Types this focus prefers for intermediate values.
    fn types(self) -> &'static [Ty] {
        match self {
            Focus::Int => &[Int, Int, Bool, Int],
            Focus::Float => &[Float, Float, Int, R2, FArr],
            Focus::Boxint => &[I64, I32, I64, Int],
            Focus::Unboxed => &[Fu, I64u, R0, Fu, I64u, V2],
            Focus::Variant => &[V0, V1, V2, V0, Int],
            Focus::Record => &[R0, R1, R2, Int],
            Focus::Closure => &[Fun1, Fun1, Int, Pair],
            Focus::Exn => &[Int, V0, IRef],
            Focus::Local => &[Pair, V0, R1, Fun1, Float],
            Focus::Loop => &[Int, IRef, Float, IArr],
            Focus::Str => &[Str, Str, Int],
            Focus::Array => &[IArr, FArr, Int, Float],
        }
    }
}

#[derive(Clone, Debug)]
struct Var {
    name: String,
    ty: Ty,
    local: bool,
    /// `let mutable` (cannot be captured by closures; assignable with `<-`)
    mutable: bool,
}

#[derive(Clone, Debug)]
struct Func {
    name: String,
    params: Vec<(Ty, bool)>, // (type, declared local_)
    ret: Ty,
    pure_: bool,
    /// recursive function: first param is fuel
    rec_: bool,
}

#[derive(Clone, Default)]
struct Env {
    vars: Vec<Var>,
}

impl Env {
    fn push(&mut self, v: Var) {
        self.vars.push(v);
    }
}

pub struct Program {
    pub text: String,
    /// focus of test i (test names are t<i>)
    pub foci: Vec<Focus>,
}

pub struct Gen<'a> {
    s: &'a mut Source,
    fresh: usize,
    funcs: Vec<Func>,
    focus: Focus,
    /// statements may have effects (raise / counter)
    effects: bool,
    /// statements may touch global state (counter, effectful calls)
    global_fx: bool,
    /// budget of nodes left for the current item, keeps programs bounded
    fuel: i32,
}

const INT_LITS: &[&str] = &[
    "0", "1", "(-1)", "2", "3", "5", "7", "8", "10", "16", "31", "32", "63", "64", "100", "255",
    "256", "(-128)", "1000003", "max_int", "min_int", "0x7fffffff", "(-0x80000000)", "0x100000000",
    "(1 lsl 62)", "(-7)", "4611686018427387903", "65535",
];
const FLOAT_LITS: &[&str] = &[
    "0.0", "(-0.0)", "1.0", "(-1.5)", "0.1", "1e308", "(-1e-308)", "5e-324", "nan", "infinity",
    "neg_infinity", "3.0", "1e16", "2.5", "max_float", "min_float", "epsilon_float", "(-2.0)",
    "0.5", "4503599627370496.5", "9007199254740993.0", "(-3.75)", "1e-10", "123456.789",
];
const I64_LITS: &[&str] = &[
    "0L", "1L", "(-1L)", "2L", "Int64.max_int", "Int64.min_int", "0x100000000L", "255L",
    "(-128L)", "0x7fffffffL", "(-0x80000000L)", "1000003L", "65536L",
];
const I32_LITS: &[&str] = &[
    "0l", "1l", "(-1l)", "2l", "Int32.max_int", "Int32.min_int", "255l", "(-128l)", "65535l",
    "0x7fffl", "1000003l",
];
const FU_LITS: &[&str] = &[
    "#0.0", "#1.0", "#2.5", "#0.1", "#1e308", "#5e-324", "(FU.of_float (-0.0))",
    "(FU.of_float nan)", "(FU.of_float infinity)", "(FU.of_float (-1.5))", "#3.0", "#1e16",
];
const I64U_LITS: &[&str] = &[
    "#0L", "#1L", "#2L", "#255L", "#0x7fffffffL", "#1000003L", "(I64U.of_int64 Int64.min_int)",
    "(I64U.of_int64 Int64.max_int)", "(I64U.of_int (-1))", "#65536L",
];
const STR_LITS: &[&str] = &["\"\"", "\"a\"", "\"hello\"", "\"\\000\\255\"", "\"abcdefghij\"", "\"xy\""];

pub const PRELUDE: &str = r#"module FU = Stdlib_upstream_compatible.Float_u
module I64U = Stdlib_upstream_compatible.Int64_u
module I32U = Stdlib_upstream_compatible.Int32_u
module NU = Stdlib_upstream_compatible.Nativeint_u
module I8 = Stdlib_stable.Int8
module I16 = Stdlib_stable.Int16
module I8U = Stdlib_stable.Int8_u
module I16U = Stdlib_stable.Int16_u
module IU = Stdlib_stable.Int_u
module F32 = Stdlib_stable.Float32
module F32U = Stdlib_stable.Float32_u
exception E0
exception E1 of int
exception E2 of string
type v0 = A0 | B0 of int | C0 of float * int | D0 of int * v0
type v1 = X1 | Y1 | Z1 | W1 | U1
type v2 = P2 of int64 | Q2 of float# * int | R2 | S2 of int32 * float
type r0 = { mutable f0 : int; f1 : float; f2 : float#; mutable f3 : int64_u }
type r1 = { g0 : int; mutable g1 : int; g2 : v1 }
type r2 = { h0 : float; mutable h1 : float }
let cnt = ref 0
let mix (h : int) (x : int) = h * 1000003 + x
let hf (local_ x : float) =
  let y = FU.to_float (FU.of_float x) in
  if Float.is_nan y then 0x7ff8 else Int64.to_int (Int64.bits_of_float y)
let hfu (x : float#) = hf (FU.to_float x)
let fi (local_ x : float) =
  let y = FU.to_float (FU.of_float x) in
  if Float.is_nan y then 0 else if y > 1e15 then 1 else if y < -1e15 then 2 else int_of_float y
let h_i64 (local_ x : int64) = I64U.to_int (I64U.of_int64 x)
let h_i32 (local_ x : int32) = Int32.to_int x
let h_v1 (x : v1) = match x with X1 -> 1 | Y1 -> 2 | Z1 -> 3 | W1 -> 4 | U1 -> 5
let rec h_v0 (local_ x : v0) =
  match x with
  | A0 -> 11 | B0 n -> mix 12 n | C0 (f, n) -> mix (mix 13 (hf f)) n
  | D0 (n, v) -> mix (mix 14 n) (h_v0 v)
let h_v2 (local_ x : v2) =
  match x with
  | P2 n -> mix 21 (h_i64 n) | Q2 (f, n) -> mix (mix 22 (hfu f)) n | R2 -> 23
  | S2 (a, f) -> mix (mix 24 (h_i32 a)) (hf f)
let h_r0 (local_ r : r0) = mix (mix (mix r.f0 (hf r.f1)) (hfu r.f2)) (I64U.to_int r.f3)
let h_r1 (local_ r : r1) = mix (mix r.g0 r.g1) (h_v1 r.g2)
let h_r2 (local_ r : r2) = mix (hf r.h0) (hf r.h1)
let h_pair (local_ p : int * int) = let (a, b) = p in mix a b
let h_str (s : string) = Hashtbl.hash s
let h_fun (local_ g : int -> int) = mix (g 0) (mix (g 7) (g (-3)))
let h_iarr (a : int array) = Array.fold_left mix 5 a
let h_farr (a : float array) = Array.fold_left (fun h x -> mix h (hf x)) 6 a
let mkpair_l (a : int) (b : int) = exclave_ (a, b)
let mkv0_l (n : int) (f : float) = exclave_ (D0 (n, C0 (f, n)))
let pr name v = print_string name; print_char ' '; print_int v; print_newline ()
"#;

fn observe(ty: Ty, e: &str) -> String {
    match ty {
        Int => e.to_string(),
        Bool => format!("(Bool.to_int {e})"),
        Float => format!("(hf {e})"),
        I64 => format!("(h_i64 {e})"),
        I32 => format!("(h_i32 {e})"),
        Fu => format!("(hfu {e})"),
        I64u => format!("(I64U.to_int {e})"),
        V0 => format!("(h_v0 {e})"),
        V1 => format!("(h_v1 {e})"),
        V2 => format!("(h_v2 {e})"),
        R0 => format!("(h_r0 {e})"),
        R1 => format!("(h_r1 {e})"),
        R2 => format!("(h_r2 {e})"),
        Pair => format!("(h_pair {e})"),
        Str => format!("(h_str {e})"),
        Fun1 => format!("(h_fun {e})"),
        IArr => format!("(h_iarr {e})"),
        FArr => format!("(h_farr {e})"),
        IRef => format!("!{e}"),
    }
}

/// Generated expression: code plus whether it may be a local value.
type E = (String, bool);

impl<'a> Gen<'a> {
    pub fn new(s: &'a mut Source) -> Self {
        Gen { s, fresh: 0, funcs: Vec::new(), focus: Focus::Int, effects: false, global_fx: false, fuel: 0 }
    }

    fn name(&mut self, p: &str) -> String {
        self.fresh += 1;
        format!("{p}{}", self.fresh)
    }

    fn pick<'b>(&mut self, xs: &'b [&'b str]) -> &'b str {
        xs[self.s.index(xs.len())]
    }

    fn chance(&mut self, p: f64) -> bool {
        self.s.bool_with(p)
    }

    /// a type biased toward the current focus
    fn some_ty(&mut self) -> Ty {
        if self.chance(0.7) {
            let ts = self.focus.types();
            ts[self.s.index(ts.len())]
        } else {
            let all = [Int, Bool, Float, I64, I32, Fu, I64u, V0, V1, V2, R0, R1, R2, Pair, Str, Fun1];
            all[self.s.index(all.len())]
        }
    }

    fn vars_of<'e>(&self, env: &'e Env, ty: Ty, allow_local: bool) -> Vec<&'e Var> {
        env.vars
            .iter()
            .filter(|v| v.ty == ty && (allow_local || !v.local || ty.immediate()))
            .collect()
    }

    fn var(&mut self, env: &Env, ty: Ty, allow_local: bool) -> Option<E> {
        let vs = self.vars_of(env, ty, allow_local);
        if vs.is_empty() {
            return None;
        }
        let v = vs[self.s.index(vs.len())];
        Some((v.name.clone(), v.local && !ty.immediate()))
    }

    fn lit(&mut self, ty: Ty, env: &Env, allow_local: bool) -> E {
        let s = match ty {
            Int => self.pick(INT_LITS).to_string(),
            Bool => self.pick(&["true", "false"]).to_string(),
            Float => self.pick(FLOAT_LITS).to_string(),
            I64 => self.pick(I64_LITS).to_string(),
            I32 => self.pick(I32_LITS).to_string(),
            Fu => self.pick(FU_LITS).to_string(),
            I64u => self.pick(I64U_LITS).to_string(),
            V0 => self.pick(&["A0", "(B0 3)", "(C0 (0.5, 2))", "(D0 (1, A0))", "(B0 (-1))"]).to_string(),
            V1 => self.pick(&["X1", "Y1", "Z1", "W1", "U1"]).to_string(),
            V2 => self.pick(&["R2", "(P2 5L)", "(Q2 (#1.5, 4))", "(S2 (7l, -0.0))"]).to_string(),
            R0 => "{ f0 = 1; f1 = 2.5; f2 = #0.5; f3 = #3L }".to_string(),
            R1 => "{ g0 = 3; g1 = 4; g2 = Y1 }".to_string(),
            R2 => "{ h0 = 1.25; h1 = -2.0 }".to_string(),
            Pair => self.pick(&["(1, 2)", "(0, -1)", "(max_int, min_int)"]).to_string(),
            Str => self.pick(STR_LITS).to_string(),
            Fun1 => self.pick(&["(fun y -> y + 1)", "(fun y -> y * 3)", "(fun y -> y lxor 5)"]).to_string(),
            IArr => "[| 1; 2; 3 |]".to_string(),
            FArr => "[| 1.5; -0.0; nan |]".to_string(),
            IRef => {
                // refs only come from statements; fall back to a fresh one
                let _ = (env, allow_local);
                "(ref 0)".to_string()
            }
        };
        (s, false)
    }

    /// leaf: a variable (preferred) or a literal
    fn leaf(&mut self, env: &Env, ty: Ty, allow_local: bool) -> E {
        if self.chance(0.75) {
            if let Some(v) = self.var(env, ty, allow_local) {
                return v;
            }
        }
        self.lit(ty, env, allow_local)
    }

    pub fn expr(&mut self, env: &Env, ty: Ty, d: u32, al: bool) -> E {
        self.fuel -= 1;
        if d == 0 || self.fuel <= 0 || self.chance(0.2) {
            return self.leaf(env, ty, al);
        }
        let d1 = d - 1;
        // generic productions available for every type
        let r = self.s.int(0u32..=99);
        if r < 6 {
            let c = self.expr(env, Bool, d1, al);
            let a = self.expr(env, ty, d1, al);
            let b = self.expr(env, ty, d1, al);
            return (format!("(if {} then {} else {})", c.0, a.0, b.0), a.1 || b.1);
        }
        if r < 14 {
            // Known -Oclassic crash ("Illegal move between registers of differing
            // types") on a float# match result in argument position
            // (agents/harness-002/cand-oclassic-illegal-move): mostly avoided so it
            // does not dominate the failure rate; still generated 1 time in 20.
            if !(matches!(ty, Fu | I64u) && self.chance(0.95)) {
                return self.match_expr(env, ty, d1, al);
            }
        }
        if r < 18 {
            // expression-level let
            let t = self.some_ty();
            let x = self.name("x");
            let e1 = self.expr(env, t, d1, al);
            let mut env2 = env.clone();
            env2.push(Var { name: x.clone(), ty: t, local: e1.1, mutable: false });
            let e2 = self.expr(&env2, ty, d1, al);
            return (format!("(let {x} = {} in {})", e1.0, e2.0), e2.1);
        }
        if r < 24 {
            if let Some(e) = self.call(env, ty, d1, al) {
                return e;
            }
        }
        self.typed(env, ty, d1, al)
    }

    fn e(&mut self, env: &Env, ty: Ty, d: u32, al: bool) -> String {
        self.expr(env, ty, d, al).0
    }

    /// type-specific productions
    fn typed(&mut self, env: &Env, ty: Ty, d: u32, al: bool) -> E {
        match ty {
            Int => (self.int_expr(env, d, al), false),
            Bool => (self.bool_expr(env, d, al), false),
            Float => self.float_expr(env, d, al),
            I64 => self.i64_expr(env, d, al),
            I32 => self.i32_expr(env, d, al),
            Fu => (self.fu_expr(env, d, al), false),
            I64u => (self.i64u_expr(env, d, al), false),
            V0 => {
                let k = self.s.int(0u32..=3);
                let s = match k {
                    0 => "A0".to_string(),
                    1 => format!("(B0 {})", self.e(env, Int, d, al)),
                    2 => {
                        let f = self.expr(env, Float, d, false).0;
                        format!("(C0 ({f}, {}))", self.e(env, Int, d, al))
                    }
                    _ => {
                        let i = self.e(env, Int, d, al);
                        let v = self.expr(env, V0, d, false).0;
                        format!("(D0 ({i}, {v}))")
                    }
                };
                (s, false)
            }
            V1 => {
                let i = self.e(env, Int, d, al);
                let cs = ["X1", "Y1", "Z1", "W1", "U1"];
                let a = cs[self.s.index(5)];
                let b = cs[self.s.index(5)];
                let c = cs[self.s.index(5)];
                (format!("(match {i} land 3 with 0 -> {a} | 1 -> {b} | 2 -> {c} | _ -> U1)"), false)
            }
            V2 => {
                let k = self.s.int(0u32..=3);
                let s = match k {
                    0 => "R2".to_string(),
                    1 => format!("(P2 {})", self.expr(env, I64, d, false).0),
                    2 => {
                        let f = self.e(env, Fu, d, al);
                        format!("(Q2 ({f}, {}))", self.e(env, Int, d, al))
                    }
                    _ => {
                        let a = self.expr(env, I32, d, false).0;
                        format!("(S2 ({a}, {}))", self.expr(env, Float, d, false).0)
                    }
                };
                (s, false)
            }
            R0 => {
                if self.chance(0.3) {
                    if let Some((r, _)) = self.var(env, R0, false) {
                        return (format!("{{ {r} with f0 = {} }}", self.e(env, Int, d, al)), false);
                    }
                }
                let f0 = self.e(env, Int, d, al);
                let f1 = self.expr(env, Float, d, false).0;
                let f2 = self.e(env, Fu, d, al);
                let f3 = self.e(env, I64u, d, al);
                (format!("{{ f0 = {f0}; f1 = {f1}; f2 = {f2}; f3 = {f3} }}"), false)
            }
            R1 => {
                if self.chance(0.3) {
                    if let Some((r, _)) = self.var(env, R1, false) {
                        return (format!("{{ {r} with g1 = {} }}", self.e(env, Int, d, al)), false);
                    }
                }
                let a = self.e(env, Int, d, al);
                let b = self.e(env, Int, d, al);
                let c = self.e(env, V1, d, al);
                (format!("{{ g0 = {a}; g1 = {b}; g2 = {c} }}"), false)
            }
            R2 => {
                let a = self.expr(env, Float, d, false).0;
                let b = self.expr(env, Float, d, false).0;
                (format!("{{ h0 = {a}; h1 = {b} }}"), false)
            }
            Pair => {
                if self.chance(0.3) {
                    if let Some((p, t)) = self.var(env, Pair, al) {
                        let x = self.name("p");
                        let y = self.name("p");
                        // swap: components are ints, so the result is fresh
                        let _ = t;
                        return (format!("(let ({x}, {y}) = {p} in ({y}, {x}))"), false);
                    }
                }
                let a = self.e(env, Int, d, al);
                let b = self.e(env, Int, d, al);
                (format!("({a}, {b})"), false)
            }
            Str => {
                let k = self.s.int(0u32..=4);
                let s = match k {
                    0 => format!("({} ^ {})", self.e(env, Str, d, al), self.e(env, Str, d, al)),
                    1 => format!("(string_of_int {})", self.e(env, Int, d, al)),
                    2 => format!("(String.make ({} land 7) 'z')", self.e(env, Int, d, al)),
                    3 => {
                        let s = self.e(env, Str, d, al);
                        let x = self.name("s");
                        format!("(let {x} = {s} in String.sub {x} 0 (String.length {x} / 2))")
                    }
                    _ => {
                        let s = self.e(env, Str, d, al);
                        format!("(String.uppercase_ascii {s})")
                    }
                };
                (s, false)
            }
            Fun1 => {
                let k = self.s.int(0u32..=3);
                match k {
                    0 | 1 => {
                        let y = self.name("y");
                        let mut env2 = env.clone();
                        env2.vars.retain(|v| !v.mutable);
                        env2.push(Var { name: y.clone(), ty: Int, local: false, mutable: false });
                        let b = self.expr(&env2, Int, d, al);
                        let uses_local = env2.vars.iter().any(|v| v.local && !v.ty.immediate() && b.0.contains(&v.name));
                        (format!("(fun {y} -> mix 1 ({}))", b.0), uses_local)
                    }
                    2 => {
                        // partial application of a two-int-arg function
                        let fs: Vec<Func> = self
                            .funcs
                            .iter()
                            .filter(|f| f.pure_ && f.ret == Int && f.params.len() >= 2 && f.params.last().map(|p| p.0) == Some(Int))
                            .cloned()
                            .collect();
                        if fs.is_empty() {
                            return self.leaf(env, Fun1, al);
                        }
                        let f = &fs[self.s.index(fs.len())];
                        let mut args = Vec::new();
                        for p in &f.params[..f.params.len() - 1] {
                            args.push(self.arg(env, p.0, false, d, f.rec_ && args.is_empty()));
                        }
                        (format!("({} {})", f.name, args.join(" ")), false)
                    }
                    _ => {
                        let g = self.expr(env, Fun1, d, al);
                        let h = self.expr(env, Fun1, d, al);
                        let y = self.name("y");
                        (format!("(let g1 = {} in let g2 = {} in fun {y} -> g1 (g2 {y}))", g.0, h.0), g.1 || h.1)
                    }
                }
            }
            IArr | FArr | IRef => self.leaf(env, ty, al),
        }
    }

    fn arg(&mut self, env: &Env, ty: Ty, local_ok: bool, d: u32, fuel_arg: bool) -> String {
        if fuel_arg {
            return format!("({} land 7)", self.e(env, Int, d.min(1), false));
        }
        self.expr(env, ty, d, local_ok).0
    }

    fn int_expr(&mut self, env: &Env, d: u32, al: bool) -> String {
        let k = self.s.int(0u32..=22);
        match k {
            0..=3 => {
                let op = self.pick(&["+", "-", "*", "land", "lor", "lxor", "+", "-"]);
                format!("({} {op} {})", self.e(env, Int, d, al), self.e(env, Int, d, al))
            }
            4 => {
                let op = self.pick(&["lsl", "lsr", "asr"]);
                format!("({} {op} ({} land 63))", self.e(env, Int, d, al), self.e(env, Int, d, al))
            }
            5 => {
                let op = self.pick(&["/", "mod"]);
                let x = self.name("d");
                format!("(let {x} = {} in if {x} = 0 then {} else {} {op} {x})", self.e(env, Int, d, al), self.e(env, Int, d, al), self.e(env, Int, d, al))
            }
            6 => {
                let op = self.pick(&["-", "lnot", "abs", "succ", "pred"]);
                format!("({op} {})", self.e(env, Int, d, al))
            }
            7 => {
                let t = *[Int, Float, I64, I32, Str, Bool].get(self.s.index(6)).unwrap();
                let (a, b) = (self.e(env, t, d, al), self.e(env, t, d, al));
                let f = self.pick(&["compare", "min", "max"]);
                if f == "compare" || t == Int {
                    let f = if t == Int { f } else { "compare" };
                    format!("({f} {a} {b})")
                } else {
                    format!("(compare {a} {b})")
                }
            }
            8 => format!("(fi {})", self.e(env, Float, d, al)),
            9 => format!("(h_i64 {})", self.e(env, I64, d, al)),
            10 => format!("(h_i32 {})", self.e(env, I32, d, al)),
            11 => format!("(I64U.to_int {})", self.e(env, I64u, d, al)),
            12 => {
                // observe any variable
                if env.vars.is_empty() {
                    return self.lit(Int, env, al).0;
                }
                let v = env.vars[self.s.index(env.vars.len())].clone();
                if v.local && !v.ty.immediate() && !al {
                    return self.lit(Int, env, al).0;
                }
                observe(v.ty, &v.name)
            }
            13 => {
                let s = self.e(env, Str, d, al);
                let x = self.name("s");
                let i = self.e(env, Int, d, al);
                if self.chance(0.5) {
                    format!("(String.length {s})")
                } else {
                    format!("(let {x} = {s} in if String.length {x} = 0 then 0 else Char.code {x}.[({i}) land max_int mod String.length {x}])")
                }
            }
            14 => {
                // closure application
                let g = self.expr(env, Fun1, d, al);
                let a = self.e(env, Int, d, al);
                format!("({} {a})", g.0)
            }
            15 => {
                let t = *[R0, R1, R1].get(self.s.index(3)).unwrap();
                let r = self.expr(env, t, d, al).0;
                match t {
                    R0 => format!("({r}).f0"),
                    _ => format!("({r}).{}", self.pick(&["g0", "g1"])),
                }
            }
            16 => {
                if let Some((a, _)) = self.var(env, IArr, false) {
                    let i = self.e(env, Int, d, al);
                    format!("{a}.(({i}) land max_int mod Array.length {a})")
                } else if let Some((r, _)) = self.var(env, IRef, false) {
                    format!("!{r}")
                } else {
                    self.lit(Int, env, al).0
                }
            }
            17 => format!("(Bool.to_int {})", self.e(env, Bool, d, al)),
            18 => format!("(h_v1 {})", self.e(env, V1, d, al)),
            20..=22 => self.small_num(env, d, al),
            _ => format!("(Float.to_int (Float.of_int ({} land 0xfffffffffff)))", self.e(env, Int, d, al)),
        }
    }

    /// Int-valued expressions through the small-number, nativeint, float32,
    /// unboxed-product and binary string/bytes primitives (Stdlib_stable /
    /// Stdlib_upstream_compatible). Shift counts are masked to the width,
    /// divisors guarded, float32 observed through `hf` (exact widening).
    fn small_num(&mut self, env: &Env, d: u32, al: bool) -> String {
        let k = self.s.int(0u32..=7);
        match k {
            0 | 1 => {
                // int8 / int16, boxed (immediate) or unboxed
                let (m, w) = *[("I8", 7), ("I16", 15), ("I8U", 7), ("I16U", 15)].get(self.s.index(4)).unwrap();
                let a = format!("({m}.of_int {})", self.e(env, Int, d, al));
                let b = format!("({m}.of_int {})", self.e(env, Int, d, al));
                let x = self.name("n");
                let body = match self.s.int(0u32..=6) {
                    0 | 1 => {
                        let op = self.pick(&["add", "sub", "mul", "logand", "logor", "logxor", "min", "max"]);
                        format!("({m}.{op} {a} {b})")
                    }
                    2 => {
                        let op = self.pick(&["shift_left", "shift_right", "shift_right_logical"]);
                        format!("({m}.{op} {a} ({} land {w}))", self.e(env, Int, d, al))
                    }
                    3 => {
                        let op = self.pick(&["div", "rem", "unsigned_div", "unsigned_rem"]);
                        format!("(let {x} = {b} in if {m}.equal {x} ({m}.of_int 0) then {a} else {m}.{op} {a} {x})")
                    }
                    4 => {
                        let op = self.pick(&["neg", "abs", "succ", "pred", "lognot"]);
                        format!("({m}.{op} {a})")
                    }
                    5 => {
                        let op = self.pick(&["compare", "unsigned_compare"]);
                        return format!("({m}.{op} {a} {b})");
                    }
                    _ => return format!("({m}.unsigned_to_int {a})"),
                };
                format!("({m}.to_int {body})")
            }
            2 => {
                // nativeint, boxed or unboxed
                let a = self.e(env, Int, d, al);
                let b = self.e(env, Int, d, al);
                let x = self.name("n");
                if self.chance(0.5) {
                    match self.s.int(0u32..=3) {
                        0 | 1 => {
                            let op = self.pick(&["add", "sub", "mul", "logand", "logor", "logxor"]);
                            format!("(Nativeint.to_int (Nativeint.{op} (Nativeint.of_int {a}) (Nativeint.of_int {b})))")
                        }
                        2 => {
                            let op = self.pick(&["shift_left", "shift_right", "shift_right_logical"]);
                            format!("(Nativeint.to_int (Nativeint.{op} (Nativeint.of_int {a}) ({b} land 63)))")
                        }
                        _ => {
                            let op = self.pick(&["div", "rem", "unsigned_div", "unsigned_rem"]);
                            format!("(let {x} = Nativeint.of_int {b} in if {x} = 0n then 0 else Nativeint.to_int (Nativeint.{op} (Nativeint.of_int {a}) {x}))")
                        }
                    }
                } else {
                    let op = self.pick(&["add", "sub", "mul", "logand", "logor", "logxor"]);
                    let u = self.pick(&["NU", "I32U"]);
                    format!("({u}.to_int ({u}.{op} ({u}.of_int {a}) ({u}.of_int {b})))")
                }
            }
            3 | 4 => {
                // float32 (correctly rounded single precision in every config, R11)
                let m = if self.chance(0.5) { "F32" } else { "F32U" };
                // F32U converts from/to float#, F32 from/to float
                let (wrap, obs) = if m == "F32" { ("", "hf") } else { ("FU.of_float ", "hfu") };
                let conv = |g: &mut Self| -> String {
                    if g.chance(0.7) {
                        format!("({m}.of_float ({wrap}{}))", g.e(env, Float, d, false))
                    } else {
                        format!("({m}.of_int {})", g.e(env, Int, d, al))
                    }
                };
                let a = conv(self);
                let body = match self.s.int(0u32..=4) {
                    0 | 1 => {
                        let op = self.pick(&["add", "sub", "mul", "div", "min", "max", "copy_sign", "rem"]);
                        let b = conv(self);
                        format!("({m}.{op} {a} {b})")
                    }
                    2 => {
                        let op = self.pick(&["neg", "abs", "sqrt", "round", "floor", "ceil", "trunc", "succ", "pred"]);
                        format!("({m}.{op} {a})")
                    }
                    3 => {
                        let (b, c) = (conv(self), conv(self));
                        format!("({m}.fma {a} {b} {c})")
                    }
                    _ => {
                        let b = conv(self);
                        return format!("(Bool.to_int ({m}.equal {a} {b}) + 2 * {m}.compare {a} {b})");
                    }
                };
                format!("({obs} ({m}.to_float {body}))")
            }
            5 => {
                // binary string accessors at a guarded index
                let s = self.e(env, Str, d, al);
                let i = self.e(env, Int, d, al);
                let x = self.name("s");
                let (f, w, obs) = *[
                    ("get_uint8", 1, ""), ("get_int8", 1, ""), ("get_uint16_le", 2, ""), ("get_int16_be", 2, ""),
                    ("get_uint16_ne", 2, ""), ("get_int32_le", 4, "Int32.to_int"), ("get_int32_be", 4, "Int32.to_int"),
                    ("get_int64_le", 8, "h_i64"), ("get_int64_be", 8, "h_i64"),
                ]
                .get(self.s.index(9))
                .unwrap();
                format!("(let {x} = {s} in if String.length {x} < {w} then 0 else {obs} (String.{f} {x} (({i}) land max_int mod (String.length {x} - {} ))))", w - 1)
            }
            6 => {
                // write into fresh bytes, read back with another width/endianness
                let s = self.e(env, Str, d, al);
                let v = self.e(env, Int, d, al);
                let i = self.e(env, Int, d, al);
                let b = self.name("b");
                let j = self.name("j");
                let (set, w) = *[("set_int8", 1), ("set_uint16_le", 2), ("set_int16_be", 2), ("set_int32_le", 4), ("set_int64_be", 8)]
                    .get(self.s.index(5))
                    .unwrap();
                let val = match w {
                    4 => format!("(Int32.of_int {v})"),
                    8 => format!("(Int64.of_int {v})"),
                    _ => v,
                };
                format!(
                    "(let {b} = Bytes.of_string ({s} ^ \"01234567\") in let {j} = ({i}) land max_int mod (Bytes.length {b} - 7) in Bytes.{set} {b} {j} {val}; Bytes.get_uint16_be {b} {j} + Int64.to_int (Bytes.get_int64_le {b} (Bytes.length {b} - 8)))"
                )
            }
            _ => {
                // unboxed products
                let a = self.e(env, Int, d, al);
                let f = self.e(env, Fu, d, al);
                let b = self.e(env, Int, d, al);
                let (x, y, z) = (self.name("u"), self.name("u"), self.name("u"));
                let p = self.name("q");
                if self.chance(0.5) {
                    format!("(let {p} = #({a}, {f}, IU.of_int {b}) in let #({x}, {y}, {z}) = {p} in mix (mix {x} (hfu {y})) (IU.to_int {z}))")
                } else {
                    format!("(let #({x}, {y}) = (if {a} land 1 = 0 then #({b}, {f}) else #({a}, FU.add {f} #1.0)) in mix {x} (hfu {y}))")
                }
            }
        }
    }

    fn bool_expr(&mut self, env: &Env, d: u32, al: bool) -> String {
        let k = self.s.int(0u32..=7);
        match k {
            0 | 1 => {
                let op = self.pick(&["<", "<=", "=", "<>", ">", ">="]);
                format!("({} {op} {})", self.e(env, Int, d, al), self.e(env, Int, d, al))
            }
            2 => {
                let op = self.pick(&["<", "<=", "=", "<>", ">"]);
                format!("({} {op} {})", self.e(env, Float, d, al), self.e(env, Float, d, al))
            }
            3 => {
                let op = self.pick(&["&&", "||"]);
                format!("({} {op} {})", self.e(env, Bool, d, al), self.e(env, Bool, d, al))
            }
            4 => format!("(not {})", self.e(env, Bool, d, al)),
            5 => {
                let f = self.pick(&["Float.is_integer", "Float.is_finite", "Float.sign_bit"]);
                // these take global floats; use a fresh untainted expression
                format!("({f} {})", self.e(env, Float, d, false))
            }
            6 => {
                let t = *[I64, I32, Fu].get(self.s.index(3)).unwrap();
                match t {
                    I64 => format!("(Int64.compare {} {} < 0)", self.e(env, I64, d, false), self.e(env, I64, d, false)),
                    I32 => format!("(Int32.unsigned_compare {} {} > 0)", self.e(env, I32, d, false), self.e(env, I32, d, false)),
                    _ => format!("(FU.compare {} {} = 0)", self.e(env, Fu, d, al), self.e(env, Fu, d, al)),
                }
            }
            _ => {
                let v = self.e(env, V0, d, al);
                format!("(match {v} with A0 | D0 (_, A0) -> true | _ -> false)")
            }
        }
    }

    fn float_expr(&mut self, env: &Env, d: u32, al: bool) -> E {
        let k = self.s.int(0u32..=11);
        let mut t = false;
        let mut f = |g: &mut Self| {
            let e = g.expr(env, Float, d, al);
            t |= e.1;
            e.0
        };
        let s = match k {
            0..=2 => {
                let op = ["+.", "-.", "*.", "/."][self.s.index(4)];
                format!("({} {op} {})", f(self), f(self))
            }
            3 => format!("(Float.fma {} {} {})", self.e(env, Float, d, false), self.e(env, Float, d, false), self.e(env, Float, d, false)),
            4 => {
                let op = self.pick(&["sqrt", "Float.abs", "Float.neg", "floor", "ceil", "Float.round", "Float.trunc", "Float.succ", "Float.pred"]);
                format!("({op} {})", self.e(env, Float, d, false))
            }
            5 => {
                let op = self.pick(&["Float.min", "Float.max", "Float.rem", "Float.copy_sign", "Float.min_num", "Float.max_num"]);
                format!("({op} {} {})", self.e(env, Float, d, false), self.e(env, Float, d, false))
            }
            6 => format!("(Float.of_int {})", self.e(env, Int, d, al)),
            7 => format!("(FU.to_float {})", self.e(env, Fu, d, al)),
            8 => format!("(Int64.to_float {})", self.e(env, I64, d, false)),
            9 => {
                let t2 = *[R0, R2, R2].get(self.s.index(3)).unwrap();
                let r = self.expr(env, t2, d, false).0;
                match t2 {
                    R0 => format!("({r}).f1"),
                    _ => format!("({r}).{}", self.pick(&["h0", "h1"])),
                }
            }
            10 => {
                if let Some((a, _)) = self.var(env, FArr, false) {
                    let i = self.e(env, Int, d, al);
                    format!("{a}.(({i}) land max_int mod Array.length {a})")
                } else {
                    format!("(Int32.to_float {})", self.e(env, I32, d, false))
                }
            }
            _ => format!("(ldexp {} ({} land 127 - 64))", self.e(env, Float, d, false), self.e(env, Int, d, al)),
        };
        (s, t)
    }

    fn i64_expr(&mut self, env: &Env, d: u32, al: bool) -> E {
        let k = self.s.int(0u32..=7);
        let mut t = false;
        let mut f = |g: &mut Self| {
            let e = g.expr(env, I64, d, al);
            t |= e.1;
            e.0
        };
        let s = match k {
            0..=2 => {
                let op = ["add", "sub", "mul", "logand", "logor", "logxor"][self.s.index(6)];
                format!("(Int64.{op} {} {})", f(self), f(self))
            }
            3 => {
                let op = self.pick(&["shift_left", "shift_right", "shift_right_logical"]);
                format!("(Int64.{op} {} ({} land 63))", f(self), self.e(env, Int, d, al))
            }
            4 => {
                let op = self.pick(&["div", "rem", "unsigned_div", "unsigned_rem"]);
                let x = self.name("d");
                format!("(let {x} = {} in if {x} = 0L then {} else Int64.{op} {} {x})", self.e(env, I64, d, false), f(self), self.e(env, I64, d, false))
            }
            5 => format!("(Int64.of_int {})", self.e(env, Int, d, al)),
            6 => format!("(Int64.of_int32 {})", self.expr(env, I32, d, false).0),
            _ => format!("(I64U.to_int64 {})", self.e(env, I64u, d, al)),
        };
        (s, t)
    }

    fn i32_expr(&mut self, env: &Env, d: u32, al: bool) -> E {
        let k = self.s.int(0u32..=6);
        let mut t = false;
        let mut f = |g: &mut Self| {
            let e = g.expr(env, I32, d, al);
            t |= e.1;
            e.0
        };
        let s = match k {
            0..=2 => {
                let op = ["add", "sub", "mul", "logand", "logor", "logxor"][self.s.index(6)];
                format!("(Int32.{op} {} {})", f(self), f(self))
            }
            3 => {
                let op = self.pick(&["shift_left", "shift_right", "shift_right_logical"]);
                format!("(Int32.{op} {} ({} land 31))", f(self), self.e(env, Int, d, al))
            }
            4 => {
                let op = self.pick(&["div", "rem", "unsigned_div"]);
                let x = self.name("d");
                format!("(let {x} = {} in if {x} = 0l then {} else Int32.{op} {} {x})", self.e(env, I32, d, false), f(self), self.e(env, I32, d, false))
            }
            5 => format!("(Int32.of_int {})", self.e(env, Int, d, al)),
            _ => format!("(Int64.to_int32 {})", self.expr(env, I64, d, false).0),
        };
        (s, t)
    }

    fn fu_expr(&mut self, env: &Env, d: u32, al: bool) -> String {
        let k = self.s.int(0u32..=6);
        match k {
            0..=2 => {
                let op = self.pick(&["add", "sub", "mul", "div", "min", "max"]);
                format!("(FU.{op} {} {})", self.e(env, Fu, d, al), self.e(env, Fu, d, al))
            }
            3 => {
                let op = self.pick(&["neg", "abs", "sqrt", "round", "floor", "succ"]);
                format!("(FU.{op} {})", self.e(env, Fu, d, al))
            }
            4 => format!("(FU.of_float {})", self.e(env, Float, d, al)),
            5 => format!("(FU.of_int {})", self.e(env, Int, d, al)),
            _ => {
                let r = self.expr(env, R0, d, al).0;
                format!("({r}).f2")
            }
        }
    }

    fn i64u_expr(&mut self, env: &Env, d: u32, al: bool) -> String {
        let k = self.s.int(0u32..=6);
        match k {
            0..=2 => {
                let op = self.pick(&["add", "sub", "mul", "logand", "logor", "logxor"]);
                format!("(I64U.{op} {} {})", self.e(env, I64u, d, al), self.e(env, I64u, d, al))
            }
            3 => {
                let op = self.pick(&["shift_left", "shift_right", "shift_right_logical"]);
                format!("(I64U.{op} {} ({} land 63))", self.e(env, I64u, d, al), self.e(env, Int, d, al))
            }
            4 => format!("(I64U.of_int64 {})", self.e(env, I64, d, al)),
            5 => format!("(I64U.of_int {})", self.e(env, Int, d, al)),
            _ => {
                let r = self.expr(env, R0, d, al).0;
                format!("({r}).f3")
            }
        }
    }

    /// pattern match producing a value of type `ty`
    fn match_expr(&mut self, env: &Env, ty: Ty, d: u32, al: bool) -> E {
        let st = *[V0, V0, V1, V2, Int, Pair, R1, Int].get(self.s.index(8)).unwrap();
        let (mut scr, stl) = self.expr(env, st, d, al);
        // Known -Oclassic crash: a match on a statically known v2 constructor
        // whose dead arms read fields of another kind (float# vs value) fails in
        // the backend ("Illegal move between registers of differing types",
        // or an assembler error). See agents/harness-002/cand-oclassic-illegal-move.
        // Hide the constant from the optimiser most of the time.
        if st == V2 && !scr.starts_with(|c: char| c.is_ascii_lowercase()) && self.chance(0.95) {
            scr = format!("(Sys.opaque_identity {scr})");
        }
        let mut tainted = false;
        let mut arm = |g: &mut Self, binds: &[(String, Ty)]| -> String {
            let mut env2 = env.clone();
            for (n, t) in binds {
                env2.push(Var { name: n.clone(), ty: *t, local: stl && !t.immediate(), mutable: false });
            }
            let e = g.expr(&env2, ty, d, al);
            tainted |= e.1;
            e.0
        };
        let mut s = format!("(match {scr} with");
        match st {
            V0 => {
                s += &format!(" | A0 -> {}", arm(self, &[]));
                let n = self.name("n");
                if self.chance(0.4) {
                    let c = {
                        let mut env2 = env.clone();
                        env2.push(Var { name: n.clone(), ty: Int, local: false, mutable: false });
                        self.e(&env2, Bool, d.min(1), al)
                    };
                    s += &format!(" | B0 {n} when {c} -> {}", arm(self, &[(n.clone(), Int)]));
                }
                s += &format!(" | B0 {n} -> {}", arm(self, &[(n.clone(), Int)]));
                let (f, m) = (self.name("f"), self.name("n"));
                if self.chance(0.5) {
                    s += &format!(" | C0 ({f}, {m}) -> {}", arm(self, &[(f.clone(), Float), (m.clone(), Int)]));
                }
                let (m2, v) = (self.name("n"), self.name("v"));
                if self.chance(0.5) {
                    s += &format!(" | D0 ({m2}, A0) -> {}", arm(self, &[(m2.clone(), Int)]));
                }
                if self.chance(0.5) {
                    s += &format!(" | D0 ({m2}, {v}) -> {}", arm(self, &[(m2.clone(), Int), (v.clone(), V0)]));
                }
                s += &format!(" | _ -> {}", arm(self, &[]));
            }
            V1 => {
                let cs = ["X1", "Y1", "Z1", "W1", "U1"];
                let k = self.s.int(1usize..=4);
                for c in cs.iter().take(k) {
                    if self.chance(0.3) {
                        s += &format!(" | {c} | U1 -> {}", arm(self, &[]));
                        break;
                    }
                    s += &format!(" | {c} -> {}", arm(self, &[]));
                }
                s += &format!(" | _ -> {}", arm(self, &[]));
            }
            V2 => {
                let n = self.name("n");
                s += &format!(" | P2 {n} -> {}", arm(self, &[(n.clone(), I64)]));
                let (f, m) = (self.name("f"), self.name("n"));
                s += &format!(" | Q2 ({f}, {m}) -> {}", arm(self, &[(f.clone(), Fu), (m.clone(), Int)]));
                if self.chance(0.5) {
                    let (a, b) = (self.name("a"), self.name("f"));
                    s += &format!(" | S2 ({a}, {b}) -> {}", arm(self, &[(a.clone(), I32), (b.clone(), Float)]));
                }
                s += &format!(" | _ -> {}", arm(self, &[]));
            }
            Int => {
                let k = self.s.int(1u32..=6);
                let mut used = std::collections::HashSet::new();
                for _ in 0..k {
                    let c = self.s.int(-2i32..=9);
                    if !used.insert(c) {
                        continue;
                    }
                    let lit = if c < 0 { format!("({c})") } else { c.to_string() };
                    s += &format!(" | {lit} -> {}", arm(self, &[]));
                }
                let n = self.name("n");
                s += &format!(" | {n} -> {}", arm(self, &[(n.clone(), Int)]));
            }
            Pair => {
                let (a, b) = (self.name("a"), self.name("b"));
                if self.chance(0.5) {
                    s += &format!(" | (0, {b}) -> {}", arm(self, &[(b.clone(), Int)]));
                }
                s += &format!(" | ({a}, {b}) -> {}", arm(self, &[(a.clone(), Int), (b.clone(), Int)]));
            }
            _ => {
                // R1
                let (a, b) = (self.name("a"), self.name("b"));
                if self.chance(0.5) {
                    s += &format!(" | {{ g2 = X1; g0 = {a}; _ }} -> {}", arm(self, &[(a.clone(), Int)]));
                }
                s += &format!(" | {{ g0 = {a}; g1 = {b}; _ }} -> {}", arm(self, &[(a.clone(), Int), (b.clone(), Int)]));
            }
        }
        s += ")";
        (s, tainted)
    }

    /// call a pure function returning `ty`
    fn call(&mut self, env: &Env, ty: Ty, d: u32, al: bool) -> Option<E> {
        let fs: Vec<Func> = self.funcs.iter().filter(|f| f.ret == ty && f.pure_).cloned().collect();
        if fs.is_empty() {
            return None;
        }
        let f = fs[self.s.index(fs.len())].clone();
        Some((self.render_call(env, &f, d, al), false))
    }

    fn render_call(&mut self, env: &Env, f: &Func, d: u32, al: bool) -> String {
        let mut args = Vec::new();
        let mut any_local = false;
        for (i, (t, loc)) in f.params.iter().enumerate() {
            if i == 0 && f.rec_ {
                args.push(self.arg(env, *t, false, d, true));
                continue;
            }
            let e = self.expr(env, *t, d.min(2), *loc && al);
            any_local |= e.1;
            args.push(e.0);
        }
        if any_local {
            format!("({} {} [@nontail])", f.name, args.join(" "))
        } else {
            format!("({} {})", f.name, args.join(" "))
        }
    }

    // ---------------------------------------------------------------- statements

    /// A block: statements followed by a final int-observing expression.
    /// `ret` is the block's result type; locals never flow into it.
    fn block(&mut self, env: &Env, n: usize, ret: Ty, d: u32, in_loop: bool) -> String {
        let mut env = env.clone();
        let mut out = String::new();
        let start = env.vars.len();
        for _ in 0..n {
            if self.fuel <= 0 {
                break;
            }
            out += &self.stmt(&mut env, d, in_loop);
        }
        // final expression: mix of what this block bound (keeps values live)
        let fin = if ret == Int {
            let mut acc = format!("mix 0 ({})", self.e(&env, Int, 1, true));
            let new: Vec<Var> = env.vars[start..].to_vec();
            for v in new.iter().rev().take(6) {
                acc = format!("mix {} ({acc})", observe(v.ty, &v.name));
            }
            acc
        } else {
            let e = self.expr(&env, ret, d, false);
            e.0
        };
        out + &fin
    }

    fn stmt(&mut self, env: &mut Env, d: u32, in_loop: bool) -> String {
        let k = self.s.int(0u32..=99);
        let foc = self.focus;
        // focus-specific statements get extra weight
        let k = if self.chance(0.35) {
            match foc {
                Focus::Local => 40 + self.s.int(0u32..=9),
                Focus::Loop => 50 + self.s.int(0u32..=9),
                Focus::Exn => 60 + self.s.int(0u32..=9),
                Focus::Array => 70 + self.s.int(0u32..=5),
                Focus::Record => 76 + self.s.int(0u32..=3),
                Focus::Closure => 80 + self.s.int(0u32..=4),
                _ => k,
            }
        } else {
            k
        };
        match k {
            0..=39 => {
                let t = self.some_ty();
                let x = self.name("x");
                let e = self.expr(env, t, d, true);
                env.push(Var { name: x.clone(), ty: t, local: e.1, mutable: false });
                format!("  let {x} = {} in\n", e.0)
            }
            40..=49 => {
                // local allocations
                let x = self.name("l");
                let w = self.s.int(0u32..=5);
                let (code, ty) = match w {
                    0 => (format!("stack_ ({}, {})", self.e(env, Int, d, true), self.e(env, Int, d, true)), Pair),
                    1 => (format!("mkpair_l {} {}", self.e(env, Int, d, true), self.e(env, Int, d, true)), Pair),
                    2 => (format!("stack_ (D0 ({}, {}))", self.e(env, Int, d, true), self.expr(env, V0, d, false).0), V0),
                    3 => (format!("mkv0_l {} {}", self.e(env, Int, d, true), self.expr(env, Float, d, false).0), V0),
                    4 => (format!("stack_ {{ g0 = {}; g1 = {}; g2 = {} }}", self.e(env, Int, d, true), self.e(env, Int, d, true), self.e(env, V1, d, true)), R1),
                    _ => {
                        let y = self.name("y");
                        let mut env2 = env.clone();
                        env2.vars.retain(|v| !v.mutable);
                        env2.push(Var { name: y.clone(), ty: Int, local: false, mutable: false });
                        let b = self.e(&env2, Int, d, true);
                        (format!("stack_ (fun {y} -> mix 2 ({b}))"), Fun1)
                    }
                };
                env.push(Var { name: x.clone(), ty, local: true, mutable: false });
                format!("  let {x} = {code} in\n")
            }
            50..=53 => {
                // let mutable + for loop
                let t = *[Int, Float, Fu, I64u, Int].get(self.s.index(5)).unwrap();
                let m = self.name("m");
                let init = self.expr(env, t, d, false).0;
                env.push(Var { name: m.clone(), ty: t, local: false, mutable: true });
                let i = self.name("i");
                let lo = self.s.int(0u32..=2);
                let hi = self.s.int(0u32..=7);
                let dir = if self.chance(0.2) { "downto" } else { "to" };
                let (lo, hi) = if dir == "downto" { (hi, lo) } else { (lo, hi) };
                let mut env2 = env.clone();
                env2.push(Var { name: i.clone(), ty: Int, local: false, mutable: false });
                let body = self.loop_body(&mut env2, d);
                let upd = self.expr(&env2, t, d, false).0;
                format!("  let mutable {m} = {init} in\n  for {i} = {lo} {dir} {hi} do\n{body}    {m} <- {upd}\n  done;\n")
            }
            54..=56 => {
                // ref + while
                let r = self.name("r");
                let c = self.name("c");
                let init = self.e(env, Int, d, false);
                env.push(Var { name: r.clone(), ty: IRef, local: false, mutable: false });
                let k = self.s.int(0u32..=6);
                let mut env2 = env.clone();
                env2.push(Var { name: c.clone(), ty: IRef, local: false, mutable: false });
                let body = self.loop_body(&mut env2, d);
                let upd = self.e(&env2, Int, d, false);
                format!("  let {r} = ref {init} in\n  let {c} = ref 0 in\n  while !{c} < {k} do\n    incr {c};\n{body}    {r} := {upd}\n  done;\n")
            }
            57..=59 => {
                // assignment to an existing mutable thing
                self.assign(env, d)
            }
            60..=63 if self.effects => {
                let c = self.e(env, Bool, d, true);
                let x = self.e(env, Int, d, true);
                let ex = match self.s.int(0u32..=3) {
                    0 => "E0".to_string(),
                    1 => format!("(E1 {x})"),
                    2 => format!("(E2 (string_of_int {x}))"),
                    _ => format!("(Invalid_argument \"g\")"),
                };
                let r = if self.chance(0.3) { "raise_notrace" } else { "raise" };
                format!("  if {c} then {r} {ex};\n")
            }
            64..=66 => {
                // try with
                let x = self.name("x");
                let saved = self.effects;
                self.effects = true;
                let n = self.s.int(1usize..=3);
                let body = self.block(env, n, Int, d, in_loop);
                self.effects = saved;
                let e = self.name("e");
                let h1 = self.e(env, Int, d, true);
                let mut env2 = env.clone();
                env2.push(Var { name: e.clone(), ty: Int, local: false, mutable: false });
                let h2 = self.e(&env2, Int, d, true);
                let h3 = self.e(env, Int, d, true);
                env.push(Var { name: x.clone(), ty: Int, local: false, mutable: false });
                let reraise = if self.effects && self.chance(0.3) { " | E2 s -> raise (E2 (s ^ \"!\"))" } else { " | E2 _ -> 0" };
                format!("  let {x} =\n    try\n{body}\n    with E0 -> {h1} | E1 {e} -> {h2} | Division_by_zero -> {h3} | Invalid_argument _ -> 9{reraise} in\n")
            }
            67 if self.effects => {
                // raising division
                let x = self.name("x");
                let a = self.e(env, Int, d, true);
                let b = self.e(env, Int, d, true);
                env.push(Var { name: x.clone(), ty: Int, local: false, mutable: false });
                format!("  let {x} = {a} / {b} in\n")
            }
            68 | 69 => {
                // effectful call, or match with exception
                let fs: Vec<Func> = self.funcs.iter().filter(|f| !f.pure_ && f.ret == Int).cloned().collect();
                if fs.is_empty() || !self.global_fx {
                    return self.stmt_let_int(env, d);
                }
                let f = fs[self.s.index(fs.len())].clone();
                let call = self.render_call(env, &f, d, true);
                let x = self.name("x");
                let s = if self.chance(0.5) {
                    let n = self.name("n");
                    let mut env2 = env.clone();
                    env2.push(Var { name: n.clone(), ty: Int, local: false, mutable: false });
                    let a = self.e(&env2, Int, d, true);
                    format!("  let {x} = match {call} with exception E1 {n} -> {a} | {n} -> {n} + 1 in\n")
                } else {
                    format!("  let {x} = {call} in\n")
                };
                env.push(Var { name: x.clone(), ty: Int, local: false, mutable: false });
                s
            }
            70..=75 => {
                // arrays
                let x = self.name("a");
                let n = self.s.int(1u32..=6);
                let (code, ty) = if self.chance(0.5) {
                    if self.chance(0.5) {
                        (format!("Array.make {n} {}", self.e(env, Int, d, false)), IArr)
                    } else {
                        let i = self.name("i");
                        let mut env2 = env.clone();
                        env2.vars.retain(|v| !v.mutable && !(v.local && !v.ty.immediate()));
                        env2.push(Var { name: i.clone(), ty: Int, local: false, mutable: false });
                        (format!("Array.init {n} (fun {i} -> {})", self.e(&env2, Int, d, false)), IArr)
                    }
                } else {
                    let els: Vec<String> = (0..n).map(|_| self.expr(env, Float, d.min(2), false).0).collect();
                    (format!("[| {} |]", els.join("; ")), FArr)
                };
                env.push(Var { name: x.clone(), ty, local: false, mutable: false });
                format!("  let {x} = {code} in\n")
            }
            76..=79 => {
                // mutable record
                let t = *[R0, R1, R2].get(self.s.index(3)).unwrap();
                let x = self.name("r");
                let e = self.expr(env, t, d, false).0;
                env.push(Var { name: x.clone(), ty: t, local: false, mutable: false });
                let upd = self.assign_field(env, &x, t, d);
                format!("  let {x} = {e} in\n{upd}")
            }
            80..=84 => {
                let x = self.name("g");
                let e = self.expr(env, Fun1, d + 1, true);
                env.push(Var { name: x.clone(), ty: Fun1, local: e.1, mutable: false });
                format!("  let {x} = {} in\n", e.0)
            }
            85..=89 if self.global_fx => {
                let e = self.e(env, Int, d, true);
                format!("  cnt := mix !cnt ({e});\n")
            }
            _ => self.stmt_let_int(env, d),
        }
    }

    fn stmt_let_int(&mut self, env: &mut Env, d: u32) -> String {
        let x = self.name("x");
        let e = self.e(env, Int, d, true);
        env.push(Var { name: x.clone(), ty: Int, local: false, mutable: false });
        format!("  let {x} = {e} in\n")
    }

    fn loop_body(&mut self, env: &mut Env, d: u32) -> String {
        let n = self.s.int(0usize..=3);
        let mut out = String::new();
        for _ in 0..n {
            let s = self.stmt(env, d.min(2), true);
            out += "  ";
            out += &s;
        }
        // statements bind with `in`; close them with a unit
        out
    }

    fn assign(&mut self, env: &mut Env, d: u32) -> String {
        let cands: Vec<Var> = env
            .vars
            .iter()
            .filter(|v| v.mutable || matches!(v.ty, IRef | IArr | FArr | R0 | R1 | R2) && !v.local)
            .cloned()
            .collect();
        if cands.is_empty() {
            return self.stmt_let_int(env, d);
        }
        let v = cands[self.s.index(cands.len())].clone();
        if v.mutable {
            let e = self.expr(env, v.ty, d, false).0;
            return format!("  {} <- {e};\n", v.name);
        }
        match v.ty {
            IRef => format!("  {} := {};\n", v.name, self.e(env, Int, d, false)),
            IArr | FArr if self.chance(0.3) => {
                // adjacent unchecked stores of loads-plus-values inside a bounds
                // guard: straight-line lanes for the vectorizer (-vectorize toggle)
                let (t, ops) = if v.ty == IArr { (Int, ["+", "-", "land", "lxor"]) } else { (Float, ["+.", "-.", "*.", "+."]) };
                let a = v.name.clone();
                let lanes = 2 + self.s.index(3);
                let j = self.name("j");
                let i = self.e(env, Int, d, true);
                let op = ops[self.s.index(4)];
                let src = self.s.index(2);
                // second operand: a scalar per lane, or adjacent loads (overlapping
                // the stored range, so lane order matters)
                let loaded = self.chance(0.5);
                let mut body = String::new();
                for l in 0..lanes {
                    let e = if loaded { format!("Array.unsafe_get {a} ({j} + {})", (l + 1) % lanes) } else { self.expr(env, t, d.min(1), false).0 };
                    let from = if src == 0 { format!("{j} + {}", lanes - 1 - l) } else { format!("{j} + {l}") };
                    body += &format!("Array.unsafe_set {a} ({j} + {l}) (Array.unsafe_get {a} ({from}) {op} {e}); ");
                }
                format!("  (if Array.length {a} >= {lanes} then begin let {j} = ({i}) land max_int mod (Array.length {a} - {}) in {body}end);\n", lanes - 1)
            }
            IArr | FArr => {
                let t = if v.ty == IArr { Int } else { Float };
                let i = self.e(env, Int, d, true);
                let e = self.expr(env, t, d, false).0;
                format!("  {a}.(({i}) land max_int mod Array.length {a}) <- {e};\n", a = v.name)
            }
            t => self.assign_field(env, &v.name, t, d),
        }
    }

    fn assign_field(&mut self, env: &Env, x: &str, t: Ty, d: u32) -> String {
        match t {
            R0 => {
                if self.chance(0.5) {
                    format!("  {x}.f0 <- {};\n", self.e(env, Int, d, true))
                } else {
                    format!("  {x}.f3 <- {};\n", self.e(env, I64u, d, true))
                }
            }
            R1 => format!("  {x}.g1 <- {};\n", self.e(env, Int, d, true)),
            _ => format!("  {x}.h1 <- {};\n", self.expr(env, Float, d, false).0),
        }
    }

    // ---------------------------------------------------------------- top level

    fn func(&mut self, idx: usize) -> String {
        let name = format!("f{idx}");
        let rec_ = self.chance(0.15);
        let pure_ = self.chance(0.7);
        let nparams = self.s.int(1usize..=3);
        let mut params = Vec::new();
        if rec_ {
            params.push((Int, false));
        }
        let ptys = [Int, Int, Float, Fu, I64u, I64, Pair, V0, V1, R1, Fun1, Str, R0, I32];
        for _ in 0..nparams {
            let t = if self.chance(0.5) { self.some_ty() } else { ptys[self.s.index(ptys.len())] };
            let t = if matches!(t, IArr | FArr | IRef | R2 | V2) && self.chance(0.5) { Int } else { t };
            let loc = matches!(t, Pair | V0 | R1 | Fun1 | Float | I64) && self.chance(0.4);
            params.push((t, loc));
        }
        let rets = [Int, Int, Int, Float, Fu, V0, Pair, I64, Bool];
        let ret = if pure_ { rets[self.s.index(rets.len())] } else { Int };
        let mut env = Env::default();
        let mut ps = Vec::new();
        for (i, (t, loc)) in params.iter().enumerate() {
            let p = format!("p{i}");
            env.push(Var { name: p.clone(), ty: *t, local: *loc, mutable: false });
            ps.push(if *loc { format!("(local_ {p} : {})", tyname(*t)) } else { format!("({p} : {})", tyname(*t)) });
        }
        let attr = *["", "", "[@inline never]", "[@inline always]", "[@inline]", "[@specialise]"]
            .get(self.s.index(6))
            .unwrap();
        let attr = if rec_ && attr == "[@inline always]" { "" } else { attr };
        self.effects = !pure_;
        self.global_fx = !pure_;
        self.fuel = 200;
        let n = self.s.int(1usize..=5);
        let me = Func { name: name.clone(), params: params.clone(), ret, pure_, rec_ };
        let body = if rec_ {
            // recursive call on decreasing fuel inside the body
            let base = self.e(&env, ret, 2, false);
            let mut env2 = env.clone();
            let call = self.render_call(&env2, &me, 2, true);
            // replace fuel argument by (p0 - 1)
            let call = fix_fuel(&call, &name);
            let r = self.name("rr");
            env2.push(Var { name: r.clone(), ty: ret, local: false, mutable: false });
            let rest = self.block(&env2, n, ret, 3, false);
            format!("if p0 <= 0 then {base} else\n  let {r} = {call} in\n{rest}")
        } else {
            self.block(&env, n, ret, 3, false)
        };
        self.funcs.push(me);
        let kw = if rec_ { format!("let{attr} rec") } else { format!("let{attr}") };
        format!("{kw} {name} {} : {} =\n{body}\n", ps.join(" "), tyname(ret))
    }

    fn test(&mut self, idx: usize, focus: Focus) -> String {
        self.focus = focus;
        self.effects = true;
        self.global_fx = true;
        self.fuel = 300;
        let n = self.s.int(2usize..=8);
        // opaque inputs, so the optimiser cannot fold the whole test away
        let mut env = Env::default();
        let mut pre = String::new();
        let k = self.s.int(1usize..=4);
        for _ in 0..k {
            let t = *[Int, Int, Float, I64, I32, Fu, I64u, V0, V2, Str].get(self.s.index(10)).unwrap();
            let t = if self.chance(0.5) { let ts = focus.types(); match ts[self.s.index(ts.len())] { IArr | FArr | IRef | Fun1 | R0 | R1 | R2 | Pair => t, x => x } } else { t };
            let x = self.name("in");
            let v = self.lit(t, &env, false).0;
            let v = match t {
                Fu => format!("(FU.of_float (Sys.opaque_identity (FU.to_float {v})))"),
                I64u => format!("(I64U.of_int64 (Sys.opaque_identity (I64U.to_int64 {v})))"),
                _ => format!("(Sys.opaque_identity {v})"),
            };
            pre += &format!("  let {x} = {v} in\n");
            env.push(Var { name: x, ty: t, local: false, mutable: false });
        }
        let body = pre + &self.block(&env, n, Int, 3, false);
        let attr = if self.chance(0.5) { "[@inline never]" } else { "" };
        format!(
            "let{attr} test_{idx} () =\n{body}\nlet () = pr \"t{idx}\" (try test_{idx} () with E0 -> -10 | E1 n -> mix (-11) n | E2 s -> mix (-12) (h_str s) | Division_by_zero -> -13 | Invalid_argument _ -> -14 | Not_found -> -15)\n"
        )
    }

    pub fn program(&mut self, nfuncs: usize, ntests: usize, foci: &[Focus]) -> Program {
        let mut text = String::from(PRELUDE);
        // functions are generated under a random focus too
        for i in 0..nfuncs {
            self.focus = FOCI[self.s.index(FOCI.len())];
            text += &self.func(i);
        }
        let mut fs = Vec::new();
        for i in 0..ntests {
            let f = if foci.is_empty() { FOCI[self.s.index(FOCI.len())] } else { foci[self.s.index(foci.len())] };
            fs.push(f);
            text += &self.test(i, f);
        }
        text += "let () = pr \"cnt\" !cnt\n";
        Program { text, foci: fs }
    }
}

fn fix_fuel(call: &str, name: &str) -> String {
    // render_call put "(<expr> land 7)" as first argument; make it decrease
    let pre = format!("({name} ");
    if let Some(rest) = call.strip_prefix(&pre) {
        // first argument is a parenthesised group
        let mut depth = 0;
        for (i, ch) in rest.char_indices() {
            match ch {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        return format!("{pre}(p0 - 1){}", &rest[i + 1..]);
                    }
                }
                _ => {}
            }
        }
    }
    call.to_string()
}

pub fn tyname(t: Ty) -> &'static str {
    match t {
        Int => "int",
        Bool => "bool",
        Float => "float",
        I64 => "int64",
        I32 => "int32",
        Fu => "float#",
        I64u => "int64_u",
        V0 => "v0",
        V1 => "v1",
        V2 => "v2",
        R0 => "r0",
        R1 => "r1",
        R2 => "r2",
        Pair => "int * int",
        Str => "string",
        Fun1 => "(int -> int)",
        IArr => "int array",
        FArr => "float array",
        IRef => "int ref",
    }
}
