//! The e-graph's language, a query's encoding into it and back, and the
//! expression rewrite rules.
//!
//! A query is one term: `(q s0 s1 ..)`, a child per expression slot
//! ([`slots_mut`]: each SELECT's items, ON clauses, WHERE, GROUP BY terms
//! and HAVING, for every CTE, the query and its compound members), the rest
//! of the query (FROM, set operators, ORDER BY) kept aside as a skeleton.
//! Leaves are symbols `#k` into a side table ([`Payload`]): columns,
//! literals, a function or cast's parameter, and subqueries, whose own
//! slots are further children of `q`, so rules rewrite inside them too.
//! Decoding rebuilds ephyra-postgresql's AST, which the caller type-checks
//! (`Query::valid`): no rule needs to know types, but none may change one.
//!
//! Every rule is an equality of PostgreSQL's three-valued logic for all
//! values of every type it can meet (TRUE, FALSE and NULL alike, NaN being
//! the largest real, `-0 = 0`), so it holds wherever the expression is, not
//! only at the top of a WHERE. Rules that duplicate an operand (BETWEEN,
//! IN-lists, simple CASE, coalesce) are fine because the language has no
//! volatile expression. Not used, because they are not equalities: `NOT`
//! pushed through a CASE condition, `x = x` to TRUE, `-(-x)` to `x` (`-x`
//! overflows on the smallest integer), reassociating real arithmetic.

use egg::{define_language, rewrite as rw, Analysis, DidMerge, EGraph, Id, RecExpr, Rewrite, Symbol};
use ephyra_postgresql::{
    expr::{arith_type, AggFunc, Arith, Col, Expr, Func, IsWhat, Query, Select},
    model::{Type, Value},
    statements::CmpOp,
};

define_language! {
    pub enum Sql {
        "q" = Q(Box<[Id]>),
        "and" = And([Id; 2]),
        "or" = Or([Id; 2]),
        "not" = Not([Id; 1]),
        "=" = Eq([Id; 2]),
        "<>" = Ne([Id; 2]),
        "<" = Lt([Id; 2]),
        "<=" = Le([Id; 2]),
        ">" = Gt([Id; 2]),
        ">=" = Ge([Id; 2]),
        "is-distinct" = IsDistinct([Id; 2]),
        "is-not-distinct" = IsNotDistinct([Id; 2]),
        "is-null" = IsNull([Id; 1]),
        "is-not-null" = IsNotNull([Id; 1]),
        "is-true" = IsTrue([Id; 1]),
        "is-not-true" = IsNotTrue([Id; 1]),
        "is-false" = IsFalse([Id; 1]),
        "is-not-false" = IsNotFalse([Id; 1]),
        "is-unknown" = IsUnknown([Id; 1]),
        "is-not-unknown" = IsNotUnknown([Id; 1]),
        "between" = Between([Id; 3]),
        "not-between" = NotBetween([Id; 3]),
        "between-sym" = BetweenSym([Id; 3]),
        "not-between-sym" = NotBetweenSym([Id; 3]),
        // (in needle item..)
        "in" = In(Box<[Id]>),
        "not-in" = NotIn(Box<[Id]>),
        // (like #pattern e): the pattern and flags in the payload.
        "like" = Like([Id; 2]),
        "regex" = Regex([Id; 2]),
        "+" = Add([Id; 2]),
        "-" = Sub([Id; 2]),
        "*" = Mul([Id; 2]),
        "/" = Div([Id; 2]),
        "%" = Mod([Id; 2]),
        "neg" = Neg([Id; 1]),
        "||" = Concat([Id; 2]),
        // (case operand-or-none when then .. else-or-none)
        "case" = Case(Box<[Id]>),
        "coalesce" = Coalesce(Box<[Id]>),
        "nullif" = NullIf([Id; 2]),
        "greatest" = Greatest(Box<[Id]>),
        "least" = Least(Box<[Id]>),
        // (cast #type e)
        "cast" = Cast([Id; 2]),
        // (call #func arg..)
        "call" = Call(Box<[Id]>),
        // (agg #func-distinct [arg])
        "agg" = Agg(Box<[Id]>),
        // (exists #sub), (not-exists #sub), (scalar #sub), (in-sub #sub needle)
        "exists" = Exists([Id; 1]),
        "not-exists" = NotExists([Id; 1]),
        "scalar" = Scalar([Id; 1]),
        "in-sub" = InSub([Id; 2]),
        "not-in-sub" = NotInSub([Id; 2]),
        // `#k` (a payload) or `none`.
        Symbol(Symbol),
    }
}

/// What a leaf symbol stands for.
#[derive(Clone, Debug)]
pub enum Payload {
    Col(Col),
    Lit(Value, Type),
    LikePattern { pattern: String, not: bool, insensitive: bool },
    RegexIndex(usize),
    CastType(Type),
    Func(Func),
    Agg(AggFunc, bool),
    /// A subquery: its skeleton, and which root slots hold its expressions.
    Sub(Box<Query>, std::ops::Range<usize>),
    /// An expression the language does not take apart (a window).
    Opaque(Expr),
}

/// Every expression slot of a SELECT, in a fixed order.
fn select_slots_mut(s: &mut Select) -> Vec<&mut Expr> {
    let Select { items, joins, filter, group, having, .. } = s;
    let mut out: Vec<&mut Expr> = items.iter_mut().collect();
    out.extend(joins.iter_mut().filter_map(|j| j.on.as_mut()));
    out.extend(filter.iter_mut());
    out.extend(group.iter_mut());
    out.extend(having.iter_mut());
    out
}

/// Every expression slot of a query: its CTEs', its own, its compound
/// members'.
pub fn slots_mut(q: &mut Query) -> Vec<&mut Expr> {
    let Query { with, select, compound, .. } = q;
    let mut out = Vec::new();
    for c in with.iter_mut() {
        out.extend(select_slots_mut(&mut c.query));
    }
    out.extend(select_slots_mut(select));
    for (_, m) in compound.iter_mut() {
        out.extend(select_slots_mut(m));
    }
    out
}

/// The GROUP BY keys of the SELECT each slot of [`slots_mut`] belongs to,
/// in the same order (empty for an ungrouped SELECT).
fn slot_keys(q: &Query) -> Vec<Vec<Expr>> {
    let mut out = Vec::new();
    let mut add = |s: &Select| {
        let mut c = s.clone();
        let n = select_slots_mut(&mut c).len();
        out.extend(std::iter::repeat_n(s.group.clone(), n));
    };
    for c in &q.with {
        add(&c.query);
    }
    add(&q.select);
    for (_, m) in &q.compound {
        add(m);
    }
    out
}

/// A query encoded: the term, the payloads its symbols name, and the
/// skeleton the root slots fill.
pub struct Encoded {
    pub term: RecExpr<Sql>,
    pub payloads: Vec<Payload>,
    pub skeleton: Query,
    /// The number of the skeleton's own slots (the first children of `q`).
    pub top_slots: usize,
}

struct Encoder {
    term: RecExpr<Sql>,
    payloads: Vec<Payload>,
    /// The GROUP BY keys of the SELECT being encoded: an item or HAVING
    /// term must match a key as written, so each key is one opaque leaf,
    /// shared by its every occurrence, which no rule rewrites.
    keys: Vec<(Expr, Id)>,
    /// The root's children, in slot order (subqueries' after their parent's
    /// are allocated).
    slots: Vec<Option<Id>>,
}

impl Encoder {
    fn leaf(&mut self, p: Payload) -> Id {
        // The type, where the leaf has one, rides in the symbol (`#3:int4`)
        // for [`Types`].
        let ty = match &p {
            Payload::Col(c) => Some(c.ty),
            Payload::Lit(_, t) | Payload::CastType(t) => Some(*t),
            Payload::Opaque(e) => Some(ephyra_postgresql::expr::type_of(e)),
            _ => None,
        };
        self.payloads.push(p);
        let k = self.payloads.len() - 1;
        let name = match ty {
            Some(t) => format!("#{k}:{}", t.name()),
            None => format!("#{k}"),
        };
        self.term.add(Sql::Symbol(name.into()))
    }

    fn none(&mut self) -> Id {
        self.term.add(Sql::Symbol("none".into()))
    }

    /// Reserve slots for a query's expressions, encode them, and return the
    /// skeleton and the slot range.
    fn query(&mut self, q: &Query) -> (Query, std::ops::Range<usize>) {
        let mut skeleton = q.clone();
        let mut exprs = Vec::new();
        for slot in slots_mut(&mut skeleton) {
            exprs.push(std::mem::replace(slot, Expr::null(Type::Bool)));
        }
        let start = self.slots.len();
        self.slots.extend(std::iter::repeat_n(None, exprs.len()));
        let saved = std::mem::take(&mut self.keys);
        let mut last: Option<Vec<Expr>> = None;
        for ((k, e), keys) in exprs.iter().enumerate().zip(slot_keys(q)) {
            if last.as_ref() != Some(&keys) {
                self.keys = Vec::new();
                for key in &keys {
                    let id = self.leaf(Payload::Opaque(key.clone()));
                    self.keys.push((key.clone(), id));
                }
                last = Some(keys);
            }
            let id = self.expr(e);
            self.slots[start + k] = Some(id);
        }
        self.keys = saved;
        (skeleton, start..start + exprs.len())
    }

    fn sub(&mut self, q: &Query) -> Id {
        let (skeleton, range) = self.query(q);
        self.leaf(Payload::Sub(Box::new(skeleton), range))
    }

    fn expr(&mut self, e: &Expr) -> Id {
        if let Some((_, id)) = self.keys.iter().find(|(k, _)| k == e) {
            return *id;
        }
        let kids = |s: &mut Self, xs: &[&Expr]| -> Vec<Id> { xs.iter().map(|x| s.expr(x)).collect() };
        let node = match e {
            Expr::Col(c) => return self.leaf(Payload::Col(*c)),
            Expr::Lit(v, t) => return self.leaf(Payload::Lit(v.clone(), *t)),
            Expr::Cmp(op, a, b) => {
                let c = kids(self, &[a, b]);
                let c = [c[0], c[1]];
                match op {
                    CmpOp::Eq => Sql::Eq(c),
                    CmpOp::Ne => Sql::Ne(c),
                    CmpOp::Lt => Sql::Lt(c),
                    CmpOp::Le => Sql::Le(c),
                    CmpOp::Gt => Sql::Gt(c),
                    CmpOp::Ge => Sql::Ge(c),
                }
            },
            Expr::Distinct(not, a, b) => {
                let c = kids(self, &[a, b]);
                if *not { Sql::IsNotDistinct([c[0], c[1]]) } else { Sql::IsDistinct([c[0], c[1]]) }
            },
            Expr::And(a, b) => {
                let c = kids(self, &[a, b]);
                Sql::And([c[0], c[1]])
            },
            Expr::Or(a, b) => {
                let c = kids(self, &[a, b]);
                Sql::Or([c[0], c[1]])
            },
            Expr::Not(a) => Sql::Not([self.expr(a)]),
            Expr::IsNull(a, not) => {
                let a = self.expr(a);
                if *not { Sql::IsNotNull([a]) } else { Sql::IsNull([a]) }
            },
            Expr::Is(a, what, not) => {
                let a = [self.expr(a)];
                match (what, not) {
                    (IsWhat::True, false) => Sql::IsTrue(a),
                    (IsWhat::True, true) => Sql::IsNotTrue(a),
                    (IsWhat::False, false) => Sql::IsFalse(a),
                    (IsWhat::False, true) => Sql::IsNotFalse(a),
                    (IsWhat::Unknown, false) => Sql::IsUnknown(a),
                    (IsWhat::Unknown, true) => Sql::IsNotUnknown(a),
                }
            },
            Expr::Between { e, lo, hi, not, symmetric } => {
                let c = kids(self, &[e, lo, hi]);
                let c = [c[0], c[1], c[2]];
                match (not, symmetric) {
                    (false, false) => Sql::Between(c),
                    (true, false) => Sql::NotBetween(c),
                    (false, true) => Sql::BetweenSym(c),
                    (true, true) => Sql::NotBetweenSym(c),
                }
            },
            Expr::InList(a, xs, not) => {
                let mut c = vec![self.expr(a)];
                c.extend(xs.iter().map(|x| self.expr(x)));
                if *not { Sql::NotIn(c.into()) } else { Sql::In(c.into()) }
            },
            Expr::Like { e, pattern, not, insensitive } => {
                let p = self.leaf(Payload::LikePattern { pattern: pattern.clone(), not: *not, insensitive: *insensitive });
                Sql::Like([p, self.expr(e)])
            },
            Expr::Regex(a, k) => {
                let p = self.leaf(Payload::RegexIndex(*k));
                Sql::Regex([p, self.expr(a)])
            },
            Expr::Arith(op, a, b) => {
                let c = kids(self, &[a, b]);
                let c = [c[0], c[1]];
                match op {
                    Arith::Add => Sql::Add(c),
                    Arith::Sub => Sql::Sub(c),
                    Arith::Mul => Sql::Mul(c),
                    Arith::Div => Sql::Div(c),
                    Arith::Mod => Sql::Mod(c),
                }
            },
            Expr::Neg(a) => Sql::Neg([self.expr(a)]),
            Expr::Concat(a, b) => {
                let c = kids(self, &[a, b]);
                Sql::Concat([c[0], c[1]])
            },
            Expr::Case { operand, whens, otherwise } => {
                let mut c = vec![match operand {
                    Some(o) => self.expr(o),
                    None => self.none(),
                }];
                for (w, t) in whens {
                    c.push(self.expr(w));
                    c.push(self.expr(t));
                }
                c.push(match otherwise {
                    Some(o) => self.expr(o),
                    None => self.none(),
                });
                Sql::Case(c.into())
            },
            Expr::Coalesce(xs) => Sql::Coalesce(xs.iter().map(|x| self.expr(x)).collect()),
            Expr::Greatest(xs) => Sql::Greatest(xs.iter().map(|x| self.expr(x)).collect()),
            Expr::Least(xs) => Sql::Least(xs.iter().map(|x| self.expr(x)).collect()),
            Expr::NullIf(a, b) => {
                let c = kids(self, &[a, b]);
                Sql::NullIf([c[0], c[1]])
            },
            Expr::Cast(a, t) => {
                let p = self.leaf(Payload::CastType(*t));
                Sql::Cast([p, self.expr(a)])
            },
            Expr::Func(f, xs) => {
                let mut c = vec![self.leaf(Payload::Func(*f))];
                c.extend(xs.iter().map(|x| self.expr(x)));
                Sql::Call(c.into())
            },
            Expr::Agg(f, d, a) => {
                let mut c = vec![self.leaf(Payload::Agg(*f, *d))];
                if let Some(a) = a {
                    c.push(self.expr(a));
                }
                Sql::Agg(c.into())
            },
            Expr::Exists(q, not) => {
                let s = self.sub(q);
                if *not { Sql::NotExists([s]) } else { Sql::Exists([s]) }
            },
            Expr::InQuery(a, q, not) => {
                let s = self.sub(q);
                let a = self.expr(a);
                if *not { Sql::NotInSub([s, a]) } else { Sql::InSub([s, a]) }
            },
            Expr::Scalar(q) => Sql::Scalar([self.sub(q)]),
            Expr::Window(_) => return self.leaf(Payload::Opaque(e.clone())),
        };
        self.term.add(node)
    }
}

pub fn encode(q: &Query) -> Encoded {
    let mut enc = Encoder { term: RecExpr::default(), payloads: Vec::new(), keys: Vec::new(), slots: Vec::new() };
    let (skeleton, range) = enc.query(q);
    let slots: Vec<Id> = enc.slots.iter().map(|s| s.expect("every slot encoded")).collect();
    enc.term.add(Sql::Q(slots.into()));
    Encoded { term: enc.term, payloads: enc.payloads, skeleton, top_slots: range.len() }
}

/// A drawn term back to a query, or why not (a shape the AST has no form
/// for, which a rule should never make).
pub fn decode(term: &RecExpr<Sql>, enc: &Encoded) -> Result<Query, String> {
    let root = term.as_ref().last().ok_or("an empty term")?;
    let Sql::Q(slots) = root else { return Err("the root is not q".into()) };
    if slots.len() != enc.term.as_ref().last().map_or(0, |r| match r {
        Sql::Q(s) => s.len(),
        _ => 0,
    }) {
        return Err("a different number of slots".into());
    }
    let d = Decoder { term, enc, slots };
    d.query(&enc.skeleton, 0..enc.top_slots)
}

struct Decoder<'a> {
    term: &'a RecExpr<Sql>,
    enc: &'a Encoded,
    slots: &'a [Id],
}

impl Decoder<'_> {
    fn query(&self, skeleton: &Query, range: std::ops::Range<usize>) -> Result<Query, String> {
        let mut q = skeleton.clone();
        let decoded: Vec<Expr> = range.map(|k| self.expr(self.slots[k])).collect::<Result<_, _>>()?;
        let mut it = decoded.into_iter();
        for slot in slots_mut(&mut q) {
            *slot = it.next().ok_or("too few slot expressions")?;
        }
        Ok(q)
    }

    fn payload(&self, id: Id) -> Result<&Payload, String> {
        match &self.term[id] {
            Sql::Symbol(s) => {
                let k: usize = s.as_str().strip_prefix('#').and_then(|k| k.split(':').next()?.parse().ok()).ok_or_else(|| format!("symbol {s} where a payload was expected"))?;
                self.enc.payloads.get(k).ok_or_else(|| "a payload out of range".to_string())
            },
            other => Err(format!("{other} where a payload was expected")),
        }
    }

    fn is_none(&self, id: Id) -> bool {
        matches!(&self.term[id], Sql::Symbol(s) if s.as_str() == "none")
    }

    fn sub(&self, id: Id) -> Result<Box<Query>, String> {
        match self.payload(id)? {
            Payload::Sub(skeleton, range) => Ok(Box::new(self.query(skeleton, range.clone())?)),
            p => Err(format!("{p:?} where a subquery was expected")),
        }
    }

    fn expr(&self, id: Id) -> Result<Expr, String> {
        let e = |i: &Id| self.expr(*i).map(Box::new);
        let cmp = |op: CmpOp, c: &[Id; 2]| -> Result<Expr, String> { Ok(Expr::Cmp(op, e(&c[0])?, e(&c[1])?)) };
        let is = |w: IsWhat, not: bool, c: &[Id; 1]| -> Result<Expr, String> { Ok(Expr::Is(e(&c[0])?, w, not)) };
        let between = |c: &[Id; 3], not: bool, symmetric: bool| -> Result<Expr, String> {
            Ok(Expr::Between { e: e(&c[0])?, lo: e(&c[1])?, hi: e(&c[2])?, not, symmetric })
        };
        let arith = |op: Arith, c: &[Id; 2]| -> Result<Expr, String> { Ok(Expr::Arith(op, e(&c[0])?, e(&c[1])?)) };
        let list = |c: &[Id]| -> Result<Vec<Expr>, String> { c.iter().map(|i| self.expr(*i)).collect() };
        Ok(match &self.term[id] {
            Sql::Q(_) => return Err("q below the root".into()),
            Sql::Symbol(_) => match self.payload(id)? {
                Payload::Col(c) => Expr::Col(*c),
                Payload::Lit(v, t) => Expr::Lit(v.clone(), *t),
                Payload::Opaque(x) => x.clone(),
                p => return Err(format!("{p:?} as an expression")),
            },
            Sql::And(c) => Expr::And(e(&c[0])?, e(&c[1])?),
            Sql::Or(c) => Expr::Or(e(&c[0])?, e(&c[1])?),
            Sql::Not(c) => Expr::Not(e(&c[0])?),
            Sql::Eq(c) => cmp(CmpOp::Eq, c)?,
            Sql::Ne(c) => cmp(CmpOp::Ne, c)?,
            Sql::Lt(c) => cmp(CmpOp::Lt, c)?,
            Sql::Le(c) => cmp(CmpOp::Le, c)?,
            Sql::Gt(c) => cmp(CmpOp::Gt, c)?,
            Sql::Ge(c) => cmp(CmpOp::Ge, c)?,
            Sql::IsDistinct(c) => Expr::Distinct(false, e(&c[0])?, e(&c[1])?),
            Sql::IsNotDistinct(c) => Expr::Distinct(true, e(&c[0])?, e(&c[1])?),
            Sql::IsNull(c) => Expr::IsNull(e(&c[0])?, false),
            Sql::IsNotNull(c) => Expr::IsNull(e(&c[0])?, true),
            Sql::IsTrue(c) => is(IsWhat::True, false, c)?,
            Sql::IsNotTrue(c) => is(IsWhat::True, true, c)?,
            Sql::IsFalse(c) => is(IsWhat::False, false, c)?,
            Sql::IsNotFalse(c) => is(IsWhat::False, true, c)?,
            Sql::IsUnknown(c) => is(IsWhat::Unknown, false, c)?,
            Sql::IsNotUnknown(c) => is(IsWhat::Unknown, true, c)?,
            Sql::Between(c) => between(c, false, false)?,
            Sql::NotBetween(c) => between(c, true, false)?,
            Sql::BetweenSym(c) => between(c, false, true)?,
            Sql::NotBetweenSym(c) => between(c, true, true)?,
            Sql::In(c) | Sql::NotIn(c) => {
                if c.len() < 2 {
                    return Err("an empty IN list".into());
                }
                Expr::InList(e(&c[0])?, list(&c[1..])?, matches!(self.term[id], Sql::NotIn(_)))
            },
            Sql::Like([p, x]) => match self.payload(*p)? {
                Payload::LikePattern { pattern, not, insensitive } => Expr::Like { e: e(x)?, pattern: pattern.clone(), not: *not, insensitive: *insensitive },
                p => return Err(format!("{p:?} as a pattern")),
            },
            Sql::Regex([p, x]) => match self.payload(*p)? {
                Payload::RegexIndex(k) => Expr::Regex(e(x)?, *k),
                p => return Err(format!("{p:?} as a regex")),
            },
            Sql::Add(c) => arith(Arith::Add, c)?,
            Sql::Sub(c) => arith(Arith::Sub, c)?,
            Sql::Mul(c) => arith(Arith::Mul, c)?,
            Sql::Div(c) => arith(Arith::Div, c)?,
            Sql::Mod(c) => arith(Arith::Mod, c)?,
            Sql::Neg(c) => Expr::Neg(e(&c[0])?),
            Sql::Concat(c) => Expr::Concat(e(&c[0])?, e(&c[1])?),
            Sql::Case(c) => {
                if c.len() < 4 || c.len() % 2 != 0 {
                    return Err("a CASE of the wrong shape".into());
                }
                let operand = if self.is_none(c[0]) { None } else { Some(e(&c[0])?) };
                let last = c[c.len() - 1];
                let otherwise = if self.is_none(last) { None } else { Some(e(&last)?) };
                let whens = c[1..c.len() - 1].chunks(2).map(|wt| Ok((self.expr(wt[0])?, self.expr(wt[1])?))).collect::<Result<_, String>>()?;
                Expr::Case { operand, whens, otherwise }
            },
            Sql::Coalesce(c) => Expr::Coalesce(list(c)?),
            Sql::Greatest(c) => Expr::Greatest(list(c)?),
            Sql::Least(c) => Expr::Least(list(c)?),
            Sql::NullIf(c) => Expr::NullIf(e(&c[0])?, e(&c[1])?),
            Sql::Cast([p, x]) => match self.payload(*p)? {
                Payload::CastType(t) => Expr::Cast(e(x)?, *t),
                p => return Err(format!("{p:?} as a cast type")),
            },
            Sql::Call(c) => match self.payload(c[0])? {
                Payload::Func(f) => Expr::Func(*f, list(&c[1..])?),
                p => return Err(format!("{p:?} as a function")),
            },
            Sql::Agg(c) => match self.payload(c[0])? {
                Payload::Agg(f, d) => Expr::Agg(*f, *d, c.get(1).map(e).transpose()?),
                p => return Err(format!("{p:?} as an aggregate")),
            },
            Sql::Exists([s]) => Expr::Exists(self.sub(*s)?, false),
            Sql::NotExists([s]) => Expr::Exists(self.sub(*s)?, true),
            Sql::Scalar([s]) => Expr::Scalar(self.sub(*s)?),
            Sql::InSub([s, a]) => Expr::InQuery(e(a)?, self.sub(*s)?, false),
            Sql::NotInSub([s, a]) => Expr::InQuery(e(a)?, self.sub(*s)?, true),
        })
    }
}

/// The type of an e-class where it is evident from what it holds: a leaf's
/// (in its symbol), arithmetic's ([`arith_type`]), a cast's, a CASE's or
/// coalesce's first branch, a comparison's (bool). Rules that hold only for
/// some types ask it.
#[derive(Default, Clone)]
pub struct Types;

fn type_named(s: &str) -> Option<Type> {
    let name = s.split_once(':')?.1;
    Type::ALL.into_iter().find(|t| t.name() == name)
}

impl Analysis<Sql> for Types {
    type Data = Option<Type>;

    fn make(eg: &mut EGraph<Sql, Self>, n: &Sql, _id: Id) -> Option<Type> {
        let t = |id: &Id| eg[*id].data;
        let arith = |op: Arith, c: &[Id; 2]| arith_type(op, t(&c[0])?, t(&c[1])?);
        match n {
            Sql::Symbol(s) => type_named(s.as_str()),
            Sql::Add(c) => arith(Arith::Add, c),
            Sql::Sub(c) => arith(Arith::Sub, c),
            Sql::Mul(c) => arith(Arith::Mul, c),
            Sql::Div(c) => arith(Arith::Div, c),
            Sql::Mod(c) => arith(Arith::Mod, c),
            Sql::Neg([a]) => t(a),
            Sql::Cast([p, _]) => t(p),
            Sql::Case(c) => t(&c[2]),
            Sql::Coalesce(c) | Sql::Greatest(c) | Sql::Least(c) => t(&c[0]),
            Sql::NullIf([a, _]) => t(a),
            Sql::Q(_) | Sql::Call(_) | Sql::Agg(_) | Sql::Concat(_) | Sql::Scalar(_) => None,
            _ => Some(Type::Bool),
        }
    }

    fn merge(&mut self, a: &mut Option<Type>, b: Option<Type>) -> DidMerge {
        if a.is_none() && b.is_some() {
            *a = b;
            DidMerge(true, false)
        } else {
            DidMerge(false, a.is_some() && b.is_none())
        }
    }
}

/// Neither operand is a date (`date + int4` has no commuted form in
/// ephyra-postgresql's type checker), and both have a known type.
fn commutes(a: &'static str, b: &'static str) -> impl Fn(&mut EGraph<Sql, Types>, Id, &egg::Subst) -> bool {
    let (a, b): (egg::Var, egg::Var) = (a.parse().unwrap(), b.parse().unwrap());
    move |eg, _, subst| {
        let ok = |v: egg::Var| matches!(eg[subst[v]].data, Some(t) if t != Type::Date);
        ok(a) && ok(b)
    }
}

/// The expression rules (module docs). Each name says what it rewrites.
pub fn rules() -> Vec<Rewrite<Sql, Types>> {
    let mut rs: Vec<Rewrite<Sql, Types>> = vec![
        // Commutativity and associativity of the connectives (3VL AND and OR
        // are commutative and associative).
        rw!("and-comm"; "(and ?a ?b)" => "(and ?b ?a)"),
        rw!("or-comm"; "(or ?a ?b)" => "(or ?b ?a)"),
        rw!("and-assoc"; "(and ?a (and ?b ?c))" => "(and (and ?a ?b) ?c)"),
        rw!("or-assoc"; "(or ?a (or ?b ?c))" => "(or (or ?a ?b) ?c)"),
        rw!("and-idem"; "(and ?a ?a)" => "?a"),
        rw!("or-idem"; "(or ?a ?a)" => "?a"),
        rw!("and-absorb"; "(and ?a (or ?a ?b))" => "?a"),
        rw!("or-absorb"; "(or ?a (and ?a ?b))" => "?a"),
        rw!("not-not"; "(not (not ?a))" => "?a"),
        // De Morgan, both ways (NOT NULL is NULL, so 3VL keeps them).
        rw!("not-and"; "(not (and ?a ?b))" => "(or (not ?a) (not ?b))"),
        rw!("or-not"; "(or (not ?a) (not ?b))" => "(not (and ?a ?b))"),
        rw!("not-or"; "(not (or ?a ?b))" => "(and (not ?a) (not ?b))"),
        rw!("and-not"; "(and (not ?a) (not ?b))" => "(not (or ?a ?b))"),
        // Comparisons: symmetric ones commute, the others flip; a negated
        // comparison is its complement (every type here is totally ordered,
        // NaN largest; on NULL both sides are NULL).
        rw!("eq-comm"; "(= ?a ?b)" => "(= ?b ?a)"),
        rw!("ne-comm"; "(<> ?a ?b)" => "(<> ?b ?a)"),
        rw!("lt-flip"; "(< ?a ?b)" => "(> ?b ?a)"),
        rw!("gt-flip"; "(> ?a ?b)" => "(< ?b ?a)"),
        rw!("le-flip"; "(<= ?a ?b)" => "(>= ?b ?a)"),
        rw!("ge-flip"; "(>= ?a ?b)" => "(<= ?b ?a)"),
        rw!("not-eq"; "(not (= ?a ?b))" => "(<> ?a ?b)"),
        rw!("not-ne"; "(not (<> ?a ?b))" => "(= ?a ?b)"),
        rw!("not-lt"; "(not (< ?a ?b))" => "(>= ?a ?b)"),
        rw!("not-le"; "(not (<= ?a ?b))" => "(> ?a ?b)"),
        rw!("not-gt"; "(not (> ?a ?b))" => "(<= ?a ?b)"),
        rw!("not-ge"; "(not (>= ?a ?b))" => "(< ?a ?b)"),
        rw!("ne-not"; "(<> ?a ?b)" => "(not (= ?a ?b))"),
        rw!("distinct-comm"; "(is-distinct ?a ?b)" => "(is-distinct ?b ?a)"),
        rw!("not-distinct-comm"; "(is-not-distinct ?a ?b)" => "(is-not-distinct ?b ?a)"),
        rw!("not-distinct"; "(not (is-distinct ?a ?b))" => "(is-not-distinct ?a ?b)"),
        rw!("not-not-distinct"; "(not (is-not-distinct ?a ?b))" => "(is-distinct ?a ?b)"),
        // BETWEEN is its two comparisons (PostgreSQL defines it so).
        rw!("between"; "(between ?e ?l ?h)" => "(and (>= ?e ?l) (<= ?e ?h))"),
        rw!("between-back"; "(and (>= ?e ?l) (<= ?e ?h))" => "(between ?e ?l ?h)"),
        rw!("not-between"; "(not-between ?e ?l ?h)" => "(or (< ?e ?l) (> ?e ?h))"),
        rw!("not-between-not"; "(not-between ?e ?l ?h)" => "(not (between ?e ?l ?h))"),
        rw!("not-between-back"; "(not (between ?e ?l ?h))" => "(not-between ?e ?l ?h)"),
        // NULL tests and the IS family on booleans.
        rw!("is-not-null"; "(is-not-null ?a)" => "(not (is-null ?a))"),
        rw!("is-not-null-back"; "(not (is-null ?a))" => "(is-not-null ?a)"),
        rw!("is-not-true"; "(is-not-true ?a)" => "(not (is-true ?a))"),
        rw!("is-not-true-back"; "(not (is-true ?a))" => "(is-not-true ?a)"),
        rw!("is-not-false"; "(is-not-false ?a)" => "(not (is-false ?a))"),
        rw!("is-not-false-back"; "(not (is-false ?a))" => "(is-not-false ?a)"),
        rw!("is-false-not"; "(is-false ?a)" => "(is-true (not ?a))"),
        rw!("is-true-not"; "(is-true (not ?a))" => "(is-false ?a)"),
        rw!("is-unknown"; "(is-unknown ?a)" => "(is-null ?a)"),
        rw!("is-not-unknown"; "(is-not-unknown ?a)" => "(is-not-null ?a)"),
        rw!("is-true-or-unknown"; "(is-not-false ?a)" => "(or (is-true ?a) (is-unknown ?a))"),
        // Commutative arithmetic: integer, numeric and real + and * are
        // commutative (overflow either way alike); reals are not
        // associative, so no reassociation.
        rw!("add-comm"; "(+ ?a ?b)" => "(+ ?b ?a)" if commutes("?a", "?b")),
        rw!("mul-comm"; "(* ?a ?b)" => "(* ?b ?a)" if commutes("?a", "?b")),
        // nullif and coalesce as CASE.
        rw!("nullif-case"; "(nullif ?a ?b)" => "(case none (is-not-true (= ?a ?b)) ?a none)"),
        rw!("coalesce2-case"; "(coalesce ?a ?b)" => "(case none (is-not-null ?a) ?a ?b)"),
        rw!("coalesce3-case"; "(coalesce ?a ?b ?c)" => "(case none (is-not-null ?a) ?a (coalesce ?b ?c))"),
        // A CASE with one WHEN: a simple CASE is a searched one on `=`; a
        // searched one swaps its branches on `IS NOT TRUE` (NULL goes to
        // ELSE either way).
        rw!("simple-case"; "(case ?o ?v ?t ?e)" => "(case none (= ?o ?v) ?t ?e)" if not_none("?o")),
        rw!("case-swap"; "(case none ?p ?t ?e)" => "(case none (is-not-true ?p) ?e ?t)" if not_none("?e")),
        rw!("greatest-comm"; "(greatest ?a ?b)" => "(greatest ?b ?a)"),
        rw!("least-comm"; "(least ?a ?b)" => "(least ?b ?a)"),
    ];
    // IN lists of 1 to 4 items as ORs of equalities, NOT IN as ANDs of
    // inequalities (PostgreSQL defines them so, NULLs included).
    for n in 1..=4 {
        let items: Vec<String> = (0..n).map(|k| format!("?x{k}")).collect();
        let chain = |op: &str, join: &str| -> String {
            let mut s = format!("({op} ?e {})", items[n - 1]);
            for x in items[..n - 1].iter().rev() {
                s = format!("({join} ({op} ?e {x}) {s})");
            }
            s
        };
        let list = items.join(" ");
        let pin: egg::Pattern<Sql> = format!("(in ?e {list})").parse().unwrap();
        let por: egg::Pattern<Sql> = chain("=", "or").parse().unwrap();
        let pnin: egg::Pattern<Sql> = format!("(not-in ?e {list})").parse().unwrap();
        let pand: egg::Pattern<Sql> = chain("<>", "and").parse().unwrap();
        rs.push(Rewrite::new(format!("in{n}-or"), pin.clone(), por.clone()).unwrap());
        if n > 1 {
            rs.push(Rewrite::new(format!("or-in{n}"), por, pin).unwrap());
        }
        rs.push(Rewrite::new(format!("not-in{n}-and"), pnin.clone(), pand.clone()).unwrap());
        if n > 1 {
            rs.push(Rewrite::new(format!("and-not-in{n}"), pand, pnin).unwrap());
        }
    }
    rs
}

fn not_none(var: &'static str) -> impl Fn(&mut egg::EGraph<Sql, Types>, Id, &egg::Subst) -> bool {
    let v: egg::Var = var.parse().unwrap();
    move |eg, _, subst| !eg[subst[v]].nodes.iter().any(|n| matches!(n, Sql::Symbol(s) if s.as_str() == "none"))
}
