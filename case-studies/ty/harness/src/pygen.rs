// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! A generator of typed, terminating Python programs.
//!
//! Programs are well-typed by construction under a small model of Python's
//! type system (`Ty`, `Model::sub`), deliberately coarser than the spec: a
//! model judgement must imply the real one, never the other way round. The
//! output is a `Tree` of lines (`tree.rs`), so shrinking can drop and hoist
//! blocks without knowing Python.
//!
//! Every function is called from `main()` with generated arguments, so union
//! parameters see several runtime types. `_probe(k, e)` lines record the
//! runtime type of `e` (`py/_rt.py`); the check file asserts each observed
//! type is assignable to ty's `TypeOf[e]` at that point.

use ephyra::Source;

use crate::tree::{Tree, node};

#[derive(Clone, Debug, PartialEq)]
pub enum Ty {
    Int,
    Bool,
    Str,
    Float,
    Bytes,
    None,
    Object,
    Tuple(Vec<Ty>),
    /// `tuple[T, ...]`
    TupleVar(Box<Ty>),
    List(Box<Ty>),
    Dict(Box<Ty>, Box<Ty>),
    Set(Box<Ty>),
    Union(Vec<Ty>),
    /// A user class, with type arguments when it is generic.
    Class(usize, Vec<Ty>),
    Enum(usize),
    Proto(usize),
    TypedDict(usize),
    Callable(Vec<Ty>, Box<Ty>),
    /// A type parameter of the enclosing generic function.
    Var(String),
    /// `Literal[<repr>]` of an int or str value (never 0/1, so no bool equals it).
    Lit(String),
    /// `type[C]` of a non-generic user class.
    Type(usize),
}

impl Ty {
    fn opt(t: Ty) -> Ty {
        Ty::Union(vec![t, Ty::None])
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ClassKind {
    Plain,
    Dataclass { frozen: bool, slots: bool, kw_only: bool },
    NamedTuple,
    /// A pydantic `BaseModel` (settings in `Class::pyd`).
    Pydantic,
}

/// Pydantic model settings (dedicated/pydantic.rs in ty). Config keys are inherited by
/// subclasses and overridden per key; `None` is "not set here".
#[derive(Clone, Debug, Default)]
pub struct Pyd {
    pub frozen: Option<bool>,
    /// `extra=`: "allow", "forbid", "ignore".
    pub extra: Option<&'static str>,
    /// `(validate_by_name, validate_by_alias, populate_by_name)` as written.
    pub by_name: (Option<bool>, Option<bool>, Option<bool>),
    pub strict: Option<bool>,
    /// 0: `model_config = ConfigDict(...)`, 1: class keywords, 2: `model_config = {...}`.
    pub cfg_style: u8,
    /// Own fields with an alias (`<field>_al`), as `Field(alias=)` or in `Annotated`.
    pub aliases: Vec<String>,
    /// Own fields with a default: 0 `= v`, 1 `= Field(default=v)`, 2 `= Field(v)`.
    pub defaults: Vec<(String, u8)>,
    /// Own fields with `Field(frozen=True)`.
    pub frozen_fields: Vec<String>,
    /// Own fields with `Field(strict=)`/`Annotated[T, Strict(..)]`.
    pub strict_fields: Vec<(String, bool)>,
    /// A root model that is a `pydantic_settings.BaseSettings` (config in `SettingsConfigDict`).
    pub settings: bool,
}

#[derive(Clone, Debug)]
pub struct Method {
    pub name: String,
    pub params: Vec<Ty>,
    pub ret: Ty,
    pub kind: MethodKind,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MethodKind {
    Normal,
    /// `def m(self) -> Self: ... return self` (`ret` is the defining class).
    SelfRet,
    Property,
    Static,
    Class,
}

#[derive(Clone, Debug)]
pub struct Class {
    pub name: String,
    pub base: Option<usize>,
    pub kind: ClassKind,
    /// Type parameter names (generic classes only use them as field types).
    pub tparams: Vec<String>,
    /// Own fields (the base's come first in the constructor).
    pub fields: Vec<(String, Ty)>,
    pub methods: Vec<Method>,
    pub protos: Vec<usize>,
    pub is_abstract: bool,
    /// Dataclasses: the last own field (a list/dict/set) is `field(default_factory=...)`.
    pub factory_last: bool,
    /// Dataclasses: `iv: InitVar[int] = 0` consumed by `__post_init__(self, iv)`.
    pub initvar: bool,
    pub post_init: bool,
    /// Root plain classes: `__init_subclass__(cls, tag: str = "", **kw)`; subclasses pass `tag=`.
    pub init_sub: bool,
    /// Plain classes only: extra attributes (name, type, kind).
    pub extras: Vec<(String, Ty, Extra)>,
    pub slots: bool,
    pub pyd: Pyd,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Extra {
    /// A property with a setter over the named field.
    Setter(String),
    ClassVar,
    /// An instance of descriptor class `D<k>`.
    Descriptor(usize),
    /// A pydantic private attribute (`_pv<k>: T = v`, not a field; writable even when frozen).
    Private,
}

#[derive(Clone, Debug)]
pub struct Enum {
    pub name: String,
    pub members: Vec<String>,
    pub str_values: bool,
    pub flavour: &'static str,
    /// `E = enum.Enum("E", ...)`
    pub functional: bool,
}

#[derive(Clone, Debug)]
pub struct Proto {
    pub name: String,
    /// Method name and return type (no parameters).
    pub methods: Vec<(String, Ty)>,
    /// Data members: name, type, read-only (declared as a property).
    pub attrs: Vec<(String, Ty, bool)>,
    pub runtime: bool,
}

#[derive(Clone, Debug)]
pub struct TypedDictDef {
    pub name: String,
    /// Own fields: name, type, required.
    pub fields: Vec<(String, Ty, bool)>,
    pub base: Option<usize>,
    pub total: bool,
    pub readonly: Vec<String>,
    pub functional: bool,
}

#[derive(Clone, Debug)]
pub struct Func {
    pub name: String,
    pub tparams: Vec<(String, Option<Ty>)>,
    pub params: Vec<(String, Ty)>,
    pub ret: Ty,
    /// `def f(x: object) -> TypeIs[T]` / `TypeGuard[T]`
    pub guard: Option<(Ty, bool)>,
    pub sig: Sig,
}

/// Signature extras of a non-generic function (or the type parameter's form).
#[derive(Clone, Debug, Default)]
pub struct Sig {
    /// The last `kwonly` params are keyword-only.
    pub kwonly: usize,
    /// The first `posonly` params are positional-only (`/`).
    pub posonly: usize,
    /// Default value expressions, per param.
    pub defaults: Vec<Option<String>>,
    /// `*args: T`
    pub star: Option<Ty>,
    /// `**kwargs: T`
    pub kwargs: Option<Ty>,
    /// `**kwargs: Unpack[TD<k>]`
    pub unpack: Option<usize>,
    /// `@overload` variants: (param type, return type); the function has one param.
    pub overloads: Vec<(Ty, Ty)>,
    /// Type parameter as `T<k> = TypeVar(...)` instead of PEP 695 syntax.
    pub legacy: bool,
    pub constraints: Vec<Ty>,
    pub tdefault: Option<Ty>,
    /// A ParamSpec-preserving decorator (`deco_pep695` / `deco_legacy`).
    pub deco: Option<&'static str>,
    /// The body (transitively) runs `asyncio.run`: never called inside a coroutine.
    pub has_async: bool,
}

#[derive(Clone, Debug)]
pub struct Alias {
    pub name: String,
    pub tparam: Option<String>,
    pub target: Ty,
    /// 0: `type A = ...`, 1: `A: TypeAlias = ...`, 2: `A = TypeAliasType("A", ...)`
    pub style: u8,
}

/// Matches `t` against an alias pattern with type parameter `v`, binding it.
fn unify(pat: &Ty, t: &Ty, v: &str, out: &mut Option<Ty>) -> bool {
    match (pat, t) {
        (Ty::Var(n), _) if n == v => match out {
            Some(b) => b == t,
            None => {
                *out = Some(t.clone());
                true
            }
        },
        (Ty::Tuple(a), Ty::Tuple(b)) | (Ty::Union(a), Ty::Union(b)) => a.len() == b.len() && a.iter().zip(b).all(|(x, y)| unify(x, y, v, out)),
        (Ty::TupleVar(a), Ty::TupleVar(b)) | (Ty::List(a), Ty::List(b)) | (Ty::Set(a), Ty::Set(b)) => unify(a, b, v, out),
        (Ty::Dict(a, b), Ty::Dict(c, d)) => unify(a, c, v, out) && unify(b, d, v, out),
        _ => pat == t,
    }
}

pub struct Model {
    pub classes: Vec<Class>,
    pub enums: Vec<Enum>,
    pub protos: Vec<Proto>,
    pub tds: Vec<TypedDictDef>,
    pub funcs: Vec<Func>,
    /// Type aliases: name, type parameter (generic aliases have one), target, style.
    pub aliases: Vec<Alias>,
    /// Generator functions `gen<k>(n: int) -> Iterator[T]`: name, element type, return annotation style.
    pub gens: Vec<(String, Ty, u8)>,
    /// Descriptor classes `D<k>` with `__get__`/`__set__` of this value type.
    pub descs: Vec<Ty>,
    /// User iterable classes `It<k>(xs: list[T])`: name, element type, protocol style.
    pub iters: Vec<(String, Ty, u8)>,
    /// Async generators `agen<k>(n) -> AsyncIterator[T]`: name, element type, annotation style.
    pub agens: Vec<(String, Ty, u8)>,
    /// User async iterable classes `AIt<k>(xs: list[T])`: name, element type.
    pub aiters: Vec<(String, Ty)>,
    /// User sequence classes `Sq<k>(xs: list[T])`: overloaded `__getitem__` (int / slice),
    /// `__setitem__`, `__len__`, `__getattr__`, `__class_getitem__`.
    pub seqs: Vec<(String, Ty)>,
    /// Generating a coroutine body (or a lambda): no calls of functions running `asyncio.run`.
    in_async: bool,
    /// The current function body runs (or calls something running) `asyncio.run`.
    async_used: bool,
    /// Alternates alias / plain / `Annotated` renderings.
    rcount: std::cell::Cell<u64>,
    /// Set while rendering an alias target (no alias inside its own target).
    no_alias: std::cell::Cell<bool>,
    /// Set while rendering inside a string annotation (no nested quotes).
    quoted: std::cell::Cell<bool>,
    /// Generating method bodies: no method calls (a subclass override could recurse).
    in_methods: bool,
    probe: usize,
    fresh: usize,
    nesting: usize,
}

/// One binding visible to generated code: an expression path (a name, or
/// `o.f`, or `t[0]` after narrowing) and its model type.
#[derive(Clone, Debug)]
struct Var {
    path: String,
    ty: Ty,
    /// A narrowed view (`Scope::narrow`), not a binding.
    narrowed: bool,
    /// Assigned again later in this scope: never narrowed in the model.
    mutable: bool,
}

#[derive(Clone, Default)]
struct Scope {
    vars: Vec<Var>,
    /// Paths that must not be assigned in this block (they are narrowed).
    frozen: Vec<String>,
    in_loop: bool,
    /// Type parameters of the enclosing generic function.
    tvars: Vec<(String, Option<Ty>)>,
    /// Inside a `for` over a container: no container mutation.
    iterating: bool,
    /// Names captured by a nested function: no attribute or key writes through them
    /// (ty keeps the caller's narrowing of their attributes across the call).
    captured: Vec<String>,
}

thread_local! {
    /// The model type of every binding made so far, by path. Names are fresh
    /// except parameters and `self.<field>`, and the latest binding wins, which is
    /// the one in scope while its function is generated.
    static BOUND: std::cell::RefCell<std::collections::HashMap<String, Ty>> = Default::default();
}

impl Scope {
    fn bind(&mut self, path: impl Into<String>, ty: Ty, mutable: bool) {
        let path = path.into();
        BOUND.with(|b| b.borrow_mut().insert(path.clone(), ty.clone()));
        self.vars.push(Var { path, ty, mutable, narrowed: false });
    }

    fn lookup(&self, path: &str) -> Option<&Var> {
        self.vars.iter().rev().find(|v| v.path == path)
    }
    fn narrow(&self, path: &str, ty: Ty) -> Scope {
        let mut s = self.clone();
        s.vars.push(Var { path: path.to_string(), ty, mutable: false, narrowed: true });
        let root = path.split(['.', '[']).next().unwrap_or(path).to_string();
        s.frozen.push(path.to_string());
        s.frozen.push(root);
        s
    }
}

fn join(parts: impl IntoIterator<Item = String>, sep: &str) -> String {
    parts.into_iter().collect::<Vec<_>>().join(sep)
}

impl Model {
    // ----- types --------------------------------------------------------------------------------

    pub fn render(&self, t: &Ty) -> String {
        if !self.no_alias.get() && !self.aliases.is_empty() {
            let n = self.rcount.get();
            self.rcount.set(n + 1);
            if n % 2 == 0 {
                for a in &self.aliases {
                    match &a.tparam {
                        None if a.target == *t => return a.name.clone(),
                        Some(v) => {
                            let mut b = None;
                            if unify(&a.target, t, v, &mut b) {
                                if let Some(b) = b {
                                    return format!("{}[{}]", a.name, self.render(&b));
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            if n % 7 == 3 && matches!(t, Ty::Int | Ty::Str | Ty::Bool | Ty::Bytes) {
                self.no_alias.set(true);
                let r = self.render(t);
                self.no_alias.set(false);
                return format!("Annotated[{r}, \"m{n}\"]");
            }
        }
        self.render_plain(t)
    }

    fn render_plain(&self, t: &Ty) -> String {
        // Sometimes the `typing` special forms: Optional, Union, List, Dict, Tuple, Set.
        let n = self.rcount.get();
        if n % 5 == 1 && !self.no_alias.get() {
            self.rcount.set(n + 1);
            match t {
                Ty::Union(ts) if ts.len() == 2 && ts[1] == Ty::None => return format!("typing.Optional[{}]", self.render(&ts[0])),
                Ty::Union(ts) => return format!("typing.Union[{}]", join(ts.iter().map(|t| self.render(t)), ", ")),
                Ty::List(t) => return format!("typing.List[{}]", self.render(t)),
                Ty::Set(t) => return format!("typing.Set[{}]", self.render(t)),
                Ty::Dict(k, v) => return format!("typing.Dict[{}, {}]", self.render(k), self.render(v)),
                Ty::Tuple(ts) if !ts.is_empty() => return format!("typing.Tuple[{}]", join(ts.iter().map(|t| self.render(t)), ", ")),
                Ty::TupleVar(t) => return format!("typing.Tuple[{}, ...]", self.render(t)),
                _ => {}
            }
        }
        // Sometimes a forward reference (string) for a user type name, a
        // `Literal[a, b]` for a union of literals, `type[A | B]` for class objects.
        if !self.no_alias.get() && !self.quoted.get() {
            match t {
                Ty::Class(_, a) if a.is_empty() && n % 4 == 2 => {
                    self.rcount.set(n + 1);
                    return format!("\"{}\"", self.classes[if let Ty::Class(i, _) = t { *i } else { 0 }].name);
                }
                Ty::Enum(i) if n % 4 == 2 => {
                    self.rcount.set(n + 1);
                    return format!("\"{}\"", self.enums[*i].name);
                }
                Ty::TypedDict(i) if n % 4 == 2 => {
                    self.rcount.set(n + 1);
                    return format!("\"{}\"", self.tds[*i].name);
                }
                _ => {}
            }
        }
        if let Ty::Union(ts) = t {
            if ts.iter().all(|m| matches!(m, Ty::Lit(_))) {
                let vs: Vec<String> = ts.iter().map(|m| if let Ty::Lit(v) = m { v.clone() } else { unreachable!() }).collect();
                return if n % 3 == 0 { format!("Literal[Literal[{}], {}]", vs[0], vs[1..].join(", ")) } else { format!("Literal[{}]", vs.join(", ")) };
            }
            if ts.iter().all(|m| matches!(m, Ty::Type(_))) {
                return format!("type[{}]", join(ts.iter().map(|m| if let Ty::Type(c) = m { self.classes[*c].name.clone() } else { unreachable!() }), " | "));
            }
        }
        match t {
            Ty::Int => "int".into(),
            Ty::Bool => "bool".into(),
            Ty::Str => "str".into(),
            Ty::Float => "float".into(),
            Ty::Bytes => "bytes".into(),
            Ty::None => "None".into(),
            Ty::Object => "object".into(),
            Ty::Tuple(ts) if ts.is_empty() => "tuple[()]".into(),
            Ty::Tuple(ts) => format!("tuple[{}]", join(ts.iter().map(|t| self.render(t)), ", ")),
            Ty::TupleVar(t) => format!("tuple[{}, ...]", self.render(t)),
            Ty::List(t) => format!("list[{}]", self.render(t)),
            Ty::Set(t) => format!("set[{}]", self.render(t)),
            Ty::Dict(k, v) => format!("dict[{}, {}]", self.render(k), self.render(v)),
            Ty::Union(ts) => {
                // `X | "C"` fails at runtime: no forward-reference leaves in a `|` union.
                let q = self.quoted.replace(true);
                let r = join(ts.iter().map(|t| self.render(t)), " | ");
                self.quoted.set(q);
                r
            }
            Ty::Class(i, args) if args.is_empty() => self.classes[*i].name.clone(),
            Ty::Class(i, args) => {
                format!("{}[{}]", self.classes[*i].name, join(args.iter().map(|t| self.render(t)), ", "))
            }
            Ty::Enum(i) => self.enums[*i].name.clone(),
            Ty::Proto(i) => self.protos[*i].name.clone(),
            Ty::TypedDict(i) => self.tds[*i].name.clone(),
            Ty::Callable(ps, r) => {
                format!("Callable[[{}], {}]", join(ps.iter().map(|t| self.render(t)), ", "), self.render(r))
            }
            Ty::Var(n) => n.clone(),
            Ty::Lit(v) => format!("Literal[{v}]"),
            Ty::Type(c) => format!("type[{}]", self.classes[*c].name),
        }
    }

    /// A TypedDict's fields, the base's first.
    fn td_fields(&self, i: usize) -> Vec<(String, Ty, bool)> {
        let td = &self.tds[i];
        let mut out = td.base.map(|b| self.td_fields(b)).unwrap_or_default();
        out.extend(td.fields.iter().cloned());
        out
    }

    fn td_readonly(&self, i: usize, f: &str) -> bool {
        let td = &self.tds[i];
        td.readonly.iter().any(|r| r == f) || td.base.is_some_and(|b| self.td_readonly(b, f))
    }

    fn all_extras(&self, c: usize) -> Vec<(String, Ty, Extra)> {
        self.ancestors(c).iter().flat_map(|&k| self.classes[k].extras.clone()).collect()
    }

    fn ancestors(&self, c: usize) -> Vec<usize> {
        let mut out = vec![c];
        let mut cur = self.classes[c].base;
        while let Some(b) = cur {
            out.push(b);
            cur = self.classes[b].base;
        }
        out
    }

    fn all_fields(&self, c: usize) -> Vec<(String, Ty)> {
        let mut chain = self.ancestors(c);
        chain.reverse();
        chain.iter().flat_map(|&k| self.classes[k].fields.clone()).collect()
    }

    /// A field type pydantic validates without changing what the model says about the value.
    fn pyd_safe(&self, t: &Ty) -> bool {
        match t {
            Ty::Proto(_) | Ty::Var(_) | Ty::TypedDict(_) => false,
            Ty::Union(ts) | Ty::Tuple(ts) => ts.iter().all(|t| self.pyd_safe(t)),
            Ty::TupleVar(t) | Ty::List(t) | Ty::Set(t) => self.pyd_safe(t),
            Ty::Dict(k, v) => self.pyd_safe(k) && self.pyd_safe(v),
            Ty::Callable(ps, r) => ps.iter().all(|t| self.pyd_safe(t)) && self.pyd_safe(r),
            Ty::Class(_, a) => a.is_empty(),
            _ => true,
        }
    }

    /// A field type whose generated values pass pydantic's strict mode (no bool for an int,
    /// no int for a float, no tuple subclass for a tuple).
    fn strict_safe(&self, t: &Ty) -> bool {
        match t {
            Ty::Str | Ty::Bytes | Ty::Lit(_) | Ty::Enum(_) | Ty::None | Ty::Type(_) => true,
            Ty::Class(_, a) => a.is_empty(),
            Ty::Union(ts) => ts.iter().all(|t| self.strict_safe(t)),
            Ty::List(t) => self.strict_safe(t),
            _ => false,
        }
    }

    /// Whether a pydantic field of this type needs `arbitrary_types_allowed` (a plain class).
    fn pyd_arbitrary(&self, t: &Ty) -> bool {
        match t {
            Ty::Class(c, _) => self.classes[*c].kind == ClassKind::Plain,
            Ty::Union(ts) | Ty::Tuple(ts) => ts.iter().any(|t| self.pyd_arbitrary(t)),
            Ty::TupleVar(t) | Ty::List(t) | Ty::Set(t) => self.pyd_arbitrary(t),
            Ty::Dict(k, v) => self.pyd_arbitrary(k) || self.pyd_arbitrary(v),
            Ty::Callable(ps, r) => ps.iter().any(|t| self.pyd_arbitrary(t)) || self.pyd_arbitrary(r),
            _ => false,
        }
    }

    /// The nearest setting of a pydantic config key along `c`'s ancestors.
    fn pyd_cfg<T>(&self, c: usize, f: impl Fn(&Pyd) -> Option<T>) -> Option<T> {
        self.ancestors(c).into_iter().find_map(|k| f(&self.classes[k].pyd))
    }

    fn pyd_frozen_opt(&self, c: usize) -> Option<bool> {
        self.pyd_cfg(c, |p| p.frozen)
    }

    fn pyd_strict_opt(&self, c: usize) -> Option<bool> {
        self.pyd_cfg(c, |p| p.strict)
    }

    /// The field's own `strict=` setting, from the class that declares it.
    fn pyd_field_strict(&self, c: usize, n: &str) -> Option<bool> {
        self.ancestors(c).into_iter().find_map(|k| self.classes[k].pyd.strict_fields.iter().find(|(f, _)| f == n).map(|(_, s)| *s))
    }

    fn pyd_lax(&self, c: usize, n: &str) -> bool {
        !self.pyd_field_strict(c, n).or(self.pyd_strict_opt(c)).unwrap_or(false)
    }

    /// Whether `c.n` can be assigned to (not frozen, model-wide or per field).
    fn pyd_writable(&self, c: usize, n: &str) -> bool {
        !self.pyd_frozen_opt(c).unwrap_or(false) && !self.ancestors(c).iter().any(|&k| self.classes[k].pyd.frozen_fields.iter().any(|f| f == n))
    }

    /// (by alias, by name): how the constructor may name an aliased field.
    fn pyd_by(&self, c: usize) -> (bool, bool) {
        let vbn = self.pyd_cfg(c, |p| p.by_name.0.or(p.by_name.2));
        let by_alias = self.pyd_cfg(c, |p| p.by_name.1).unwrap_or(true);
        (by_alias, vbn.unwrap_or(!by_alias))
    }

    fn pyd_has_alias(&self, c: usize, n: &str) -> bool {
        self.ancestors(c).iter().any(|&k| self.classes[k].pyd.aliases.iter().any(|f| f == n))
    }

    fn pyd_has_default(&self, c: usize, n: &str) -> bool {
        self.ancestors(c).iter().any(|&k| {
            let k = &self.classes[k];
            k.pyd.defaults.iter().any(|(f, _)| f == n) || (k.factory_last && k.fields.last().is_some_and(|(f, _)| f == n))
        })
    }

    /// A value pydantic converts to `t` in lax mode (its conversion table), if there is one.
    fn lax_expr(&mut self, g: &mut Source, t: &Ty, sc: &Scope, d: usize) -> Option<String> {
        let int_enums: Vec<String> = self.enums.iter().filter(|e| !e.str_values).map(|e| format!("{}.{}", e.name, e.members[0])).collect();
        let any_enums: Vec<String> = self.enums.iter().map(|e| format!("{}.{}", e.name, e.members[e.members.len() - 1])).collect();
        let n = g.int(-3..=99i64);
        Some(match t {
            Ty::Int => match g.int(0..=3u8) {
                0 => format!("\"{n}\""),
                1 => format!("b\"{n}\""),
                2 if !int_enums.is_empty() => g.pick(&int_enums).clone(),
                _ => format!("{n}.0"),
            },
            Ty::Float => g.pick(&["\"1.5\"", "b\"-2.25\"", "True", "7", "\"3\""]).to_string(),
            Ty::Str => match g.int(0..=1u8) {
                0 if !any_enums.is_empty() => g.pick(&any_enums).clone(),
                _ => g.pick(&["b\"ab\"", "b\"\"", "bytearray(b\"xy\")"]).to_string(),
            },
            Ty::Bool => g.pick(&["1", "0", "\"true\"", "\"off\"", "b\"yes\"", "1.0"]).to_string(),
            Ty::Bytes => g.pick(&["\"ab\"", "\"\"", "bytearray(b\"q\")"]).to_string(),
            Ty::List(e) => {
                let e = (**e).clone();
                let xs: Vec<String> = (0..g.int(0..=2usize)).map(|_| self.lax_expr(g, &e, sc, d).unwrap_or_else(|| self.expr(g, &e, sc, d + 1))).collect();
                if g.bool() { format!("({}{})", xs.join(", "), if xs.len() == 1 { "," } else { "" }) } else { format!("[{}]", xs.join(", ")) }
            }
            Ty::TupleVar(e) => {
                let e = (**e).clone();
                let xs: Vec<String> = (0..g.int(0..=2usize)).map(|_| self.lax_expr(g, &e, sc, d).unwrap_or_else(|| self.expr(g, &e, sc, d + 1))).collect();
                format!("[{}]", xs.join(", "))
            }
            Ty::Dict(k, v) if **k == Ty::Str => {
                let v = (**v).clone();
                let x = self.lax_expr(g, &v, sc, d)?;
                format!("{{\"k\": {x}}}")
            }
            Ty::Union(ts) => {
                let ts = ts.clone();
                let m = g.pick(&ts).clone();
                return self.lax_expr(g, &m, sc, d);
            }
            _ => return None,
        })
    }

    /// Methods visible on `c`, most derived first, one per name.
    fn all_methods(&self, c: usize) -> Vec<(usize, Method)> {
        let mut out: Vec<(usize, Method)> = Vec::new();
        for k in self.ancestors(c) {
            for m in &self.classes[k].methods {
                if !out.iter().any(|(_, o)| o.name == m.name) {
                    out.push((k, m.clone()));
                }
            }
        }
        out
    }

    /// Protocols with a constructible implementer so far.
    fn inhabited_protos(&self) -> Vec<usize> {
        (0..self.protos.len()).filter(|&p| (0..self.classes.len()).any(|c| self.implements(c, p) && self.classes[c].tparams.is_empty())).collect()
    }

    fn implements(&self, c: usize, p: usize) -> bool {
        self.ancestors(c).iter().any(|&k| self.classes[k].protos.contains(&p))
    }

    /// Substitutes a generic class's type arguments into a member type.
    fn subst(&self, c: usize, args: &[Ty], t: &Ty) -> Ty {
        let tps = &self.classes[c].tparams;
        let go = |t: &Ty| self.subst(c, args, t);
        match t {
            Ty::Var(n) => tps.iter().position(|p| p == n).map(|i| args[i].clone()).unwrap_or(t.clone()),
            Ty::Tuple(ts) => Ty::Tuple(ts.iter().map(go).collect()),
            Ty::TupleVar(t) => Ty::TupleVar(Box::new(go(t))),
            Ty::List(t) => Ty::List(Box::new(go(t))),
            Ty::Set(t) => Ty::Set(Box::new(go(t))),
            Ty::Dict(k, v) => Ty::Dict(Box::new(go(k)), Box::new(go(v))),
            Ty::Union(ts) => Ty::Union(ts.iter().map(go).collect()),
            Ty::Callable(ps, r) => Ty::Callable(ps.iter().map(go).collect(), Box::new(go(r))),
            Ty::Class(i, a) => Ty::Class(*i, a.iter().map(go).collect()),
            _ => t.clone(),
        }
    }

    /// `a <: b` in the model. Sound but incomplete.
    pub fn sub(&self, a: &Ty, b: &Ty) -> bool {
        if a == b {
            return true;
        }
        match (a, b) {
            (_, Ty::Object) => true,
            (Ty::Union(xs), _) => xs.iter().all(|x| self.sub(x, b)),
            (_, Ty::Union(ys)) => ys.iter().any(|y| self.sub(a, y)),
            (Ty::Bool, Ty::Int | Ty::Float) | (Ty::Int, Ty::Float) => true,
            (Ty::Lit(v), Ty::Str) => v.starts_with('"'),
            (Ty::Type(i), Ty::Type(j)) => self.ancestors(*i).contains(j),
            (Ty::Lit(v), Ty::Int | Ty::Float) => !v.starts_with('"'),
            (Ty::Tuple(xs), Ty::Tuple(ys)) => xs.len() == ys.len() && xs.iter().zip(ys).all(|(x, y)| self.sub(x, y)),
            (Ty::Tuple(xs), Ty::TupleVar(y)) => xs.iter().all(|x| self.sub(x, y)),
            (Ty::TupleVar(x), Ty::TupleVar(y)) => self.sub(x, y),
            (Ty::Class(i, xa), Ty::Class(j, ya)) => {
                if xa.is_empty() && ya.is_empty() {
                    self.ancestors(*i).contains(j)
                } else {
                    false
                }
            }
            (Ty::Class(i, xa), Ty::Proto(p)) => xa.is_empty() && self.implements(*i, *p),
            (Ty::Class(i, xa), Ty::Tuple(ys)) => {
                xa.is_empty() && self.classes[*i].kind == ClassKind::NamedTuple && {
                    let fs = self.all_fields(*i);
                    fs.len() == ys.len() && fs.iter().zip(ys).all(|((_, x), y)| self.sub(x, y))
                }
            }
            _ => false,
        }
    }

    fn members(t: &Ty) -> Vec<Ty> {
        match t {
            Ty::Union(ts) => ts.iter().flat_map(Self::members).collect(),
            t => vec![t.clone()],
        }
    }

    fn union_of(ts: Vec<Ty>) -> Ty {
        let mut out: Vec<Ty> = Vec::new();
        for t in ts.into_iter().flat_map(|t| Self::members(&t)) {
            if !out.contains(&t) {
                out.push(t);
            }
        }
        if out.len() == 1 { out.pop().unwrap() } else { Ty::Union(out) }
    }

    /// The type an `isinstance(x, <runtime class>)` check can name, if any.
    fn isinstance_name(&self, t: &Ty) -> Option<String> {
        Some(match t {
            Ty::Int => "int".into(),
            Ty::Bool => "bool".into(),
            Ty::Str => "str".into(),
            Ty::Float => "float".into(),
            Ty::Bytes => "bytes".into(),
            Ty::Lit(v) => if v.starts_with('"') { "str".into() } else { "int".into() },
            Ty::Tuple(_) | Ty::TupleVar(_) => "tuple".into(),
            Ty::List(_) => "list".into(),
            Ty::Dict(..) => "dict".into(),
            Ty::Set(_) => "set".into(),
            Ty::Class(i, a) if a.is_empty() && self.classes[*i].tparams.is_empty() => self.classes[*i].name.clone(),
            Ty::Enum(i) => self.enums[*i].name.clone(),
            Ty::Proto(i) if self.protos[*i].runtime => self.protos[*i].name.clone(),
            _ => return None,
        })
    }

    /// Types whose inferred type, from an expression generated for them without
    /// context, is a subtype of the type itself (no invariant containers of unions).
    fn inference_stable(&self, t: &Ty) -> bool {
        match t {
            Ty::Int | Ty::Bool | Ty::Str | Ty::Bytes | Ty::None | Ty::Enum(_) | Ty::Type(_) => true,
            Ty::Class(_, a) => a.is_empty(),
            Ty::Tuple(ts) => ts.iter().all(|t| self.inference_stable(t)),
            _ => false,
        }
    }

    /// Members a sequence pattern can be checked against precisely: anything
    /// else could have a subclass that is also a `Sequence` (ty keeps those).
    fn seq_pattern_safe(&self, t: &Ty) -> bool {
        matches!(t, Ty::List(_) | Ty::Tuple(_) | Ty::TupleVar(_) | Ty::None | Ty::Str | Ty::Bytes | Ty::Bool | Ty::Enum(_))
    }

    /// Whether a probe on a value of this type yields a checkable runtime type.
    fn probe_safe(&self, t: &Ty) -> bool {
        match t {
            Ty::TypedDict(_) | Ty::Var(_) => false,
            Ty::Union(ts) | Ty::Tuple(ts) => ts.iter().all(|t| self.probe_safe(t)),
            Ty::TupleVar(t) => self.probe_safe(t),
            // Runtime renders `Box[Any]`; fine.
            _ => true,
        }
    }

    fn fresh(&mut self, prefix: &str) -> String {
        self.fresh += 1;
        format!("{prefix}{}", self.fresh)
    }

    fn probe_line(&mut self, expr: &str) -> Tree {
        // A value typed by a type variable has no runtime type that is assignable
        // to it, so only probe values whose binding's type has none.
        let root = expr.split(['[', '(']).next().unwrap_or(expr);
        let root = if root.starts_with("self.") { root.splitn(3, '.').take(2).collect::<Vec<_>>().join(".") } else { root.split('.').next().unwrap_or(root).to_string() };
        let ty = BOUND.with(|b| { let b = b.borrow(); b.get(expr).or_else(|| b.get(&root)).cloned() });
        if let Some(t) = ty {
            if contains_var(&t) || !self.probe_safe(&t) {
                return node(format!("str({expr})"));
            }
        }
        self.probe += 1;
        node(format!("_probe({}, {expr})", self.probe))
    }

    // ----- random types -------------------------------------------------------------------------

    fn scalar(&self, g: &mut Source) -> Ty {
        g.weighted(&[(4, Ty::Int), (3, Ty::Str), (2, Ty::Bool), (1, Ty::Float), (1, Ty::Bytes)]).clone()
    }

    pub fn ty(&self, g: &mut Source, depth: usize, tvars: &[(String, Option<Ty>)]) -> Ty {
        let k = if depth == 0 { 0 } else { g.int(0..=19u32) };
        match k {
            19 => Ty::Callable((0..g.int(0..=2usize)).map(|_| self.scalar(g)).collect(), Box::new(self.ty(g, depth - 1, &[]))),
            18 => {
                let cs: Vec<usize> = (0..self.classes.len()).filter(|&c| self.classes[c].tparams.is_empty()).collect();
                if cs.is_empty() {
                    self.scalar(g)
                } else if cs.len() >= 2 && g.bool_with(0.3) {
                    Model::union_of(vec![Ty::Type(*g.pick(&cs)), Ty::Type(*g.pick(&cs))])
                } else {
                    Ty::Type(*g.pick(&cs))
                }
            }
            17 => {
                let mut lit = |g: &mut Source| {
                    let v = if g.bool() { g.pick(&["-1", "2", "3", "7", "42"]).to_string() } else { format!("{:?}", *g.pick(&["a", "b", "x y", ""])) };
                    Ty::Lit(v)
                };
                if g.bool_with(0.35) {
                    let n = g.int(2..=3usize);
                    Model::union_of((0..n).map(|_| lit(g)).collect())
                } else {
                    lit(g)
                }
            }
            15 | 16 if !self.aliases.is_empty() => {
                let a = g.pick(&self.aliases).clone();
                match &a.tparam {
                    None => a.target,
                    Some(v) => {
                        let arg = self.ty(g, depth - 1, tvars);
                        subst_var(&a.target, v, &arg)
                    }
                }
            }
            0..=4 => self.scalar(g),
            5 => Ty::opt(self.ty(g, depth - 1, tvars)),
            6 => {
                let n = g.int(2..=3usize);
                Model::union_of((0..n).map(|_| self.ty(g, depth - 1, tvars)).collect())
            }
            7 => Ty::Tuple((0..g.int(0..=3usize)).map(|_| self.ty(g, depth - 1, tvars)).collect()),
            8 => Ty::List(Box::new(self.ty(g, depth - 1, tvars))),
            9 => Ty::Dict(Box::new(g.pick(&[Ty::Str, Ty::Int]).clone()), Box::new(self.ty(g, depth - 1, tvars))),
            10 if !self.enums.is_empty() => Ty::Enum(g.index(self.enums.len())),
            11 if !self.classes.is_empty() => self.class_ty(g, depth),
            12 if !self.inhabited_protos().is_empty() => {
                let ps = self.inhabited_protos();
                Ty::Proto(*g.pick(&ps))
            }
            13 if !self.tds.is_empty() => Ty::TypedDict(g.index(self.tds.len())),
            14 if !tvars.is_empty() => Ty::Var(g.pick(tvars).0.clone()),
            14 => Ty::TupleVar(Box::new(self.scalar(g))),
            _ => self.scalar(g),
        }
    }

    /// A type whose type variables all have a value in `sc`.
    fn ty_in(&self, g: &mut Source, depth: usize, sc: &Scope) -> Ty {
        loop {
            let t = self.ty(g, depth, &sc.tvars);
            if vars_inhabited(&t, sc) {
                return t;
            }
        }
    }

    fn class_ty(&self, g: &mut Source, depth: usize) -> Ty {
        let i = g.index(self.classes.len());
        let args = self.classes[i].tparams.iter().map(|_| self.ty(g, depth.saturating_sub(1).min(1), &[])).collect();
        Ty::Class(i, args)
    }

    // ----- expressions --------------------------------------------------------------------------

    fn int_lit(&self, g: &mut Source) -> String {
        let (a, b) = (g.int(-3..=9i64), g.int(-1000..=1000i64));
        g.weighted(&[(6, a), (1, b), (1, 2i64.pow(31)), (1, -(2i64.pow(40)))]).to_string()
    }

    fn str_lit(&self, g: &mut Source) -> String {
        let s = *g.pick(&["", "a", "b", "ab", "x y", "1", "42", "A", "k", "none"]);
        format!("{s:?}")
    }

    /// An expression whose runtime value has model type `t` (or a subtype).
    pub fn expr(&mut self, g: &mut Source, t: &Ty, sc: &Scope, depth: usize) -> String {
        // Variables first: they carry narrowing.
        let fits: Vec<String> = sc.vars.iter().filter(|v| self.sub(&v.ty, t)).map(|v| v.path.clone()).collect();
        if !fits.is_empty() && g.bool_with(if depth == 0 { 0.8 } else { 0.45 }) {
            return g.pick(&fits).clone();
        }
        if depth > 0 && g.bool_with(0.12) {
            if let Some(e) = self.call_expr(g, t, sc, depth - 1) {
                return e;
            }
        }
        if depth > 0 && g.bool_with(0.06) {
            let c = self.expr(g, &Ty::Bool, sc, depth - 1);
            let a = self.expr(g, t, sc, depth - 1);
            let b = self.expr(g, t, sc, depth - 1);
            return format!("({a} if {c} else {b})");
        }
        let d = depth.saturating_sub(1);
        match t {
            Ty::Int => match g.int(0..=7u32) {
                0 if depth > 0 => format!("({} + {})", self.expr(g, &Ty::Int, sc, d), self.expr(g, &Ty::Int, sc, d)),
                1 if depth > 0 => format!("len({})", self.expr(g, &Ty::Str, sc, d)),
                2 if depth > 0 => format!("max({}, {})", self.expr(g, &Ty::Int, sc, d), self.expr(g, &Ty::Int, sc, d)),
                3 if depth > 0 => format!("({} * 2 - 1)", self.expr(g, &Ty::Int, sc, d)),
                4 if depth > 0 => format!("abs({})", self.expr(g, &Ty::Int, sc, d)),
                5 if depth > 0 && !self.enums.is_empty() => {
                    let e = g.index(self.enums.len());
                    if self.enums[e].str_values {
                        format!("len({}.value)", self.expr(g, &Ty::Enum(e), sc, d))
                    } else {
                        format!("{}.value", self.expr(g, &Ty::Enum(e), sc, d))
                    }
                }
                _ => self.int_lit(g),
            },
            Ty::Bool => match g.int(0..=6u32) {
                0 if depth > 0 => format!("({} < {})", self.expr(g, &Ty::Int, sc, d), self.expr(g, &Ty::Int, sc, d)),
                // Depth 0 operands: a str method on a literal is `LiteralString`, and
                // `<literal> != <LiteralString>` narrows to Never (candidate
                // bugs/candidates/literalstring-eq-negative-never).
                1 if depth > 0 => format!("({} == {})", self.expr(g, &Ty::Str, sc, 0), self.expr(g, &Ty::Str, sc, 0)),
                2 if depth > 0 => format!("(not {})", self.expr(g, &Ty::Bool, sc, d)),
                3 if depth > 0 => format!("bool({})", self.expr(g, &Ty::Int, sc, d)),
                4 if depth > 0 => {
                    let o = self.ty_in(g, 1, sc);
                    // Not a class object: ty then narrows the class name inside type
                    // expressions too (candidate isinstance-narrows-class-in-type-expr).
                    let o = if Model::members(&o).iter().any(|m| matches!(m, Ty::Type(_) | Ty::Object)) { Ty::Int } else { o };
                    let e = self.expr(g, &o, sc, d);
                    let n = self.isinstance_name(&self.ty(g, 0, &[])).unwrap_or("int".into());
                    format!("isinstance({e}, {n})")
                }
                _ => g.pick(&["True", "False"]).to_string(),
            },
            Ty::Str => match g.int(0..=6u32) {
                0 if depth > 0 => format!("({} + {})", self.expr(g, &Ty::Str, sc, d), self.expr(g, &Ty::Str, sc, d)),
                1 if depth > 0 => format!("str({})", self.expr(g, &Ty::Int, sc, d)),
                2 if depth > 0 => format!("f\"{{{}}}-{{{}!r}}\"", self.expr(g, &Ty::Int, sc, d), self.expr(g, &Ty::Str, sc, d)),
                3 if depth > 0 => format!("{}.upper()", self.expr(g, &Ty::Str, sc, d)),
                4 if depth > 0 && !self.enums.is_empty() => {
                    let e = g.index(self.enums.len());
                    format!("{}.name", self.expr(g, &Ty::Enum(e), sc, d))
                }
                _ => self.str_lit(g),
            },
            Ty::Float => match g.int(0..=2u32) {
                0 => format!("{}.5", g.int(-5..=5i32)),
                1 if depth > 0 => format!("({} / 2)", self.expr(g, &Ty::Int, sc, d)),
                _ => self.int_lit(g),
            },
            Ty::Bytes => g.pick(&["b\"\"", "b\"x\"", "b\"ab\""]).to_string(),
            Ty::Lit(v) => v.clone(),
            Ty::Type(c) => {
                let subs: Vec<usize> = (0..self.classes.len()).filter(|&k| self.classes[k].tparams.is_empty() && self.ancestors(k).contains(c)).collect();
                let k = *g.pick(&subs);
                // `type(o)` of an instance, or the class itself.
                if depth > 0 && g.bool_with(0.3) {
                    let o = self.construct(g, k, &[], sc, d);
                    format!("type({o})")
                } else {
                    self.classes[k].name.clone()
                }
            }
            Ty::None => "None".into(),
            Ty::Object => {
                let o = self.ty_in(g, 1, sc);
                self.expr(g, &o, sc, d)
            }
            Ty::Tuple(ts) => {
                let parts: Vec<String> = ts.iter().map(|t| self.expr(g, t, sc, d)).collect();
                // `tup*` return `tuple[*Ts]` of the inferred argument types.
                if depth > 0 && g.bool_with(0.12) && ts.iter().all(|t| self.inference_stable(t)) {
                    return format!("{}({})", g.pick(&["tupv", "tupv_legacy"]), parts.join(", "));
                }
                match parts.len() {
                    0 => "()".into(),
                    1 => format!("({},)", parts[0]),
                    _ => format!("({})", parts.join(", ")),
                }
            }
            Ty::TupleVar(t) => {
                let n = g.int(0..=3usize);
                let parts: Vec<String> = (0..n).map(|_| self.expr(g, t, sc, d)).collect();
                if g.bool_with(0.2) && depth > 0 {
                    return format!("tuple([{}])", parts.join(", "));
                }
                match parts.len() {
                    0 => "()".into(),
                    1 => format!("({},)", parts[0]),
                    _ => format!("({})", parts.join(", ")),
                }
            }
            Ty::List(t) => {
                if depth > 0 && g.bool_with(0.25) {
                    let src = self.ty(g, 0, &[]);
                    let x = self.fresh("x");
                    let mut it = self.expr(g, &Ty::List(Box::new(src.clone())), sc, d);
                    // A bare `[]` iterates `Unknown`, not `src`: `isinstance` in the condition
                    // then narrows `x` to `Unknown & float` and `abs(x)` is a genuine `float`
                    // (triage fp-invalid-assignment). Iterate one element of `src` instead.
                    if it == "[]" {
                        it = format!("[{}]", self.expr(g, &src, sc, d));
                    }
                    let mut inner = sc.clone();
                    inner.bind(x.clone(), src, false);
                    let e = self.expr(g, t, &inner, d);
                    let cond = if g.bool() { format!(" if {}", self.expr(g, &Ty::Bool, &inner, d)) } else { String::new() };
                    return format!("[{e} for {x} in {it}{cond}]");
                }
                let n = g.int(0..=3usize);
                format!("[{}]", join((0..n).map(|_| self.expr(g, t, sc, d)), ", "))
            }
            Ty::Set(t) => {
                let n = g.int(1..=3usize);
                format!("{{{}}}", join((0..n).map(|_| self.expr(g, t, sc, d)), ", "))
            }
            Ty::Dict(k, v) => {
                let n = g.int(0..=3usize);
                format!("{{{}}}", join((0..n).map(|_| format!("{}: {}", self.expr(g, k, sc, d), self.expr(g, v, sc, d))), ", "))
            }
            Ty::Union(ts) => {
                let m = g.pick(ts).clone();
                self.expr(g, &m, sc, depth)
            }
            Ty::Class(i, args) => self.construct(g, *i, args, sc, d),
            Ty::Enum(i) => {
                let e = &self.enums[*i];
                let m = g.pick(&e.members).clone();
                if g.bool_with(0.15) {
                    let idx = e.members.iter().position(|x| *x == m).unwrap();
                    if e.str_values { format!("{}({:?})", e.name, m.to_lowercase()) } else { format!("{}({})", e.name, idx + 1) }
                } else {
                    format!("{}.{m}", e.name)
                }
            }
            Ty::Proto(p) => {
                let impls: Vec<usize> = (0..self.classes.len())
                    .filter(|&c| self.implements(c, *p) && self.classes[c].tparams.is_empty() && !self.classes[c].is_abstract)
                    .collect();
                // The earliest implementer cannot have a field of this protocol (a
                // protocol field type needs an earlier implementer), so falling back
                // to it when deep ends the recursion.
                let c = if self.nesting > 6 { *impls.iter().min().unwrap() } else { *g.pick(&impls) };
                self.nesting += 1;
                let e = self.construct(g, c, &[], sc, d);
                self.nesting -= 1;
                e
            }
            Ty::TypedDict(i) => {
                let td = self.tds[*i].clone();
                let mut parts = Vec::new();
                for (n, t, required) in &self.td_fields(*i) {
                    if *required || g.bool() {
                        parts.push(format!("{n:?}: {}", self.expr(g, t, sc, d)));
                    }
                }
                if g.bool_with(0.3) {
                    format!("{}({})", td.name, join(parts.iter().map(|p| {
                        let (k, v) = p.split_once(": ").unwrap();
                        format!("{}={v}", k.trim_matches('"'))
                    }), ", "))
                } else {
                    format!("{{{}}}", parts.join(", "))
                }
            }
            Ty::Callable(ps, r) => {
                // A matching module function, or a lambda.
                let fs: Vec<String> = self
                    .funcs
                    .iter()
                    .filter(|f| !self.in_async && !f.sig.has_async && f.tparams.is_empty() && f.guard.is_none() && f.sig.overloads.is_empty() && f.sig.kwonly == 0 && f.sig.unpack.is_none() && f.params.len() == ps.len() && self.sub(&f.ret, r) && f.params.iter().zip(ps).all(|((_, p), q)| self.sub(q, p)))
                    .map(|f| f.name.clone())
                    .collect();
                if !fs.is_empty() && g.bool() {
                    return g.pick(&fs).clone();
                }
                let names: Vec<String> = ps.iter().map(|_| self.fresh("l")).collect();
                let mut inner = sc.clone();
                // A lambda body runs later: no narrowing of captured names carries over.
                inner.vars.retain(|v| !v.narrowed && !v.mutable);
                for (n, t) in names.iter().zip(ps) {
                    inner.bind(n.clone(), t.clone(), false);
                }
                let saved = self.in_async;
                self.in_async = true;
                let body = self.expr(g, r, &inner, d);
                self.in_async = saved;
                format!("(lambda {}: {body})", names.join(", "))
            }
            Ty::Var(n) => {
                // Only variables of a type parameter can produce one.
                let fits: Vec<String> = sc.vars.iter().filter(|v| v.ty == Ty::Var(n.clone())).map(|v| v.path.clone()).collect();
                fits.first().cloned().unwrap_or_else(|| panic!("no value of type {n}"))
            }
        }
    }

    fn construct(&mut self, g: &mut Source, c: usize, args: &[Ty], sc: &Scope, d: usize) -> String {
        let cls = self.classes[c].clone();
        let fields: Vec<(String, Ty)> = self.all_fields(c).into_iter().map(|(n, t)| (n, self.subst(c, args, &t))).collect();
        if cls.kind == ClassKind::Pydantic {
            // Keyword-only; an aliased field by its alias or its name as the config allows;
            // fields with defaults may be left out; lax fields sometimes get a value pydantic
            // converts (ty widens their parameters to `LaxInt` and the like).
            let (by_alias, by_name) = self.pyd_by(c);
            let mut vals: Vec<String> = Vec::new();
            let raw = self.all_fields(c);
            for ((n, t), (_, rt)) in fields.iter().zip(&raw) {
                if self.pyd_has_default(c, n) && g.bool_with(0.4) {
                    continue;
                }
                let key = if self.pyd_has_alias(c, n) && by_alias && (!by_name || g.bool()) { format!("{n}_al") } else { n.clone() };
                // Not for a type variable's field: an unspecialized model validates it as `Any`.
                let lax = if self.pyd_lax(c, n) && !contains_var(rt) && g.bool_with(0.3) { self.lax_expr(g, t, sc, d) } else { None };
                let v = match lax { Some(v) => v, None => self.expr(g, t, sc, d) };
                vals.push(format!("{key}={v}"));
            }
            // (BaseSettings forbids extra arguments unless told otherwise. ty rejects them on
            // settings even with `extra="allow"`: rarely there, so it does not hide the rest.)
            let settings = self.ancestors(c).iter().any(|&k| self.classes[k].pyd.settings);
            if self.pyd_cfg(c, |p| p.extra) == Some("allow") && g.bool_with(if settings { 0.03 } else { 0.4 }) {
                vals.push(format!("x_{}={}", g.int(0..=9u8), self.int_lit(g)));
            }
            if vals.len() > 1 && g.bool_with(0.3) {
                let i = g.index(vals.len());
                let v = vals.remove(i);
                vals.push(v);
            }
            return format!("{}({})", cls.name, vals.join(", "));
        }
        let kw = matches!(cls.kind, ClassKind::Dataclass { kw_only: true, .. });
        let vals: Vec<String> = fields
            .iter()
            .map(|(n, t)| {
                let v = self.expr(g, t, sc, d);
                if kw || g.bool_with(0.25) { format!("{n}={v}") } else { v }
            })
            .collect();
        // Keyword arguments may not precede positional ones.
        let mut seen_kw = false;
        let vals: Vec<String> = vals
            .into_iter()
            .zip(&fields)
            .map(|(v, (n, _))| {
                if v.starts_with(&format!("{n}=")) {
                    seen_kw = true;
                    v
                } else if seen_kw {
                    format!("{n}={v}")
                } else {
                    v
                }
            })
            .collect();
        let mut vals = vals;
        if cls.factory_last && !vals.is_empty() && g.bool_with(0.4) {
            vals.pop();
        }
        if cls.initvar && g.bool() {
            vals.push(format!("iv={}", self.int_lit(g)));
        }
        let explicit = !args.is_empty() && g.bool_with(0.4);
        if explicit {
            format!("{}[{}]({})", cls.name, join(args.iter().map(|a| self.render(a)), ", "), vals.join(", "))
        } else {
            format!("{}({})", cls.name, vals.join(", "))
        }
    }

    /// Arguments for a call to non-generic `f`: positional or keyword, defaults
    /// omitted, extra `*args`/`**kwargs`, `Unpack[TypedDict]` keys.
    fn call_args(&mut self, g: &mut Source, f: &Func, sc: &Scope, d: usize) -> String {
        let out = self.call_args_v(g, f, sc, d);
        let (mut out, lead) = out;
        // Sometimes pass a leading run of positional arguments as `*(...)`, and
        // `**kwargs` / `Unpack[TD]` keys as `**{...}` / `**TD(...)`.
        // Elements of a starred tuple are inferred without the parameters' context.
        let stable = f.params.iter().all(|(_, t)| self.inference_stable(t)) && f.sig.star.as_ref().is_none_or(|t| self.inference_stable(t));
        if lead >= 1 && stable && g.bool_with(0.25) {
            let m = g.int(1..=lead);
            let tup: Vec<String> = out.drain(..m).collect();
            let t = if tup.len() == 1 { format!("*({},)", tup[0]) } else { format!("*({})", tup.join(", ")) };
            out.insert(0, t);
        }
        if let Some(u) = f.sig.unpack {
            if g.bool_with(0.3) {
                let keys: Vec<String> = self.td_fields(u).iter().map(|(n, _, _)| format!("{n}=")).collect();
                out.retain(|a| !keys.iter().any(|k| a.starts_with(k.as_str())));
                let mut parts = Vec::new();
                for (n, t, req) in self.td_fields(u) {
                    if req || g.bool() {
                        parts.push(format!("{n}={}", self.expr(g, &t, sc, d)));
                    }
                }
                out.push(format!("**{}({})", self.tds[u].name, parts.join(", ")));
            }
        } else if let Some(k) = &f.sig.kwargs {
            if g.bool_with(0.3) {
                let k = k.clone();
                out.push(format!("**{{\"zz\": {}}}", self.expr(g, &k, sc, d)));
            }
        }
        out.join(", ")
    }

    /// The argument list of `call_args`, and how many leading entries are positional.
    fn call_args_v(&mut self, g: &mut Source, f: &Func, sc: &Scope, d: usize) -> (Vec<String>, usize) {
        let mut out = Vec::new();
        let mut lead = 0usize;
        let npos = f.params.len() - f.sig.kwonly;
        let mut kw = false;
        for (i, (n, t)) in f.params.iter().enumerate() {
            let has_d = f.sig.defaults.get(i).is_some_and(|x| x.is_some());
            if i < f.sig.posonly {
                out.push(self.expr(g, t, sc, d));
                lead += 1;
                if i + 1 == npos {
                    if let Some(st) = &f.sig.star {
                        for _ in 0..g.int(0..=2usize) {
                            out.push(self.expr(g, st, sc, d));
                            lead += 1;
                        }
                    }
                }
                continue;
            }
            if has_d && g.bool_with(0.35) {
                kw = true;
                continue;
            }
            let e = self.expr(g, t, sc, d);
            if i >= npos || kw || g.bool_with(0.2) {
                kw = true;
                out.push(format!("{n}={e}"));
            } else {
                out.push(e);
                lead += 1;
            }
            if i + 1 == npos && !kw {
                if let Some(st) = &f.sig.star {
                    for _ in 0..g.int(0..=2usize) {
                        out.push(self.expr(g, st, sc, d));
                        lead += 1;
                    }
                }
            }
        }
        if npos == 0 && !kw {
            if let Some(st) = &f.sig.star {
                for _ in 0..g.int(0..=2usize) {
                    out.insert(0, self.expr(g, st, sc, d));
                    lead += 1;
                }
            }
        }
        if let Some(k) = &f.sig.kwargs {
            for i in 0..g.int(0..=2usize) {
                out.push(format!("z{i}={}", self.expr(g, k, sc, d)));
            }
        } else if let Some(u) = f.sig.unpack {
            for (n, t, req) in self.td_fields(u) {
                if req || g.bool() {
                    out.push(format!("{n}={}", self.expr(g, &t, sc, d)));
                }
            }
        }
        (out, lead)
    }

    /// A call to a module function or a method returning (a subtype of) `t`.
    fn call_expr(&mut self, g: &mut Source, t: &Ty, sc: &Scope, d: usize) -> Option<String> {
        let mut opts: Vec<String> = Vec::new();
        for fi in 0..self.funcs.len() {
            let f = self.funcs[fi].clone();
            if f.guard.is_some() || (self.in_async && f.sig.has_async) {
                continue;
            }
            if f.sig.has_async {
                self.async_used = true;
            }
            if !f.sig.overloads.is_empty() {
                for (pt, rt) in f.sig.overloads.clone() {
                    if self.sub(&rt, t) {
                        let a = self.expr(g, &pt, sc, d);
                        opts.push(format!("{}({a})", f.name));
                    }
                }
            } else if f.tparams.is_empty() {
                if self.sub(&f.ret, t) {
                    let args = self.call_args(g, &f, sc, d);
                    opts.push(format!("{}({args})", f.name));
                }
            } else if f.tparams.len() == 1 && f.tparams[0].1.is_none() && f.sig.constraints.is_empty() {
                // Instantiate the single type parameter with `t` itself when the
                // return type is exactly the parameter.
                let v = Ty::Var(f.tparams[0].0.clone());
                if f.ret == v && self.inference_stable(t) && f.params.iter().all(|(_, p)| *p == v || p.clone() == Ty::List(Box::new(v.clone())) || !contains_var(p)) {
                    let args = join(
                        f.params.iter().map(|(_, p)| {
                            if *p == v {
                                self.expr(g, t, sc, d)
                            } else if *p == Ty::List(Box::new(v.clone())) {
                                format!("[{}]", self.expr(g, t, sc, d))
                            } else {
                                self.expr(g, p, sc, d)
                            }
                        }),
                        ", ",
                    );
                    opts.push(format!("{}({args})", f.name));
                }
            }
        }
        // Calls of callable-typed variables.
        for v in sc.vars.clone() {
            if let Ty::Callable(ps, r) = &v.ty {
                if self.sub(r, t) && !contains_var(r) {
                    let argv = join(ps.iter().map(|p| self.expr(g, p, sc, d)), ", ");
                    opts.push(format!("{}({argv})", v.path));
                }
            }
        }
        // Calls of a union of two callable-typed variables with the same parameters.
        let cvs: Vec<Var> = sc.vars.iter().filter(|v| matches!(&v.ty, Ty::Callable(_, r) if !contains_var(r))).cloned().collect();
        if cvs.len() >= 2 {
            let (a, b) = (g.pick(&cvs).clone(), g.pick(&cvs).clone());
            if let (Ty::Callable(pa, ra), Ty::Callable(pb, rb)) = (&a.ty, &b.ty) {
                if pa == pb && a.path != b.path && self.sub(ra, t) && self.sub(rb, t) {
                    let c = self.expr(g, &Ty::Bool, sc, 1);
                    let argv = join(pa.iter().map(|p| self.expr(g, p, sc, d)), ", ");
                    opts.push(format!("({} if {c} else {})({argv})", a.path, b.path));
                }
            }
        }
        // Class objects: `__name__`, class and static methods.
        for v in sc.vars.clone() {
            if let Ty::Type(c) = &v.ty {
                if self.sub(&Ty::Str, t) {
                    opts.push(format!("{}.__name__", v.path));
                }
                if self.in_methods {
                    continue;
                }
                for (_, m) in self.all_methods(*c) {
                    if matches!(m.kind, MethodKind::Static | MethodKind::Class) && self.sub(&m.ret, t) {
                        let argv = join(m.params.iter().map(|p| self.expr(g, p, sc, d)), ", ");
                        opts.push(format!("{}.{}({argv})", v.path, m.name));
                    }
                }
            }
        }
        // Members of protocol-typed variables.
        for v in sc.vars.clone() {
            if let Ty::Proto(pi) = &v.ty {
                let pr = self.protos[*pi].clone();
                for (n, mt) in &pr.methods {
                    if self.sub(mt, t) && !self.in_methods {
                        opts.push(format!("{}.{n}()", v.path));
                    }
                }
                for (n, at, _) in &pr.attrs {
                    if self.sub(at, t) {
                        opts.push(format!("{}.{n}", v.path));
                    }
                }
            }
        }
        // Methods on variables of class type.
        for v in sc.vars.clone() {
            let (c, args) = match &v.ty {
                Ty::Class(c, a) => (*c, a.clone()),
                _ => continue,
            };
            for (_, m) in self.all_methods(c) {
                if self.in_methods {
                    break;
                }
                let ret = if m.kind == MethodKind::SelfRet { v.ty.clone() } else { self.subst(c, &args, &m.ret) };
                if !self.sub(&ret, t) {
                    continue;
                }
                let ps: Vec<Ty> = m.params.iter().map(|p| self.subst(c, &args, p)).collect();
                let argv = join(ps.iter().map(|p| self.expr(g, p, sc, d)), ", ");
                match m.kind {
                    MethodKind::Property => opts.push(format!("{}.{}", v.path, m.name)),
                    MethodKind::Static | MethodKind::Class if g.bool() => {
                        opts.push(format!("{}.{}({argv})", self.classes[c].name, m.name))
                    }
                    _ => opts.push(format!("{}.{}({argv})", v.path, m.name)),
                }
            }
            for (n, ft) in self.all_fields(c) {
                if self.sub(&self.subst(c, &args, &ft), t) {
                    opts.push(format!("{}.{n}", v.path));
                }
            }
            for (n, xt, _) in self.all_extras(c) {
                if self.sub(&xt, t) {
                    opts.push(format!("{}.{n}", v.path));
                }
            }
        }
        if opts.is_empty() { None } else { Some(g.pick(&opts).clone()) }
    }

    // ----- statements ---------------------------------------------------------------------------

    fn probe_target(&mut self, g: &mut Source, sc: &Scope) -> Option<String> {
        let vs: Vec<Var> = sc.vars.iter().filter(|v| self.probe_safe(&v.ty)).cloned().collect();
        if vs.is_empty() {
            return None;
        }
        let v = g.pick(&vs).clone();
        // Sometimes probe a field, element or method result instead.
        match &v.ty {
            Ty::Class(c, args) if g.bool_with(0.4) => {
                let (c, args) = (*c, args.clone());
                let fs = self.all_fields(c);
                let ms: Vec<Method> = self.all_methods(c).into_iter().map(|(_, m)| m).filter(|m| m.params.is_empty()).collect();
                if !ms.is_empty() && g.bool() {
                    let m = g.pick(&ms).clone();
                    if self.probe_safe(&self.subst(c, &args, &m.ret)) {
                        return Some(if m.kind == MethodKind::Property { format!("{}.{}", v.path, m.name) } else { format!("{}.{}()", v.path, m.name) });
                    }
                }
                if !fs.is_empty() {
                    let (n, t) = g.pick(&fs).clone();
                    if self.probe_safe(&self.subst(c, &args, &t)) {
                        return Some(format!("{}.{n}", v.path));
                    }
                }
                Some(v.path)
            }
            Ty::Type(_) if g.bool_with(0.3) => Some(format!("{}.__name__", v.path)),
            Ty::Proto(pi) if g.bool_with(0.5) => {
                let pr = &self.protos[*pi];
                let mut ms: Vec<String> = pr.methods.iter().map(|(n, _)| format!("{}.{n}()", v.path)).collect();
                ms.extend(pr.attrs.iter().map(|(n, _, _)| format!("{}.{n}", v.path)));
                if ms.is_empty() { Some(v.path) } else { Some(g.pick(&ms).clone()) }
            }
            Ty::Tuple(_) | Ty::List(..) | Ty::TupleVar(..) | Ty::Str if g.bool_with(0.15) => {
                let _ = ts_len(&v.ty);
                let sl = *g.pick(&["1:", ":2", "::-1", "-2:", "1:3", ":"]);
                Some(format!("{}[{sl}]", v.path))
            }
            Ty::Tuple(ts) if !ts.is_empty() && g.bool_with(0.4) => {
                let i = g.index(ts.len());
                Some(if g.bool_with(0.3) { format!("{}[{}]", v.path, i as i64 - ts.len() as i64) } else { format!("{}[{i}]", v.path) })
            }
            _ => Some(v.path),
        }
    }

    fn assignable_vars(&self, sc: &Scope) -> Vec<Var> {
        sc.vars.iter().filter(|v| v.mutable && !v.path.contains(['.', '[']) && !sc.frozen.contains(&v.path)).cloned().collect()
    }

    pub fn block(&mut self, g: &mut Source, sc: &mut Scope, ret: Option<&Ty>, depth: usize, n: (usize, usize)) -> Vec<Tree> {
        let n = g.int(n.0..=n.1);
        let mut out = Vec::new();
        for _ in 0..n {
            out.extend(self.stmt(g, sc, ret, depth));
        }
        if out.is_empty() {
            out.push(node("pass"));
        }
        out
    }

    fn stmt(&mut self, g: &mut Source, sc: &mut Scope, ret: Option<&Ty>, depth: usize) -> Vec<Tree> {
        let table = [
            (5, 0u32), // annotated local
            (2, 1),    // unannotated scalar local
            (5, 2),    // probe
            (if depth > 0 { 6 } else { 0 }, 3), // narrowing if
            (if depth > 0 { 3 } else { 0 }, 4), // match
            (if depth > 0 { 3 } else { 0 }, 5), // loop with an accumulator
            (2, 6),    // reassignment of a mutable local
            (if depth > 0 { 1 } else { 0 }, 7), // try/except
            (1, 8),    // tuple unpacking
            (if depth > 0 { 1 } else { 0 }, 9), // nested function
            (if ret.is_some() && depth > 0 { 1 } else { 0 }, 10), // early return in a branch
            (1, 11),   // container mutation
            (if depth > 0 { 2 } else { 0 }, 12), // walrus narrowing
            (if depth > 0 { 1 } else { 0 }, 13), // with
            (if depth > 0 { 3 } else { 0 }, 14), // for over an iterable
            (3, 15), // TypedDict key operations
            (3, 16), // attribute write
            (if depth > 0 && !self.in_methods && !self.in_async { 2 } else { 0 }, 17), // coroutine run with asyncio.run
            (if depth > 0 { 2 } else { 0 }, 18), // narrowing by a NoReturn call
            (if self.seqs.is_empty() { 0 } else { 2 }, 19), // user sequence subscripts
            (1, 20), // generic helpers: solving through callables, generic methods, generic protocols
            (3, 21), // operator probes over variables
        ];
        let k = *g.weighted(&table);
        match k {
            0 => {
                let t = self.ty_in(g, 2, sc);
                if matches!(t, Ty::Var(_)) && !sc.vars.iter().any(|v| v.ty == t) {
                    return vec![];
                }
                let name = self.fresh("v");
                let e = self.expr(g, &t, sc, 2);
                let mutable = g.bool_with(0.3);
                sc.bind(name.clone(), t.clone(), mutable);
                if !mutable && g.bool_with(0.15) {
                    // A bare `Final` takes the inferred type.
                    if self.inference_stable(&t) && g.bool() {
                        return vec![node(format!("{name}: Final = {e}"))];
                    }
                    return vec![node(format!("{name}: Final[{}] = {e}", self.render(&t)))];
                }
                if g.bool_with(0.15) {
                    self.quoted.set(true);
                    let r = self.render(&t);
                    self.quoted.set(false);
                    if !r.contains('\'') {
                        return vec![node(format!("{name}: '{r}' = {e}"))];
                    }
                }
                vec![node(format!("{name}: {} = {e}", self.render(&t)))]
            }
            1 => {
                let t = self.scalar(g);
                let t = if g.bool_with(0.3) { Ty::Tuple(vec![t, self.scalar(g)]) } else { t };
                let name = self.fresh("v");
                let e = self.expr(g, &t, sc, 2);
                sc.bind(name.clone(), t, false);
                vec![node(format!("{name} = {e}"))]
            }
            2 => self.probe_target(g, sc).map(|e| vec![self.probe_line(&e)]).unwrap_or_default(),
            3 => self.narrowing_if(g, sc, ret, depth),
            4 => self.match_stmt(g, sc, ret, depth),
            5 => self.loop_stmt(g, sc, ret, depth),
            6 => {
                let vs = self.assignable_vars(sc);
                if vs.is_empty() {
                    return vec![];
                }
                let v = g.pick(&vs).clone();
                let e = self.expr(g, &v.ty, sc, 2);
                vec![node(format!("{} = {e}", v.path)), self.probe_line(&v.path)]
            }
            7 => {
                let name = self.fresh("v");
                let s = self.expr(g, &Ty::Str, sc, 1);
                let t = Ty::opt(Ty::Int);
                let mut body_sc = sc.clone();
                let body = self.block(g, &mut body_sc, ret, depth - 1, (0, 2));
                let ename = self.fresh("e");
                let mut out = vec![node(format!("{name}: {} = None", self.render(&t)))];
                let mut try_body = vec![node(format!("{name} = int({s})"))];
                try_body.extend(body);
                out.push(node("try:").with(try_body));
                let mut exc_sc = sc.clone();
                exc_sc.bind(ename.clone(), Ty::Object, false);
                let mut eb = vec![node(format!("_probe({}, {ename})", { self.probe += 1; self.probe }))];
                eb.extend(self.block(g, &mut exc_sc.clone(), ret, depth - 1, (0, 1)));
                out.push(node(format!("except (ValueError, KeyError) as {ename}:")).with(eb));
                if g.bool() {
                    out.push(node("else:").with(vec![self.probe_line(&name)]));
                }
                // Probes in `finally` would find astral-sh/ty#233 (control flow through
                // `finally` is not modelled yet) again and again.
                if g.bool_with(0.3) {
                    out.push(node("finally:").with(vec![node(format!("str({name})"))]));
                }
                out.push(self.probe_line(&name));
                sc.bind(name, t, true);
                out
            }
            8 => {
                let ts: Vec<Var> = sc.vars.iter().filter(|v| matches!(&v.ty, Ty::Tuple(xs) if xs.len() >= 2)).cloned().collect();
                if ts.is_empty() {
                    return vec![];
                }
                let v = g.pick(&ts).clone();
                let Ty::Tuple(xs) = &v.ty else { unreachable!() };
                let names: Vec<String> = xs.iter().map(|_| self.fresh("u")).collect();
                let star = xs.len() >= 3 && g.bool_with(0.4);
                let mut out = Vec::new();
                if star {
                    let tail: Vec<Ty> = xs[1..].to_vec();
                    out.push(node(format!("{}, *{} = {}", names[0], names[1], v.path)));
                    sc.bind(names[0].clone(), xs[0].clone(), false);
                    // The starred target is an unannotated list of the inferred element
                    // types: probe it, but never use it as a `list[...]` of the model type.
                    let _ = tail;
                    out.push(self.probe_line(&names[1]));
                } else {
                    out.push(node(format!("{} = {}", names.join(", "), v.path)));
                    for (n, t) in names.iter().zip(xs) {
                        sc.bind(n.clone(), t.clone(), false);
                    }
                    out.push(self.probe_line(&names[g.index(names.len())]));
                }
                out
            }
            9 => {
                let name = self.fresh("inner");
                let pt = self.ty_in(g, 1, sc);
                let rt = self.ty_in(g, 1, sc);
                if matches!(pt, Ty::Var(_)) || matches!(rt, Ty::Var(_)) {
                    return vec![];
                }
                let p = self.fresh("q");
                // Inner functions see outer bindings at their declared types.
                let mut isc = Scope { vars: sc.vars.iter().filter(|v| !v.path.contains(['.', '['])).cloned().collect(), frozen: vec![], in_loop: false, tvars: sc.tvars.clone(), iterating: sc.iterating, captured: vec![] };
                isc.vars.retain(|v| !v.mutable);
                // Drop narrowed views. ty keeps narrowing of a captured name in a nested
                // function, but loses it in a comprehension inside one (candidate
                // bugs/candidates/closure-narrowing-comprehension).
                isc.vars.retain(|v| !v.narrowed);
                // No attribute writes on captured objects: ty keeps the caller's
                // narrowing of their attributes across the call.
                isc.captured = isc.vars.iter().map(|v| v.path.clone()).collect();
                isc.bind(p.clone(), pt.clone(), false);
                let mut body = self.block(g, &mut isc, Some(&rt), depth - 1, (0, 2));
                body.push(node(format!("return {}", self.expr(g, &rt, &isc, 2))));
                let r = self.fresh("v");
                let arg = self.expr(g, &pt, sc, 1);
                sc.bind(r.clone(), rt.clone(), false);
                vec![
                    node(format!("def {name}({p}: {}) -> {}:", self.render(&pt), self.render(&rt))).with(body),
                    node(format!("{r} = {name}({arg})")),
                    self.probe_line(&r),
                ]
            }
            10 => {
                let ret = ret.unwrap().clone();
                let c = self.expr(g, &Ty::Bool, sc, 2);
                let mut isc = sc.clone();
                let mut body = self.block(g, &mut isc, Some(&ret), 0, (0, 1));
                body.push(node(format!("return {}", self.expr(g, &ret, &isc, 2))));
                vec![node(format!("if {c}:")).with(body)]
            }
            11 if sc.iterating => vec![],
            11 => {
                let ls: Vec<Var> = sc.vars.iter().filter(|v| matches!(v.ty, Ty::List(_) | Ty::Dict(..) | Ty::Set(_)) && !v.path.contains(['.', '['])).cloned().collect();
                if ls.is_empty() {
                    return vec![];
                }
                let v = g.pick(&ls).clone();
                let line = match &v.ty {
                    Ty::List(t) => match g.int(0..=2u32) {
                        0 => format!("{}.append({})", v.path, self.expr(g, t, sc, 1)),
                        1 => format!("{}.extend([{}])", v.path, self.expr(g, t, sc, 1)),
                        _ => format!("{}.insert(0, {})", v.path, self.expr(g, t, sc, 1)),
                    },
                    Ty::Set(t) => format!("{}.add({})", v.path, self.expr(g, t, sc, 1)),
                    Ty::Dict(k, t) => format!("{}[{}] = {}", v.path, self.expr(g, k, sc, 1), self.expr(g, t, sc, 1)),
                    _ => unreachable!(),
                };
                let mut out = vec![node(line)];
                if let Ty::List(t) = &v.ty {
                    if self.probe_safe(t) {
                        out.push(self.probe_line(&format!("{}[-1]", v.path)));
                    }
                }
                out
            }
            12 => {
                let t = self.ty_in(g, 1, sc);
                // The target is bound to the inferred type of the expression.
                if !self.inference_stable(&t) {
                    return vec![];
                }
                let name = self.fresh("w");
                let e = self.expr(g, &Ty::opt(t.clone()), sc, 2);
                let mut isc = sc.narrow(&name, t.clone());
                let mut body = vec![self.probe_line(&name)];
                body.extend(self.block(g, &mut isc, ret, depth - 1, (0, 2)));
                let mut out = vec![node(format!("if ({name} := {e}) is not None:")).with(body)];
                let mut esc = sc.narrow(&name, Ty::None);
                out.push(node("else:").with(vec![self.probe_line(&name)].into_iter().chain(self.block(g, &mut esc, ret, 0, (0, 1))).collect()));
                sc.bind(name.clone(), Ty::opt(t), false);
                out.push(self.probe_line(&name));
                out
            }
            13 => {
                let t = self.ty_in(g, 1, sc);
                // The target is bound to the inferred type of the expression.
                if !self.inference_stable(&t) {
                    return vec![];
                }
                let name = self.fresh("cm");
                let e = self.expr(g, &t, sc, 1);
                let mut isc = sc.clone();
                isc.bind(name.clone(), t.clone(), false);
                let mut body = vec![self.probe_line(&name)];
                body.extend(self.block(g, &mut isc, ret, depth - 1, (0, 2)));
                sc.bind(name.clone(), t, false);
                vec![node(format!("with contextlib.nullcontext({e}) as {name}:")).with(body)]
            }
            14 => self.foreach_stmt(g, sc, ret, depth),
            15 => self.typed_dict_stmt(g, sc),
            16 => self.attr_write_stmt(g, sc),
            17 => self.async_stmt(g, sc, depth),
            18 => self.noreturn_stmt(g, sc, ret, depth),
            19 => self.seq_stmt(g, sc),
            20 => self.generic_stmt(g, sc),
            21 => self.op_probe_stmt(g, sc),
            _ => unreachable!(),
        }
    }

    /// Picks a binding with a union-like type (or a class with subclasses) to narrow.
    fn narrowable(&self, g: &mut Source, sc: &Scope) -> Option<Var> {
        let vs: Vec<Var> = sc
            .vars
            .iter()
            .filter(|v| !v.mutable && !sc.frozen.contains(&v.path))
            .filter(|v| match &v.ty {
                Ty::Union(_) | Ty::Object | Ty::Enum(_) | Ty::Proto(_) | Ty::Bool => true,
                Ty::Class(c, a) => a.is_empty() && self.classes.iter().any(|k| k.base == Some(*c)),
                Ty::Type(c) => self.classes.iter().any(|k| k.base == Some(*c)),
                Ty::TypedDict(i) => self.td_fields(*i).iter().any(|f| !f.2),
                Ty::Tuple(ts) => ts.iter().any(|t| matches!(t, Ty::Union(_))),
                _ => false,
            })
            .cloned()
            .collect();
        if vs.is_empty() { None } else { Some(g.pick(&vs).clone()) }
    }

    /// Splits `t` into what `isinstance(_, name)` keeps and what it removes, when exact.
    fn split_isinstance(&self, t: &Ty, pick: &Ty) -> Option<(String, Ty, Option<Ty>)> {
        let name = self.isinstance_name(pick)?;
        let ms = Model::members(t);
        // Exactness: every member is either entirely inside or entirely outside the
        // checked class. `bool` is inside `int`; generic and protocol members are never split.
        let inside = |m: &Ty| -> Option<bool> {
            match (m, pick) {
                (Ty::Bool, Ty::Int) => Some(true),
                (Ty::Bool, Ty::Lit(v)) if !v.starts_with('"') => Some(true),
                // A `float` annotation is `int | float`: never split exactly.
                (Ty::Int, Ty::Bool) | (Ty::Float, _) | (_, Ty::Float) => None,
                (Ty::Class(i, a), Ty::Class(j, _)) if a.is_empty() => {
                    if self.ancestors(*i).contains(j) {
                        Some(true)
                    } else if self.ancestors(*j).contains(i) {
                        None
                    } else {
                        Some(false)
                    }
                }
                (Ty::Class(..), Ty::Class(..)) => None,
                // A non-final class may have a subclass that also derives from a builtin:
                // `isinstance(x, str)` on `C | Literal["b"]` keeps `(C & str) | Literal["b"]`.
                // Only final checked classes (`bool`, enums) exclude it.
                (Ty::Class(..), Ty::Bool | Ty::Enum(_)) => Some(false),
                (Ty::Class(..), _) => None,
                // IntEnum/StrEnum members are ints/strs.
                (Ty::Enum(e), p) if self.enums[*e].flavour != "Enum" && !matches!(p, Ty::Enum(_)) => None,
                (Ty::Class(i, _), Ty::Tuple(_) | Ty::TupleVar(_)) if self.classes[*i].kind == ClassKind::NamedTuple => None,
                // A callable may be an instance of any non-final class.
                (Ty::Callable(..), _) | (_, Ty::Callable(..)) => None,
                (Ty::Proto(_), _) | (_, Ty::Proto(_)) | (Ty::Object, _) | (Ty::TypedDict(_), _) | (Ty::Var(_), _) => None,
                (Ty::TypedDict(_), Ty::Dict(..)) => None,
                _ => Some(self.isinstance_name(m).as_deref() == Some(name.as_str())),
            }
        };
        let mut yes = Vec::new();
        let mut no = Vec::new();
        for m in &ms {
            match inside(m)? {
                true => yes.push(m.clone()),
                false => no.push(m.clone()),
            }
        }
        if yes.is_empty() {
            return None;
        }
        let rest = if no.is_empty() { None } else { Some(Model::union_of(no)) };
        Some((name, Model::union_of(yes), rest))
    }

    fn narrowing_if(&mut self, g: &mut Source, sc: &mut Scope, ret: Option<&Ty>, depth: usize) -> Vec<Tree> {
        let Some(v) = self.narrowable(g, sc) else {
            let c = self.expr(g, &Ty::Bool, sc, 2);
            let mut a = sc.clone();
            let mut b = sc.clone();
            return vec![
                node(format!("if {c}:")).with(self.block(g, &mut a, ret, depth - 1, (1, 2))),
                node("else:").with(self.block(g, &mut b, ret, depth - 1, (0, 2))),
            ];
        };
        let ms = Model::members(&v.ty);
        let p = v.path.clone();
        // (condition, then-type, else-type)
        let mut arms: Vec<(String, Option<Ty>, Option<Ty>)> = Vec::new();
        if ms.contains(&Ty::None) {
            let rest = Model::union_of(ms.iter().filter(|m| **m != Ty::None).cloned().collect());
            if g.bool() {
                arms.push((format!("{p} is None"), Some(Ty::None), Some(rest)));
            } else {
                arms.push((format!("{p} is not None"), Some(rest), Some(Ty::None)));
            }
        }
        for m in &ms {
            if let Some((name, yes, no)) = self.split_isinstance(&v.ty, m) {
                arms.push((format!("isinstance({p}, {name})"), Some(yes.clone()), no.clone()));
                if let Some(no) = no {
                    arms.push((format!("not isinstance({p}, {name})"), Some(no), Some(yes)));
                }
            }
        }
        // `isinstance` with a tuple of two classes, when both split exactly.
        let splits: Vec<(String, Ty, Option<Ty>)> = ms.iter().filter_map(|m| self.split_isinstance(&v.ty, m)).collect();
        if splits.len() >= 2 {
            let (a, b) = (&splits[0], &splits[1]);
            if a.0 != b.0 {
                let yes = Model::union_of(vec![a.1.clone(), b.1.clone()]);
                let ys = Model::members(&yes);
                let no: Vec<Ty> = ms.iter().filter(|m| !ys.contains(m)).cloned().collect();
                let no = if no.is_empty() { None } else { Some(Model::union_of(no)) };
                arms.push((format!("isinstance({p}, ({}, {}))", a.0, b.0), Some(yes), no));
            }
        }
        // Forms whose narrowing the model does not track (the declared type stays valid).
        // No 0/1 (a bool equals them: candidate eq-int-literal-drops-bool) and no
        // IntEnum/StrEnum members (they equal plain values: strenum-eq-narrowing).
        let mixed_enum = ms.iter().any(|m| matches!(m, Ty::Enum(e) if self.enums[*e].flavour != "Enum"));
        let lit = |g: &mut Source, m: &Ty| -> Option<String> {
            if mixed_enum {
                return None;
            }
            match m {
                Ty::Int => Some(g.pick(&[-1i64, 2, 3, 7]).to_string()),
                Ty::Str => Some(format!("{:?}", *g.pick(&["", "a", "b", "x y"]))),
                _ => None,
            }
        };
        for m in &ms {
            if let Some(l) = lit(g, m) {
                let l2 = lit(g, m).unwrap();
                arms.push((format!("{p} == {l}"), None, None));
                arms.push((format!("{p} != {l}"), None, None));
                arms.push((format!("{p} in ({l}, {l2})"), None, None));
                arms.push((format!("{p} not in ({l}, {l2})"), None, None));
            }
            match m {
                Ty::Int | Ty::Str | Ty::Bool | Ty::Bytes => {
                    let n = self.isinstance_name(m).unwrap();
                    arms.push((format!("type({p}) is {n}"), Some(m.clone()), None));
                    // ty does not narrow on `type(x) == C` (only `is`).
                    arms.push((format!("type({p}) == {n}"), None, None));
                    arms.push((format!("issubclass(type({p}), {n})"), None, None));
                }
                Ty::Class(c, a) if a.is_empty() && self.classes[*c].tparams.is_empty() => {
                    let n = self.classes[*c].name.clone();
                    arms.push((format!("type({p}) is {n}"), Some(m.clone()), None));
                    // `hasattr` positive branch only: the negative one is `~<has f>`, which no
                    // runtime type of a non-final class provably fits.
                    let _ = c;
                }
                // No `callable(x)`: its negative branch is `~Callable`, which no runtime
                // type of a non-final class provably fits.
                _ => {}
            }
        }
        if ms.len() >= 2 && ms.iter().all(|m| matches!(m, Ty::Tuple(_))) {
            if let Ty::Tuple(ts) = g.pick(&ms) {
                arms.push((format!("len({p}) == {}", ts.len()), None, None));
            }
        }
        if ms.contains(&Ty::None) && ms.len() >= 2 && !mixed_enum {
            arms.push((format!("{p} is None or bool({p})"), None, None));
        }
        if let Ty::Type(c) = &v.ty {
            for s in (0..self.classes.len()).filter(|&k| self.classes[k].base == Some(*c)) {
                arms.push((format!("issubclass({p}, {})", self.classes[s].name), Some(Ty::Type(s)), None));
                arms.push((format!("{p} is {}", self.classes[s].name), None, None));
            }
        }
        // Subclass checks on a class hierarchy.
        if let Ty::Class(c, _) = &v.ty {
            let subs: Vec<usize> = (0..self.classes.len()).filter(|&k| self.classes[k].base == Some(*c)).collect();
            for s in subs {
                arms.push((format!("isinstance({p}, {})", self.classes[s].name), Some(Ty::Class(s, vec![])), None));
                arms.push((format!("type({p}) is {}", self.classes[s].name), Some(Ty::Class(s, vec![])), None));
            }
        }
        if let Ty::Enum(e) = &v.ty {
            let en = self.enums[*e].clone();
            let m = g.pick(&en.members).clone();
            arms.push((format!("{p} is {}.{m}", en.name), None, None));
            arms.push((format!("{p} == {}.{m}", en.name), None, None));
            let m2 = g.pick(&en.members).clone();
            arms.push((format!("{p} in ({}.{m}, {}.{m2})", en.name, en.name), None, None));
        }
        if v.ty == Ty::Bool {
            arms.push((format!("{p} is True"), None, None));
            arms.push((format!("{p}"), None, None));
        } else if g.bool_with(0.5) {
            // Truthiness narrowing (`~AlwaysFalsy`/`~AlwaysTruthy`), and the conditions ty then
            // judges always true/false (the `truth__` oracle).
            arms.push((format!("{p}"), None, None));
            arms.push((format!("not {p}"), None, None));
        }
        if let Ty::Proto(pi) = &v.ty {
            if self.protos[*pi].runtime {
                let impls: Vec<usize> = (0..self.classes.len()).filter(|&c| self.implements(c, *pi) && self.classes[c].tparams.is_empty()).collect();
                if !impls.is_empty() {
                    let c = *g.pick(&impls);
                    arms.push((format!("isinstance({p}, {})", self.classes[c].name), Some(Ty::Class(c, vec![])), None));
                }
            }
        }
        if let Ty::TypedDict(i) = &v.ty {
            let opt: Vec<(String, Ty, bool)> = self.td_fields(*i).into_iter().filter(|f| !f.2).collect();
            let (n, t, _) = g.pick(&opt).clone();
            arms.push((format!("{n:?} in {p}"), None, None));
            // Narrowed view of the key.
            let key = format!("{p}[{n:?}]");
            let mut a = sc.narrow(&key, t);
            let mut body = vec![self.probe_line(&key)];
            body.extend(self.block(g, &mut a, ret, depth - 1, (0, 2)));
            return vec![node(format!("if {n:?} in {p}:")).with(body)];
        }
        // Truthiness on optional scalars: only the then-branch narrows exactly.
        // Not IntEnum/StrEnum: ty keeps `E & ~AlwaysFalsy` there (known intenum-truthiness-precision).
        if ms.contains(&Ty::None) && ms.len() == 2 && ms.iter().any(|m| matches!(m, Ty::Class(..)) || matches!(m, Ty::Enum(e) if self.enums[*e].flavour == "Enum")) {
            let rest = ms.iter().find(|m| **m != Ty::None).unwrap().clone();
            arms.push((p.clone(), Some(rest), None));
        }
        // Tuples with a union element: narrow the element by index.
        if let Ty::Tuple(ts) = &v.ty {
            if let Some(i) = ts.iter().position(|t| matches!(t, Ty::Union(_))) {
                let el = format!("{p}[{i}]");
                let ems = Model::members(&ts[i]);
                if ems.contains(&Ty::None) {
                    let rest = Model::union_of(ems.iter().filter(|m| **m != Ty::None).cloned().collect());
                    let mut a = sc.narrow(&el, rest);
                    let mut body = vec![self.probe_line(&el)];
                    body.extend(self.block(g, &mut a, ret, depth - 1, (0, 2)));
                    return vec![node(format!("if {el} is not None:")).with(body), node("else:").with(vec![self.probe_line(&el)])];
                }
            }
        }
        // User type guards.
        for f in self.funcs.clone() {
            if let Some((gt, is_typeis)) = &f.guard {
                if self.sub(gt, &v.ty) || (*is_typeis && Model::members(&v.ty).contains(gt)) {
                    arms.push((format!("{}({p})", f.name), Some(gt.clone()), None));
                }
            }
        }
        if arms.is_empty() {
            return vec![];
        }
        let (cond, yes, no) = g.pick(&arms).clone();
        // A `<cond> and <use>` conjunct exercising the narrowed type inside the condition.
        let mut cond = cond;
        if let Some(y) = &yes {
            if g.bool_with(0.2) && matches!(y, Ty::Int) {
                cond = format!("{cond} and {p} > 0");
            } else if g.bool_with(0.15) && matches!(y, Ty::Str) {
                cond = format!("{cond} and {p}.startswith(\"a\")");
            }
        }
        let mut a = match &yes {
            Some(t) => sc.narrow(&p, t.clone()),
            None => sc.narrow(&p, v.ty.clone()),
        };
        let mut then_b = vec![self.probe_line(&p)];
        then_b.extend(self.block(g, &mut a, ret, depth - 1, (0, 3)));
        let mut out = vec![node(format!("if {cond}:")).with(then_b)];
        if g.bool_with(0.7) {
            let exact_else = !cond.contains(" and ");
            let mut b = match (&no, exact_else) {
                (Some(t), true) => sc.narrow(&p, t.clone()),
                _ => sc.narrow(&p, v.ty.clone()),
            };
            let mut else_b = vec![self.probe_line(&p)];
            else_b.extend(self.block(g, &mut b, ret, depth - 1, (0, 2)));
            out.push(node("else:").with(else_b));
        }
        out.push(self.probe_line(&p));
        out
    }

    fn match_stmt(&mut self, g: &mut Source, sc: &mut Scope, ret: Option<&Ty>, depth: usize) -> Vec<Tree> {
        let Some(v) = self.narrowable(g, sc) else { return vec![] };
        // A capture from a union with a type variable has type `T & int`, which no
        // runtime type is provably assignable to.
        if contains_var(&v.ty) {
            return vec![];
        }
        let p = v.path.clone();
        let ms = Model::members(&v.ty);
        let mut cases: Vec<Tree> = Vec::new();
        let mut covered = false;
        for m in ms.iter().take(4) {
            let cap = self.fresh("c");
            let (pat, bind): (String, Option<Ty>) = match m {
                Ty::None => ("None".into(), None),
                Ty::Int | Ty::Str | Ty::Bool | Ty::Float | Ty::Bytes => {
                    let n = self.isinstance_name(m).unwrap();
                    // `int()` also matches bools, so only bind a precise type when exact.
                    let exact = !(m == &Ty::Int && ms.contains(&Ty::Bool)) && !(m == &Ty::Float && ms.iter().any(|x| matches!(x, Ty::Int | Ty::Bool)));
                    if !exact {
                        continue;
                    }
                    // A value pattern compares with `==`, which other members may override.
                    if g.bool_with(0.3) && *m == Ty::Int && ms.iter().all(|x| *x == Ty::Int) {
                        (format!("0 | 1 as {cap}"), Some(Ty::Int))
                    } else if g.bool_with(0.25) {
                        let guard = match m {
                            Ty::Int => format!("{cap} > 0"),
                            Ty::Str => format!("{cap}.startswith(\"a\")"),
                            _ => format!("bool({cap})"),
                        };
                        (format!("{n}() as {cap} if {guard}"), Some(m.clone()))
                    } else if g.bool_with(0.2) && m == &Ty::Str && ms.contains(&Ty::Bytes) {
                        (format!("str() | bytes() as {cap}"), Some(Model::union_of(vec![Ty::Str, Ty::Bytes])))
                    } else {
                        (format!("{n}() as {cap}"), Some(m.clone()))
                    }
                }
                Ty::Tuple(ts) if !ts.is_empty() && ts.len() <= 3 && ms.iter().all(|x| self.seq_pattern_safe(x)) => {
                    if ms.iter().filter(|x| matches!(x, Ty::Tuple(_) | Ty::TupleVar(_) | Ty::List(_) | Ty::Class(..))).count() > 1 {
                        continue;
                    }
                    let names: Vec<String> = ts.iter().map(|_| self.fresh("c")).collect();
                    let mut a = sc.clone();
                    for (n, t) in names.iter().zip(ts) {
                        a.bind(n.clone(), t.clone(), false);
                    }
                    let mut body = vec![self.probe_line(&names[0])];
                    body.extend(self.block(g, &mut a, ret, depth - 1, (0, 1)));
                    cases.push(node(format!("case ({},):", names.join(", "))).with(body));
                    continue;
                }
                Ty::Class(c, a) if a.is_empty() && self.classes[*c].tparams.is_empty() => {
                    let c = *c;
                    let fs = self.all_fields(c);
                    let subs_in_union = ms.iter().any(|x| matches!(x, Ty::Class(k, _) if *k != c && self.ancestors(*k).contains(&c)));
                    if subs_in_union {
                        continue;
                    }
                    let positional = match self.classes[c].kind {
                        ClassKind::NamedTuple | ClassKind::Dataclass { kw_only: false, .. } => true,
                        _ => false,
                    };
                    if positional && !fs.is_empty() && g.bool_with(0.4) {
                        let k = g.int(1..=fs.len());
                        let names: Vec<String> = (0..k).map(|_| self.fresh("c")).collect();
                        let mut a = sc.clone();
                        for (n, (_, ft)) in names.iter().zip(&fs) {
                            a.bind(n.clone(), ft.clone(), false);
                        }
                        let mut body: Vec<Tree> = names.iter().zip(&fs).filter(|(_, (_, ft))| self.probe_safe(ft)).map(|(n, _)| n.clone()).collect::<Vec<_>>().iter().map(|n| self.probe_line(n)).collect();
                        body.extend(self.block(g, &mut a, ret, depth - 1, (0, 1)));
                        cases.push(node(format!("case {}({}):", self.classes[c].name, names.join(", "))).with(body));
                        continue;
                    }
                    if !fs.is_empty() && g.bool() {
                        let (fname, ft) = g.pick(&fs).clone();
                        let mut a = sc.clone();
                        a.bind(cap.clone(), ft, false);
                        let mut body = vec![self.probe_line(&cap)];
                        body.extend(self.block(g, &mut a, ret, depth - 1, (0, 1)));
                        cases.push(node(format!("case {}({fname}={cap}):", self.classes[c].name)).with(body));
                        continue;
                    }
                    (format!("{}() as {cap}", self.classes[c].name), Some(m.clone()))
                }
                Ty::Enum(e) => {
                    let en = self.enums[*e].clone();
                    // `case E.A` compares with `==`, and a StrEnum/IntEnum member equals its
                    // plain value: ty narrowing such a union to the enum is candidate
                    // bugs/candidates/strenum-eq-narrowing, so don't find it again.
                    if en.flavour != "Enum" && ms.iter().any(|x| !matches!(x, Ty::Enum(_) | Ty::None)) {
                        continue;
                    }
                    let mut body = vec![self.probe_line(&p)];
                    // A value pattern compares with `==`: other members narrow away only
                    // when they are final (a list subclass could equal `E.A`).
                    let to = if ms.iter().all(|x| matches!(x, Ty::Enum(_) | Ty::None | Ty::Bool)) { m.clone() } else { v.ty.clone() };
                    let mut a = sc.narrow(&p, to);
                    body.extend(self.block(g, &mut a, ret, depth - 1, (0, 1)));
                    let mem = g.pick(&en.members).clone();
                    cases.push(node(format!("case {}.{mem}:", en.name)).with(body));
                    continue;
                }
                Ty::List(t) if ms.iter().all(|x| self.seq_pattern_safe(x)) && ms.iter().filter(|x| matches!(x, Ty::Tuple(_) | Ty::TupleVar(_) | Ty::List(_)) || matches!(x, Ty::Class(c, _) if self.classes[*c].kind == ClassKind::NamedTuple)).count() == 1 => {
                    let h = self.fresh("c");
                    let mut a = sc.clone();
                    a.bind(h.clone(), (**t).clone(), false);
                    a.bind(cap.clone(), Ty::List(t.clone()), false);
                    let mut body = vec![self.probe_line(&cap)];
                    if self.probe_safe(t) {
                        body.push(self.probe_line(&h));
                    }
                    body.extend(self.block(g, &mut a, ret, depth - 1, (0, 1)));
                    cases.push(node(format!("case [{h}, *{cap}]:")).with(body));
                    continue;
                }
                // Only members that cannot be a Mapping (final types) besides dicts;
                // the capture can come from any dict member.
                Ty::Dict(k, _) if **k == Ty::Str && ms.iter().all(|x| matches!(x, Ty::Dict(..) | Ty::None | Ty::Bool | Ty::Enum(_))) => {
                    let vals: Vec<Ty> = ms.iter().filter_map(|x| if let Ty::Dict(_, v) = x { Some((**v).clone()) } else { None }).collect();
                    let t = Box::new(Model::union_of(vals));
                    let mut a = sc.clone();
                    a.bind(cap.clone(), (*t).clone(), false);
                    let mut body = Vec::new();
                    if self.probe_safe(&t) {
                        body.push(self.probe_line(&cap));
                    }
                    body.extend(self.block(g, &mut a, ret, depth - 1, (0, 1)));
                    cases.push(node(format!("case {{\"a\": {cap}}}:")).with(body));
                    continue;
                }
                _ => continue,
            };
            let mut a = match &bind {
                Some(t) => {
                    let mut a = sc.clone();
                    a.bind(cap.clone(), t.clone(), false);
                    a
                }
                None => sc.narrow(&p, Ty::None),
            };
            let target = if bind.is_some() { cap.clone() } else { p.clone() };
            let mut body = vec![self.probe_line(&target)];
            body.extend(self.block(g, &mut a, ret, depth - 1, (0, 2)));
            cases.push(node(format!("case {pat}:")).with(body));
        }
        if cases.is_empty() {
            return vec![];
        }
        if g.bool_with(0.7) {
            covered = true;
            let mut a = sc.clone();
            let mut body = vec![self.probe_line(&p)];
            body.extend(self.block(g, &mut a, ret, 0, (0, 1)));
            cases.push(node("case _:").with(body));
        }
        let _ = covered;
        vec![node(format!("match {p}:")).with(cases), self.probe_line(&p)]
    }

    fn loop_stmt(&mut self, g: &mut Source, sc: &mut Scope, ret: Option<&Ty>, depth: usize) -> Vec<Tree> {
        // An accumulator whose type the loop body widens and narrows: ty's fixpoint.
        let acc = self.fresh("acc");
        let t = Model::union_of(vec![self.scalar(g), Ty::None, if g.bool() { self.scalar(g) } else { Ty::None }]);
        let init = self.expr(g, &t, sc, 1);
        let i = self.fresh("i");
        let mut out = vec![node(format!("{acc}: {} = {init}", self.render(&t)))];
        let mut body_sc = sc.clone();
        body_sc.in_loop = true;
        // The header may iterate a list variable: no container mutation in the body.
        body_sc.iterating = true;
        body_sc.bind(acc.clone(), t.clone(), true);
        body_sc.bind(i.clone(), Ty::Int, false);
        let mut body = vec![self.probe_line(&acc)];
        let non_none = Model::union_of(Model::members(&t).into_iter().filter(|m| *m != Ty::None).collect());
        let mut b2 = body_sc.clone();
        let mut ifb = vec![node(format!("{acc} = {}", self.expr(g, &non_none, &body_sc, 1)))];
        ifb.extend(self.block(g, &mut b2, ret, depth - 1, (0, 1)));
        body.push(node(format!("if {acc} is None:")).with(ifb));
        let mut elb = vec![self.probe_line(&acc)];
        if g.bool() {
            elb.push(node(format!("{acc} = {}", self.expr(g, &t, &body_sc, 1))));
            elb.extend(self.block(g, &mut body_sc.clone(), ret, 0, (0, 1)));
        } else {
            let mut b3 = body_sc.narrow(&acc, non_none.clone());
            elb.extend(self.block(g, &mut b3, ret, 0, (0, 1)));
        }
        body.push(node("else:").with(elb));
        if g.bool_with(0.3) {
            body.push(node(format!("if {i} > {}:", g.int(0..=3i32))).with(vec![node(if g.bool() { "break" } else { "continue" })]));
        }
        body.extend(self.block(g, &mut body_sc, ret, depth - 1, (0, 2)));
        let header = match g.int(0..=3u32) {
            0 => format!("for {i} in range({}):", g.int(0..=4i32)),
            1 => {
                let xs = self.expr(g, &Ty::List(Box::new(Ty::Int)), sc, 1);
                format!("for {i} in {xs}:")
            }
            2 => format!("for {i}, _ in enumerate({}):", self.expr(g, &Ty::Str, sc, 1)),
            _ => {
                out.push(node(format!("{i} = 0")));
                body.insert(0, node(format!("{i} += 1")));
                format!("while {i} < {}:", g.int(0..=4i32))
            }
        };
        let mut lp = vec![node(header).with(body)];
        if g.bool_with(0.2) {
            lp.push(node("else:").with(vec![self.probe_line(&acc)]));
        }
        out.extend(lp);
        out.push(self.probe_line(&acc));
        sc.bind(acc, t, true);
        out
    }

    // ----- declarations -------------------------------------------------------------------------

    fn gen_decls(&mut self, g: &mut Source) {
        for k in 0..g.int(0..=2usize) {
            let n = g.int(1..=4usize);
            let flavour = *g.pick(&["Enum", "IntEnum", "StrEnum", "Enum"]);
            let str_values = flavour == "StrEnum" || (flavour == "Enum" && g.bool());
            let functional = g.bool_with(0.35);
            self.enums.push(Enum { name: format!("E{k}"), members: (0..n).map(|i| ["A", "B", "C", "D"][i].to_string()).collect(), str_values, flavour, functional });
        }
        for k in 0..g.int(0..=2usize) {
            let methods = (0..g.int(1..=2usize)).map(|i| (format!("pm{k}_{i}"), self.scalar(g))).collect();
            let attrs = (0..g.int(0..=2usize)).map(|i| (format!("pa{k}_{i}"), self.scalar(g), g.bool())).collect();
            self.protos.push(Proto { name: format!("P{k}"), methods, attrs, runtime: g.bool() });
        }
        for _ in 0..g.int(0..=1usize) {
            let mut t = self.ty(g, 1, &[]);
            while !builtin_only(&t) {
                t = self.ty(g, 1, &[]);
            }
            self.descs.push(t);
        }
        for k in 0..g.int(0..=5usize) {
            let base = if !self.classes.is_empty() && g.bool_with(0.4) {
                let cands: Vec<usize> = (0..self.classes.len()).filter(|&c| self.classes[c].kind == ClassKind::Plain && self.classes[c].tparams.is_empty()).collect();
                if cands.is_empty() { None } else { Some(*g.pick(&cands)) }
            } else {
                None
            };
            // Pydantic models subclass only pydantic models.
            let pyd_bases: Vec<usize> = (0..self.classes.len()).filter(|&c| self.classes[c].kind == ClassKind::Pydantic && self.classes[c].tparams.is_empty()).collect();
            let pyd_base = if base.is_none() && !pyd_bases.is_empty() && g.bool_with(0.35) { Some(*g.pick(&pyd_bases)) } else { None };
            let base = base.or(pyd_base);
            let generic = base.is_none() && g.bool_with(0.25);
            let kind = if pyd_base.is_some() || (generic && g.bool_with(0.3)) {
                ClassKind::Pydantic
            } else if base.is_some() || generic {
                ClassKind::Plain
            } else {
                let (sl, kw) = (g.bool(), g.bool());
                *g.weighted(&[
                    (3, ClassKind::Plain),
                    (2, ClassKind::Dataclass { frozen: false, slots: false, kw_only: false }),
                    (1, ClassKind::Dataclass { frozen: true, slots: sl, kw_only: kw }),
                    (1, ClassKind::NamedTuple),
                    (2, ClassKind::Pydantic),
                ])
            };
            let is_pyd = kind == ClassKind::Pydantic;
            let tparams: Vec<String> = if generic { (0..g.int(1..=2usize)).map(|i| ["T", "U"][i].to_string()).collect() } else { vec![] };
            let tv: Vec<(String, Option<Ty>)> = tparams.iter().map(|t| (t.clone(), None)).collect();
            let fields: Vec<(String, Ty)> = (0..g.int(if generic { tparams.len() } else { 0 }..=3usize))
                .map(|i| {
                    let mut t = if generic && i < tparams.len() { Ty::Var(tparams[i].clone()) } else { self.ty(g, 1, &[]) };
                    // pydantic validates fields: no protocols (isinstance raises for most).
                    while is_pyd && !(self.pyd_safe(&t) || matches!(t, Ty::Var(_))) {
                        t = self.ty(g, 1, &[]);
                    }
                    (format!("f{k}_{i}"), t)
                })
                .collect();
            let mut fields: Vec<(String, Ty)> = fields.into_iter().filter(|(_, t)| !mentions_class_from(t, k)).collect();
            let mut methods = Vec::new();
            for i in 0..g.int(0..=2usize) {
                let mk = *g.weighted(&[(4, MethodKind::Normal), (2, MethodKind::Property), (1, MethodKind::Static), (1, MethodKind::Class)]);
                let mk = if kind == ClassKind::NamedTuple && mk == MethodKind::Class { MethodKind::Normal } else { mk };
                let ret = if generic && g.bool() { Ty::Var(g.pick(&tparams).clone()) } else { self.ty(g, 1, &[]) };
                let ret = if mk != MethodKind::Normal && mk != MethodKind::Property && matches!(ret, Ty::Var(_)) { Ty::Int } else { ret };
                let params = if mk == MethodKind::Property { vec![] } else { (0..g.int(0..=1usize)).map(|_| self.scalar(g)).collect() };
                methods.push(Method { name: format!("m{k}_{i}"), params, ret, kind: mk });
            }
            if !generic && kind != ClassKind::NamedTuple && g.bool_with(0.3) {
                methods.push(Method { name: format!("me{k}"), params: vec![], ret: Ty::Class(k, vec![]), kind: MethodKind::SelfRet });
            }
            // Overrides of a base method, with a covariant return.
            if let Some(b) = base {
                for (_, m) in self.all_methods(b) {
                    if g.bool_with(0.5) && matches!(m.kind, MethodKind::Normal | MethodKind::Class | MethodKind::Property) {
                        let ret = match &m.ret {
                            Ty::Union(ts) if g.bool() => g.pick(ts).clone(),
                            Ty::Float if g.bool() => Ty::Int,
                            Ty::Int if g.bool() => Ty::Bool,
                            t => t.clone(),
                        };
                        methods.push(Method { name: m.name.clone(), params: m.params.clone(), ret, kind: m.kind });
                    }
                }
            }
            let mut pyd = Pyd::default();
            if is_pyd {
                let base_frozen = base.and_then(|b| self.pyd_frozen_opt(b)).unwrap_or(false);
                // A subclass may thaw a frozen model, never freeze a mutable one: code that holds it
                // as its base assigns to its fields (and its protocols' data members stay writable).
                pyd.frozen = if base_frozen {
                    if g.bool_with(0.3) { Some(false) } else { None }
                } else if base.is_none() && g.bool_with(0.3) {
                    Some(true)
                } else if g.bool_with(0.1) {
                    Some(false)
                } else {
                    None
                };
                pyd.extra = if g.bool_with(0.35) { Some(*g.pick(&["allow", "forbid", "ignore"])) } else { None };
                pyd.by_name = *g.weighted(&[
                    (4, (None, None, None)),
                    (2, (Some(true), None, None)),
                    (1, (None, None, Some(true))),
                    (1, (None, Some(false), None)),
                    (1, (Some(true), Some(false), None)),
                    (1, (Some(true), Some(true), None)),
                ]);
                pyd.cfg_style = *g.weighted(&[(3, 0u8), (2, 1), (1, 2)]);
                pyd.settings = base.is_none() && !generic && g.bool_with(0.15);
            }
            let mut protos = Vec::new();
            for (pi, p) in self.protos.iter().enumerate() {
                if g.bool_with(0.5) && !generic {
                    // A base method of the same name must already conform.
                    let inherited: Vec<(usize, Method)> = base.map(|b| self.all_methods(b)).unwrap_or_default();
                    if p.methods.iter().any(|(mn, mt)| inherited.iter().any(|(_, m)| &m.name == mn && (m.kind != MethodKind::Normal || !m.params.is_empty() || !self.sub(&m.ret, mt)))) {
                        continue;
                    }
                    // Data members become fields of exactly the member's type; fields of
                    // frozen dataclasses and NamedTuples are read-only.
                    let read_only = matches!(kind, ClassKind::NamedTuple | ClassKind::Dataclass { frozen: true, .. }) || (is_pyd && (pyd.frozen.or(base.and_then(|b| self.pyd_frozen_opt(b))).unwrap_or(false)));
                    if p.attrs.iter().any(|(_, _, ro)| !ro && read_only) {
                        continue;
                    }
                    let inherited_f: Vec<(String, Ty)> = base.map(|b| self.all_fields(b)).unwrap_or_default();
                    if p.attrs.iter().any(|(an, at, _)| inherited_f.iter().any(|(n, t)| n == an && t != at)) {
                        continue;
                    }
                    for (an, at, _) in &p.attrs {
                        if !inherited_f.iter().any(|(n, _)| n == an) && !fields.iter().any(|(n, _)| n == an) {
                            fields.push((an.clone(), at.clone()));
                        }
                    }
                    protos.push(pi);
                    for (mn, mt) in &p.methods {
                        if !methods.iter().any(|m| &m.name == mn) && !inherited.iter().any(|(_, m)| &m.name == mn) {
                            let ret = if *mt == Ty::Int && g.bool_with(0.3) { Ty::Bool } else { mt.clone() };
                            methods.push(Method { name: mn.clone(), params: vec![], ret, kind: MethodKind::Normal });
                        }
                    }
                }
            }
            let _ = tv;
            let mut extras = Vec::new();
            let mut slots = false;
            if is_pyd {
                if g.bool_with(0.3) {
                    extras.push((format!("cv{k}"), self.scalar(g), Extra::ClassVar));
                }
                if g.bool_with(0.3) {
                    extras.push((format!("_pv{k}"), self.scalar(g), Extra::Private));
                }
                // Protocol data members are not given aliases, defaults or per-field settings.
                let proto_attrs: Vec<String> = protos.iter().flat_map(|&p: &usize| self.protos[p].attrs.iter().map(|a| a.0.clone())).collect();
                for (n, t) in &fields {
                    if proto_attrs.contains(n) {
                        continue;
                    }
                    if g.bool_with(0.25) {
                        pyd.aliases.push(n.clone());
                    }
                    if !contains_var(t) && g.bool_with(0.3) {
                        pyd.defaults.push((n.clone(), g.int(0..=2u8)));
                    }
                    if g.bool_with(0.15) {
                        pyd.frozen_fields.push(n.clone());
                    }
                    // Per-field strictness applies to scalar schemas only (pydantic raises
                    // "Unable to apply constraint 'strict'" for unions, literals, classes...).
                    if matches!(t, Ty::Int | Ty::Str | Ty::Bool | Ty::Float | Ty::Bytes) && g.bool_with(0.2) {
                        let strict = self.strict_safe(t) && g.bool_with(0.7);
                        pyd.strict_fields.push((n.clone(), strict));
                    }
                }
                // Model-wide strict only when every field takes its generated values strictly
                // (lax fields opt out with `strict=False`).
                let all: Vec<(String, Ty)> = base.map(|b| self.all_fields(b)).unwrap_or_default().into_iter().chain(fields.iter().cloned()).collect();
                let lax_ok = |n: &String| pyd.strict_fields.iter().any(|(f, s)| f == n && !s) || base.is_some_and(|b| self.pyd_field_strict(b, n) == Some(false));
                let strict_ok = all.iter().all(|(n, t)| self.strict_safe(t) || lax_ok(n));
                let inherited = base.and_then(|b| self.pyd_strict_opt(b)).unwrap_or(false);
                if strict_ok && g.bool_with(0.25) {
                    pyd.strict = Some(true);
                } else if (inherited && !strict_ok) || g.bool_with(0.1) {
                    pyd.strict = Some(false);
                }
            }
            if kind == ClassKind::Plain && !generic {
                for (i, (fname, ft)) in fields.iter().enumerate() {
                    if g.bool_with(0.3) {
                        extras.push((format!("p{k}_{i}"), ft.clone(), Extra::Setter(fname.clone())));
                    }
                }
                if g.bool_with(0.3) {
                    extras.push((format!("cv{k}"), self.scalar(g), Extra::ClassVar));
                }
                if !self.descs.is_empty() && g.bool_with(0.4) {
                    let d = g.index(self.descs.len());
                    extras.push((format!("d{k}"), self.descs[d].clone(), Extra::Descriptor(d)));
                }
                slots = !fields.is_empty() && g.bool_with(0.3);
            }
            let is_dc = matches!(kind, ClassKind::Dataclass { .. });
            let factory_last = (is_dc || is_pyd) && matches!(fields.last(), Some((_, Ty::List(_) | Ty::Dict(..) | Ty::Set(_)))) && g.bool_with(0.6);
            let initvar = is_dc && g.bool_with(0.3);
            let post_init = initvar || (is_dc && g.bool_with(0.3));
            let init_sub = base.is_none() && kind == ClassKind::Plain && !generic && g.bool_with(0.3);
            self.classes.push(Class { name: format!("C{k}"), base, kind, tparams, fields, methods, protos, is_abstract: false, extras, slots, factory_last, initvar, post_init, init_sub, pyd });
        }
        for k in 0..g.int(0..=3usize) {
            let generic = g.bool_with(0.4);
            let (tparam, target) = if generic {
                let v = Ty::Var("T".into());
                let t = match g.int(0..=4u32) {
                    0 => Ty::List(Box::new(v)),
                    1 => Ty::Union(vec![v, Ty::None]),
                    2 => Ty::Tuple(vec![v.clone(), v]),
                    3 => Ty::Dict(Box::new(Ty::Str), Box::new(v)),
                    _ => Ty::TupleVar(Box::new(v)),
                };
                (Some("T".to_string()), t)
            } else {
                let mut t = self.ty(g, 2, &[]);
                while matches!(t, Ty::Int | Ty::Str | Ty::Bool | Ty::Bytes | Ty::Float) && g.bool_with(0.7) {
                    t = self.ty(g, 2, &[]);
                }
                (None, t)
            };
            // Eager aliases may only name builtins (they come before the classes).
            let style = if generic || !builtin_only(&target) { 0 } else { *g.pick(&[0u8, 1, 2]) };
            self.aliases.push(Alias { name: format!("A{k}"), tparam, target, style });
        }
        for k in 0..g.int(0..=2usize) {
            let t = self.ty(g, 1, &[]);
            self.gens.push((format!("gen{k}"), t, g.int(0..=2u8)));
        }
        for k in 0..g.int(0..=2usize) {
            let t = self.ty(g, 1, &[]);
            self.iters.push((format!("It{k}"), t, *g.pick(&[0u8, 1, 2, 3])));
        }
        for k in 0..g.int(0..=2usize) {
            let t = self.ty(g, 1, &[]);
            self.agens.push((format!("agen{k}"), t, g.int(0..=2u8)));
        }
        for k in 0..g.int(0..=1usize) {
            let t = self.ty(g, 1, &[]);
            self.aiters.push((format!("AIt{k}"), t));
        }
        for k in 0..g.int(0..=1usize) {
            let t = self.ty(g, 1, &[]);
            self.seqs.push((format!("Sq{k}"), t));
        }
        for k in 0..g.int(0..=3usize) {
            let total = g.bool_with(0.7);
            let base = if k > 0 && g.bool_with(0.4) { Some(g.index(k)) } else { None };
            let fields: Vec<(String, Ty, bool)> = (0..g.int(1..=3usize)).map(|i| (format!("k{k}_{i}"), self.ty(g, 1, &[]), (i == 0 && base.is_none()) || g.bool())).collect();
            let readonly = fields.iter().filter(|_| g.bool_with(0.2)).map(|f| f.0.clone()).collect();
            let functional = base.is_none() && g.bool_with(0.3);
            self.tds.push(TypedDictDef { name: format!("TD{k}"), fields, base, total, readonly, functional });
        }
    }

    fn class_tree(&mut self, g: &mut Source, c: usize) -> Tree {
        let cls = self.classes[c].clone();
        let mut bases: Vec<String> = Vec::new();
        if let Some(b) = cls.base {
            bases.push(self.classes[b].name.clone());
        }
        if cls.kind == ClassKind::NamedTuple {
            bases.push("NamedTuple".into());
        }
        if cls.kind == ClassKind::NamedTuple && cls.methods.is_empty() && cls.base.is_none() && g.bool_with(0.5) {
            let fs = &cls.fields;
            return node(if g.bool_with(0.7) {
                format!("{} = NamedTuple({:?}, [{}])", cls.name, cls.name, join(fs.iter().map(|(n, t)| format!("({n:?}, {})", self.render(t))), ", "))
            } else {
                format!("{} = collections.namedtuple({:?}, [{}])", cls.name, cls.name, join(fs.iter().map(|(n, _)| format!("{n:?}")), ", "))
            });
        }
        // Not with data members: a field annotation does not override an inherited
        // protocol property (no setter, abstract), and dataclasses take it as a default.
        let explicit_protos = !matches!(cls.kind, ClassKind::NamedTuple | ClassKind::Pydantic) && cls.protos.iter().all(|p| self.protos[*p].attrs.is_empty()) && g.bool_with(0.4);
        if explicit_protos {
            for p in &cls.protos {
                bases.push(self.protos[*p].name.clone());
            }
        }
        let legacy = !cls.tparams.is_empty() && g.bool_with(0.4);
        if legacy {
            bases.push(format!("Generic[{}]", cls.tparams.join(", ")));
        }
        let tps = if cls.tparams.is_empty() || legacy { String::new() } else { format!("[{}]", cls.tparams.join(", ")) };
        if cls.base.is_some() && self.ancestors(c).iter().any(|&a| self.classes[a].init_sub) && g.bool_with(0.7) {
            bases.push(format!("tag={:?}", cls.name));
        }
        // Pydantic config: `model_config = ConfigDict(...)`, a dict literal, or class keywords.
        let mut pyd_body: Vec<Tree> = Vec::new();
        if cls.kind == ClassKind::Pydantic {
            if cls.base.is_none() {
                bases.insert(0, if cls.pyd.settings { "BaseSettings".into() } else { "BaseModel".into() });
            }
            let p = &cls.pyd;
            let mut cfg: Vec<(&str, String)> = Vec::new();
            let b = |v: bool| if v { "True".to_string() } else { "False".to_string() };
            if let Some(f) = p.frozen {
                cfg.push(("frozen", b(f)));
            }
            if let Some(e) = p.extra {
                cfg.push(("extra", format!("{e:?}")));
            }
            if let Some(v) = p.by_name.0 {
                cfg.push(("validate_by_name", b(v)));
            }
            if let Some(v) = p.by_name.1 {
                cfg.push(("validate_by_alias", b(v)));
            }
            if let Some(v) = p.by_name.2 {
                cfg.push(("populate_by_name", b(v)));
            }
            if let Some(v) = p.strict {
                cfg.push(("strict", b(v)));
            }
            if cls.fields.iter().any(|(_, t)| self.pyd_arbitrary(t)) {
                cfg.push(("arbitrary_types_allowed", "True".into()));
            }
            if !cfg.is_empty() {
                match p.cfg_style {
                    1 => bases.extend(cfg.iter().map(|(k, v)| format!("{k}={v}"))),
                    2 => pyd_body.push(node(format!("model_config = {{{}}}", join(cfg.iter().map(|(k, v)| format!("{k:?}: {v}")), ", ")))),
                    _ => pyd_body.push(node(format!("model_config = {}({})", if self.ancestors(c).iter().any(|&k| self.classes[k].pyd.settings) { "SettingsConfigDict" } else { "ConfigDict" }, join(cfg.iter().map(|(k, v)| format!("{k}={v}")), ", ")))),
                }
            }
            for (i, (n, t)) in cls.fields.iter().enumerate() {
                let r = self.render(t);
                let mut meta: Vec<String> = Vec::new();
                if p.aliases.contains(n) {
                    meta.push(format!("alias=\"{n}_al\""));
                }
                if p.frozen_fields.contains(n) {
                    meta.push("frozen=True".into());
                }
                let strict = p.strict_fields.iter().find(|(f, _)| f == n).map(|(_, s)| *s);
                let strict_marker = strict.is_some() && g.bool_with(0.4);
                if let (Some(sv), false) = (strict, strict_marker) {
                    meta.push(format!("strict={}", if sv { "True" } else { "False" }));
                }
                let factory = cls.factory_last && i + 1 == cls.fields.len();
                let dflt = if factory { None } else { p.defaults.iter().find(|(f, _)| f == n).map(|(_, st)| *st) };
                let in_annotated = !meta.is_empty() && g.bool_with(0.3);
                let mut ann_parts: Vec<String> = Vec::new();
                if strict_marker {
                    ann_parts.push(if strict == Some(true) { "Strict()".into() } else { "Strict(False)".into() });
                }
                let mut rhs_field: Vec<String> = Vec::new();
                if in_annotated {
                    ann_parts.push(format!("Field({})", meta.join(", ")));
                } else {
                    rhs_field = meta;
                }
                let ann = if ann_parts.is_empty() { r } else { format!("Annotated[{r}, {}]", ann_parts.join(", ")) };
                let rhs = if factory {
                    let f = match t { Ty::List(_) => "list", Ty::Dict(..) => "dict", _ => "set" };
                    rhs_field.insert(0, format!("default_factory={f}"));
                    Some(format!("Field({})", rhs_field.join(", ")))
                } else if let Some(st) = dflt {
                    let v = self.expr(g, t, &Scope::default(), 0);
                    // A value naming this class or a later one (`type[C0]` may be its subclass
                    // C1) is not defined yet: compute it at instantiation instead.
                    let later = v.split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_')).any(|w| w.strip_prefix('C').and_then(|d| d.parse::<usize>().ok()).is_some_and(|j| j >= c));
                    let (v, st) = if later { (format!("default_factory=lambda: {v}"), 3) } else { (v, st) };
                    match st {
                        0 if rhs_field.is_empty() => Some(v),
                        2 | 3 => {
                            rhs_field.insert(0, v);
                            Some(format!("Field({})", rhs_field.join(", ")))
                        }
                        _ => {
                            rhs_field.insert(0, format!("default={v}"));
                            Some(format!("Field({})", rhs_field.join(", ")))
                        }
                    }
                } else if !rhs_field.is_empty() {
                    Some(format!("Field({})", rhs_field.join(", ")))
                } else {
                    None
                };
                pyd_body.push(node(match rhs { Some(v) => format!("{n}: {ann} = {v}"), None => format!("{n}: {ann}") }));
            }
            for (n, t, x) in cls.extras.clone() {
                let e = self.expr(g, &t, &Scope::default(), 0);
                match x {
                    Extra::ClassVar => pyd_body.push(node(format!("{n}: ClassVar[{}] = {e}", self.render(&t)))),
                    Extra::Private => pyd_body.push(node(format!("{n}: {} = {e}", self.render(&t)))),
                    _ => {}
                }
            }
        }
        let head = if bases.is_empty() { format!("class {}{tps}:", cls.name) } else { format!("class {}{tps}({}):", cls.name, bases.join(", ")) };
        let mut body: Vec<Tree> = pyd_body;
        let self_sc = |m: &Model, extra: &[(String, Ty)]| {
            let mut s = Scope { tvars: cls.tparams.iter().map(|t| (t.clone(), None)).collect(), ..Scope::default() };
            for (n, t) in m.all_fields(c) {
                s.bind(format!("self.{n}"), t, false);
            }
            for (n, t) in extra {
                s.bind(n.clone(), t.clone(), false);
            }
            s
        };
        match cls.kind {
            ClassKind::Plain => {
                let all = self.all_fields(c);
                if !cls.fields.is_empty() {
                    for (n, t) in &cls.fields {
                        body.push(node(format!("{n}: {}", self.render(t))));
                    }
                    let params = join(all.iter().map(|(n, t)| format!("{n}: {}", self.render(t))), ", ");
                    let mut init = Vec::new();
                    if let Some(b) = cls.base {
                        let bf = self.all_fields(b);
                        let sup = if g.bool_with(0.3) { format!("super({}, self)", cls.name) } else { "super()".to_string() };
                        init.push(node(format!("{sup}.__init__({})", join(bf.iter().map(|(n, _)| n.clone()), ", "))));
                    }
                    for (n, _) in &cls.fields {
                        init.push(node(format!("self.{n} = {n}")));
                    }
                    body.push(node(format!("def __init__(self, {params}) -> None:")).with(init));
                }
                if cls.slots {
                    body.insert(0, node(format!("__slots__ = ({})", join(cls.fields.iter().map(|(n, _)| format!("{n:?}, ")), ""))));
                }
                for (n, t, x) in cls.extras.clone() {
                    let r = self.render(&t);
                    match x {
                        Extra::Setter(f) => {
                            body.push(node("@property"));
                            body.push(node(format!("def {n}(self) -> {r}:")).with(vec![node(format!("return self.{f}"))]));
                            body.push(node(format!("@{n}.setter")));
                            body.push(node(format!("def {n}(self, v: {r}) -> None:")).with(vec![node(format!("self.{f} = v"))]));
                        }
                        Extra::ClassVar => {
                            let e = self.expr(g, &t, &Scope::default(), 0);
                            body.push(node(format!("{n}: ClassVar[{r}] = {e}")));
                        }
                        Extra::Descriptor(d) => body.push(node(format!("{n} = D{d}()"))),
                        Extra::Private => {}
                    }
                }
            }
            ClassKind::Pydantic => {}
            ClassKind::Dataclass { .. } | ClassKind::NamedTuple => {
                for (i, (n, t)) in cls.fields.iter().enumerate() {
                    if cls.factory_last && i + 1 == cls.fields.len() {
                        let f = match t { Ty::List(_) => "list", Ty::Dict(..) => "dict", _ => "set" };
                        body.push(node(format!("{n}: {} = field(default_factory={f})", self.render(t))));
                    } else {
                        body.push(node(format!("{n}: {}", self.render(t))));
                    }
                }
                if cls.initvar {
                    body.push(node("iv: InitVar[int] = 0"));
                }
                if cls.post_init {
                    let mut pb = Vec::new();
                    for (n, _) in self.all_fields(c) {
                        if g.bool_with(0.5) {
                            pb.push(self.probe_line(&format!("self.{n}")));
                        }
                    }
                    if cls.initvar {
                        pb.push(node("str(iv)"));
                    }
                    pb.push(node("pass"));
                    let ps = if cls.initvar { "self, iv: int" } else { "self" };
                    body.push(node(format!("def __post_init__({ps}) -> None:")).with(pb));
                }
            }
        }
        if cls.init_sub {
            body.push(node("def __init_subclass__(cls, tag: str = \"\", **kw: object) -> None:").with(vec![node("super().__init_subclass__()")]));
        }
        for m in &cls.methods {
            let ps: Vec<(String, Ty)> = m.params.iter().enumerate().map(|(i, t)| (format!("a{i}"), t.clone())).collect();
            let sig = join(ps.iter().map(|(n, t)| format!("{n}: {}", self.render(t))), ", ");
            let first = match m.kind {
                MethodKind::Static => String::new(),
                MethodKind::Class => "cls".into(),
                _ => "self".into(),
            };
            let all_ps = if first.is_empty() { sig.clone() } else if sig.is_empty() { first.clone() } else { format!("{first}, {sig}") };
            let mut sc = if m.kind == MethodKind::Static || m.kind == MethodKind::Class { Scope::default() } else { self_sc(self, &ps) };
            if m.kind == MethodKind::Static || m.kind == MethodKind::Class {
                for (n, t) in &ps {
                    sc.bind(n.clone(), t.clone(), false);
                }
            }
            if m.kind == MethodKind::SelfRet {
                let mut mb = self.block(g, &mut sc, None, 1, (0, 1));
                mb.push(node("return self"));
                body.push(node(format!("def {}(self) -> Self:", m.name)).with(mb));
                continue;
            }
            let mut mb = self.block(g, &mut sc, Some(&m.ret), 1, (0, 2));
            // An override with the base's return type can delegate to `super()`.
            let base_m = cls.base.and_then(|b| self.all_methods(b).into_iter().find(|(_, bm)| bm.name == m.name && bm.kind == m.kind && matches!(m.kind, MethodKind::Normal | MethodKind::Class | MethodKind::Property)));
            if let Some((_, bm)) = base_m.filter(|(_, bm)| bm.ret == m.ret && !contains_var(&m.ret)).filter(|_| g.bool_with(0.6)) {
                let args = join(ps.iter().map(|(n, _)| n.clone()), ", ");
                let recv = if m.kind == MethodKind::Class { "cls" } else { "self" };
                let call = match (m.kind, g.bool_with(0.6)) {
                    (MethodKind::Property, true) => format!("super().{}", bm.name),
                    (MethodKind::Property, false) => format!("super({}, self).{}", cls.name, bm.name),
                    (_, true) => format!("super().{}({args})", bm.name),
                    (_, false) => format!("super({}, {recv}).{}({args})", cls.name, bm.name),
                };
                let sv = self.fresh("s");
                mb.push(node(format!("{sv} = {call}")));
                BOUND.with(|b| b.borrow_mut().insert(sv.clone(), m.ret.clone()));
                mb.push(self.probe_line(&sv));
                mb.push(node(format!("return {sv}")));
            } else if matches!(m.ret, Ty::Var(_)) && !sc.vars.iter().any(|v| v.ty == m.ret) {
                mb = vec![node("raise NotImplementedError")];
            } else {
                mb.push(node(format!("return {}", self.expr(g, &m.ret, &sc, 2))));
            }
            // A pydantic property may be a `@computed_field` (decided by name, so that an
            // override is one too).
            let computed = cls.kind == ClassKind::Pydantic && m.name.bytes().map(u32::from).sum::<u32>() % 3 == 0;
            let deco = match m.kind {
                MethodKind::Property if computed => vec![node("@computed_field"), node("@property")],
                MethodKind::Property => vec![node("@property")],
                MethodKind::Static => vec![node("@staticmethod")],
                MethodKind::Class => vec![node("@classmethod")],
                MethodKind::Normal | MethodKind::SelfRet => vec![],
            };
            body.extend(deco);
            let overrides = cls.base.is_some_and(|b| self.all_methods(b).iter().any(|(_, bm)| bm.name == m.name));
            if overrides && g.bool_with(0.5) {
                body.push(node("@override"));
            }
            body.push(node(format!("def {}({all_ps}) -> {}:", m.name, self.render(&m.ret))).with(mb));
        }
        if body.is_empty() {
            body.push(node("pass"));
        }
        let mut t = node(head).with(body);
        if let ClassKind::Dataclass { frozen, slots, kw_only } = cls.kind {
            let mut args = Vec::new();
            if frozen {
                args.push("frozen=True");
            }
            if slots {
                args.push("slots=True");
            }
            if kw_only {
                args.push("kw_only=True");
            }
            let deco = if args.is_empty() { "@dataclass".into() } else { format!("@dataclass({})", args.join(", ")) };
            t = node(deco).with(vec![]).then(t);
        }
        if !self.classes.iter().any(|k| k.base == Some(c)) && g.bool_with(0.3) {
            t = node("@final").then(t);
        }
        if cls.kind == ClassKind::Plain && cls.protos.iter().any(|p| self.protos[*p].runtime) && !explicit_protos {
            // nothing: structural only
        }
        t
    }

    fn iter_tree(&mut self, g: &mut Source, it: &(String, Ty, u8)) -> Tree {
        let (name, t, style) = it;
        let r = self.render(t);
        let init = node("def __init__(self, xs: list[".to_string() + &r + "]) -> None:").with(vec![node("self.xs = xs"), node("self.i = 0")]);
        let body = match style {
            // `__iter__` returning a fresh iterator.
            0 => vec![init, node(format!("def __iter__(self) -> Iterator[{r}]:")).with(vec![node("return iter(self.xs)")])],
            // An iterator class: `__iter__` returns self, `__next__` raises StopIteration.
            1 => vec![
                init,
                node(format!("def __iter__(self) -> \"{name}\":")).with(vec![node("return self")]),
                node(format!("def __next__(self) -> {r}:")).with(vec![
                    node("if self.i >= len(self.xs):").with(vec![node("raise StopIteration")]),
                    node("self.i += 1"),
                    node("return self.xs[self.i - 1]"),
                ]),
            ],
            // An iterator class whose `__iter__` returns `Self`.
            3 => vec![
                init,
                node("def __iter__(self) -> Self:").with(vec![node("return self")]),
                node(format!("def __next__(self) -> {r}:")).with(vec![
                    node("if self.i >= len(self.xs):").with(vec![node("raise StopIteration")]),
                    node("self.i += 1"),
                    node("return self.xs[self.i - 1]"),
                ]),
            ],
            // The old sequence protocol: `__getitem__` raising IndexError.
            _ => vec![init, node(format!("def __getitem__(self, i: int) -> {r}:")).with(vec![node("return self.xs[i]")]), node("def __len__(self) -> int:").with(vec![node("return len(self.xs)")])],
        };
        let _ = g;
        node(format!("class {name}:")).with(body)
    }

    fn gen_tree(&mut self, g: &mut Source, k: usize) -> Tree {
        let (name, t, style) = self.gens[k].clone();
        let r = self.render(&t);
        let ann = match style {
            0 => format!("Iterator[{r}]"),
            1 => format!("Generator[{r}, None, None]"),
            _ => format!("Iterable[{r}]"),
        };
        let sc = Scope { vars: vec![], ..Scope::default() };
        let mut sc = sc;
        sc.bind("n", Ty::Int, false);
        let mut body = Vec::new();
        let i = self.fresh("i");
        let e = self.expr(g, &t, &sc, 2);
        body.push(node(format!("for {i} in range(n):")).with(vec![node(format!("yield {e}"))]));
        if g.bool() {
            let es: Vec<String> = (0..g.int(0..=2usize)).map(|_| self.expr(g, &t, &sc, 1)).collect();
            body.push(node(format!("yield from [{}]", es.join(", "))));
        }
        // Delegation to an earlier generator of the same element type.
        if let Some((o, _, _)) = self.gens[..k].iter().find(|(_, u, _)| *u == t) {
            body.push(node(format!("yield from {o}(n)")));
        }
        if g.bool_with(0.3) {
            let x = self.fresh("y");
            body.push(node(format!("{x} = yield {}", self.expr(g, &t, &sc, 1))));
            body.push(node(format!("str({x})")));
        }
        node(format!("def {name}(n: int) -> {ann}:")).with(body)
    }

    /// An iterable expression with element type `t`... any element type: (expr, elem type).
    fn iterable_src(&mut self, g: &mut Source, sc: &Scope) -> Option<(String, Ty)> {
        let mut opts: Vec<(String, Ty)> = Vec::new();
        for v in &sc.vars {
            match &v.ty {
                Ty::List(t) | Ty::Set(t) | Ty::TupleVar(t) => opts.push((v.path.clone(), (**t).clone())),
                Ty::Dict(k, t) => {
                    opts.push((v.path.clone(), (**k).clone()));
                    opts.push((format!("{}.values()", v.path), (**t).clone()));
                    opts.push((format!("{}.items()", v.path), Ty::Tuple(vec![(**k).clone(), (**t).clone()])));
                }
                Ty::Tuple(ts) if !ts.is_empty() => opts.push((v.path.clone(), Model::union_of(ts.clone()))),
                Ty::Str => opts.push((v.path.clone(), Ty::Str)),
                _ => {}
            }
        }
        for (n, t, _) in self.gens.clone() {
            let k = g.int(0..=3i32);
            opts.push((format!("{n}({k})"), t));
        }
        for (n, t, _) in self.iters.clone() {
            let es: Vec<String> = (0..g.int(0..=3usize)).map(|_| self.expr(g, &t, sc, 1)).collect();
            opts.push((format!("{n}([{}])", es.join(", ")), t));
        }
        opts.retain(|(_, t)| vars_inhabited(t, sc) && !contains_var(t));
        if opts.is_empty() {
            return None;
        }
        // Sometimes a union of two iterables.
        if opts.len() >= 2 && g.bool_with(0.2) {
            let (a, ta) = g.pick(&opts).clone();
            let (b, tb) = g.pick(&opts).clone();
            let c = self.expr(g, &Ty::Bool, sc, 1);
            return Some((format!("({a} if {c} else {b})"), Model::union_of(vec![ta, tb])));
        }
        Some(g.pick(&opts).clone())
    }

    /// `try: if <x is None / not isinstance(x, C)>: fail_(..)` then `x` narrowed
    /// (`fail_` returns `NoReturn` / `Never`), `except ValueError`.
    fn noreturn_stmt(&mut self, g: &mut Source, sc: &mut Scope, ret: Option<&Ty>, depth: usize) -> Vec<Tree> {
        let Some(v) = self.narrowable(g, sc) else { return vec![] };
        let ms = Model::members(&v.ty);
        let p = v.path.clone();
        let mut arms: Vec<(String, Ty)> = Vec::new();
        if ms.contains(&Ty::None) && ms.len() >= 2 {
            arms.push((format!("{p} is None"), Model::union_of(ms.iter().filter(|m| **m != Ty::None).cloned().collect())));
        }
        for m in &ms {
            if let Some((name, yes, no)) = self.split_isinstance(&v.ty, m) {
                arms.push((format!("not isinstance({p}, {name})"), yes.clone()));
                if let Some(no) = no {
                    arms.push((format!("isinstance({p}, {name})"), no));
                }
            }
        }
        if arms.is_empty() {
            return vec![];
        }
        let (cond, t) = g.pick(&arms).clone();
        let f = *g.pick(&["fail_", "fail_never_"]);
        let mut isc = sc.narrow(&p, t);
        let mut body = vec![node(format!("if {cond}:")).with(vec![node(format!("{f}({:?})", p))]), self.probe_line(&p)];
        body.extend(self.block(g, &mut isc, ret, depth - 1, (0, 2)));
        let e = self.fresh("e");
        vec![node("try:").with(body), node(format!("except ValueError as {e}:")).with(vec![node(format!("_probe({}, {e})", { self.probe += 1; self.probe }))])]
    }

    fn seq_tree(&mut self, it: &(String, Ty)) -> Tree {
        let (name, t) = it;
        let r = self.render(t);
        node(format!("class {name}:")).with(vec![
            node(format!("def __init__(self, xs: list[{r}]) -> None:")).with(vec![node("self.xs = xs")]),
            node("@overload").then(node(format!("def __getitem__(self, i: int) -> {r}: ..."))),
            node("@overload").then(node(format!("def __getitem__(self, i: slice) -> list[{r}]: ..."))),
            node(format!("def __getitem__(self, i: int | slice) -> {}:", self.render(&Model::union_of(vec![t.clone(), Ty::List(Box::new(t.clone()))])))).with(vec![
                node("if isinstance(i, int):").with(vec![node("return self.xs[i % len(self.xs)]")]),
                node("return self.xs[i]"),
            ]),
            node(format!("def __setitem__(self, i: int, v: {r}) -> None:")).with(vec![node("self.xs[i % len(self.xs)] = v")]),
            node("def __len__(self) -> int:").with(vec![node("return len(self.xs)")]),
            node("def __getattr__(self, name: str) -> int:").with(vec![node("return len(name)")]),
            node("def __class_getitem__(cls, item: object) -> str:").with(vec![node("return \"alias\"")]),
        ])
    }

    /// Subscripts of a user sequence (never empty): `s[i]`, `s[-1]`, slices,
    /// `s[i] = e`, `s.<missing attr>` through `__getattr__`, `Sq[int]`.
    fn seq_stmt(&mut self, g: &mut Source, sc: &mut Scope) -> Vec<Tree> {
        let (name, t) = g.pick(&self.seqs).clone();
        if !vars_inhabited(&t, sc) || contains_var(&t) {
            return vec![];
        }
        let s = self.fresh("sq");
        let es: Vec<String> = (0..g.int(1..=3usize)).map(|_| self.expr(g, &t, sc, 1)).collect();
        let mut out = vec![node(format!("{s} = {name}([{}])", es.join(", ")))];
        let x = self.fresh("v");
        let i = self.expr(g, &Ty::Int, sc, 1);
        out.push(node(format!("{x} = {s}[{i}]")));
        sc.bind(x.clone(), t.clone(), false);
        out.push(self.probe_line(&x));
        for _ in 0..g.int(1..=3usize) {
            let e = match g.int(0..=5u32) {
                0 => format!("{s}[-1]"),
                1 => format!("{s}[{}:]", g.int(0..=2u32)),
                2 => format!("{s}[::-1]"),
                3 => format!("{s}.{}", g.pick(&["zz", "missing", "q"])),
                4 => format!("{name}[int]"),
                _ => {
                    let e = self.expr(g, &t, sc, 1);
                    out.push(node(format!("{s}[0] = {e}")));
                    format!("{s}[0]")
                }
            };
            out.push(self.probe_line(&e));
        }
        out
    }

    /// Calls of the prelude's generic helpers (`apply_`, `compose_`, `Box_.map`,
    /// `take_` over the generic protocol `Getter_`, constrained / bounded `pick_`, `mx_`).
    fn generic_stmt(&mut self, g: &mut Source, sc: &mut Scope) -> Vec<Tree> {
        let fs: Vec<Func> = self
            .funcs
            .iter()
            .filter(|f| !f.sig.has_async && f.tparams.is_empty() && f.guard.is_none() && f.sig.overloads.is_empty() && f.sig.kwonly == 0 && f.sig.unpack.is_none() && f.params.len() == 1 && !contains_var(&f.ret))
            // `T` is solved from the argument's inferred type too: only parameters it cannot exceed.
            .filter(|f| self.inference_stable(&f.params[0].1))
            .filter(|f| !(self.in_async && f.sig.has_async))
            .cloned()
            .collect();
        let vs: Vec<Var> = sc.vars.iter().filter(|v| !contains_var(&v.ty) && self.inference_stable(&v.ty)).cloned().collect();
        let mut out = Vec::new();
        let r = self.fresh("v");
        match g.int(0..=5u32) {
            0 | 1 | 2 if !fs.is_empty() => {
                let f = g.pick(&fs).clone();
                let (p, ret) = (f.params[0].1.clone(), f.ret.clone());
                let x = self.expr(g, &p, sc, 1);
                let e = match g.int(0..=2u32) {
                    0 => format!("apply_({}, {x})", f.name),
                    // `Box_(x)` infers `T` from `x` alone: only for inference-stable params.
                    1 if self.inference_stable(&p) => format!("Box_({x}).map({}).get()", f.name),
                    1 => format!("apply_({}, {x})", f.name),
                    _ => {
                        // compose with a second function taking the first's result
                        let hs: Vec<Func> = fs.iter().filter(|h| self.sub(&ret, &h.params[0].1)).cloned().collect();
                        if hs.is_empty() {
                            format!("apply_({}, {x})", f.name)
                        } else {
                            let h = g.pick(&hs).clone();
                            let e = format!("compose_({}, {})({x})", f.name, h.name);
                            out.push(node(format!("{r} = {e}")));
                            sc.bind(r.clone(), h.ret.clone(), false);
                            out.push(self.probe_line(&r));
                            return out;
                        }
                    }
                };
                out.push(node(format!("{r} = {e}")));
                sc.bind(r.clone(), ret, false);
                out.push(self.probe_line(&r));
            }
            3 if !vs.is_empty() => {
                let v = g.pick(&vs).clone();
                let e = match g.int(0..=2u32) {
                    0 => format!("take_(Box_({}))", v.path),
                    1 => format!("Box_({}).get()", v.path),
                    _ => format!("Box_({}).map(lambda z: z).get()", v.path),
                };
                out.push(node(format!("{r} = {e}")));
                sc.bind(r.clone(), v.ty.clone(), false);
                out.push(self.probe_line(&r));
            }
            4 if vs.len() >= 2 => {
                let (a, b) = (g.pick(&vs).clone(), g.pick(&vs).clone());
                out.push(node(format!("{r} = Box_({}).pair(Box_({}))", a.path, b.path)));
                sc.bind(r.clone(), Ty::Tuple(vec![a.ty.clone(), b.ty.clone()]), false);
                out.push(self.probe_line(&r));
            }
            _ => {
                let t = g.pick(&[Ty::Int, Ty::Str, Ty::Float]).clone();
                let (a, b) = (self.expr(g, &t, sc, 1), self.expr(g, &t, sc, 1));
                let e = if t == Ty::Float { format!("mx_({a}, {b})") } else { format!("pick_({a}, {b})") };
                out.push(node(format!("{r} = {e}")));
                sc.bind(r.clone(), t, false);
                out.push(self.probe_line(&r));
            }
        }
        out
    }

    /// Probes of operator expressions over variables: tuple concatenation and
    /// repetition, tuple / chained comparisons, `in` on tuples, arithmetic on
    /// int / float / bool, set and dict `|`, indices of unions of tuples.
    fn op_probe_stmt(&mut self, g: &mut Source, sc: &mut Scope) -> Vec<Tree> {
        // Values typed by a type variable have no runtime type assignable to it: none here.
        let safe: Vec<Var> = sc.vars.iter().filter(|v| !contains_var(&v.ty) && self.probe_safe(&v.ty)).cloned().collect();
        let of = |pred: &dyn Fn(&Ty) -> bool| -> Vec<String> { safe.iter().filter(|v| pred(&v.ty)).map(|v| v.path.clone()).collect() };
        let tuples = of(&|t| matches!(t, Ty::Tuple(_) | Ty::TupleVar(_)));
        let ne_tuple_unions = of(&|t| matches!(t, Ty::Union(ms) if ms.iter().all(|m| matches!(m, Ty::Tuple(xs) if !xs.is_empty()))));
        let ints = of(&|t| matches!(t, Ty::Int | Ty::Bool | Ty::Lit(_)) && !matches!(t, Ty::Lit(v) if v.starts_with('"')));
        let floats = of(&|t| matches!(t, Ty::Float));
        let bools = of(&|t| *t == Ty::Bool);
        let strs = of(&|t| *t == Ty::Str || matches!(t, Ty::Lit(v) if v.starts_with('"')));
        let sets = of(&|t| matches!(t, Ty::Set(_)));
        let dicts = of(&|t| matches!(t, Ty::Dict(..)));
        let mut cands: Vec<String> = Vec::new();
        if !tuples.is_empty() {
            let (a, b) = (g.pick(&tuples).clone(), g.pick(&tuples).clone());
            cands.push(format!("{a} + {b}"));
            cands.push(format!("{a} * 2"));
            cands.push(format!("({a}, 1) == ({b}, 2)"));
        }
        if !ne_tuple_unions.is_empty() {
            let a = g.pick(&ne_tuple_unions).clone();
            cands.push(format!("{a}[0]"));
            cands.push(format!("{a}[-1]"));
            cands.push(format!("{a}[:1]"));
        }
        if !ints.is_empty() {
            let (a, b, c) = (g.pick(&ints).clone(), g.pick(&ints).clone(), g.pick(&ints).clone());
            cands.push(format!("{a} < {b} <= {c}"));
            cands.push(format!("({a}, {b}) < ({c}, {a})"));
            cands.push(format!("{a} in ({b}, {c})"));
            cands.push(format!("{a} // 3 + {b} % 5"));
            cands.push(format!("divmod({a}, 7)"));
            cands.push(format!("{a} ** 2 - {b} << 1"));
            cands.push(format!("-{a} | {b} ^ 3"));
            if !floats.is_empty() {
                let f = g.pick(&floats).clone();
                cands.push(format!("{f} + {a}"));
                cands.push(format!("{a} * {f} > {b}"));
            }
        }
        if bools.len() >= 1 {
            let (a, b) = (g.pick(&bools).clone(), g.pick(&bools).clone());
            cands.push(format!("{a} & {b}"));
            cands.push(format!("{a} ^ {b} | {a}"));
            cands.push(format!("{a} + {b}"));
        }
        if !strs.is_empty() {
            let (a, b) = (g.pick(&strs).clone(), g.pick(&strs).clone());
            cands.push(format!("{a} * 2 + {b}"));
            cands.push(format!("{a} in {b}"));
            cands.push(format!("{a} < {b} != {a}"));
            cands.push(format!("({a}, {b}) >= ({b}, {a})"));
        }
        if !sets.is_empty() {
            let (a, b) = (g.pick(&sets).clone(), g.pick(&sets).clone());
            cands.push(format!("{a} | {b}"));
            cands.push(format!("{a} & {b}"));
            if safe.iter().find(|v| v.path == a).map(|v| &v.ty) == safe.iter().find(|v| v.path == b).map(|v| &v.ty) {
                cands.push(format!("{a} - {b}"));
            }
        }
        if !dicts.is_empty() {
            let (a, b) = (g.pick(&dicts).clone(), g.pick(&dicts).clone());
            cands.push(format!("{a} | {b}"));
        }
        if cands.is_empty() {
            return vec![];
        }
        let mut out = Vec::new();
        for _ in 0..g.int(1..=2usize) {
            let e = g.pick(&cands).clone();
            let v = self.fresh("o");
            out.push(node(format!("{v} = {e}")));
            out.push(self.probe_line(&v));
        }
        out
    }

    /// An async iterable expression: (expr, element type).
    fn aiterable_src(&mut self, g: &mut Source, sc: &Scope) -> Option<(String, Ty)> {
        let mut opts: Vec<(String, Ty)> = Vec::new();
        for (n, t, _) in self.agens.clone() {
            let k = g.int(0..=3i32);
            opts.push((format!("{n}({k})"), t));
        }
        for (n, t) in self.aiters.clone() {
            let es: Vec<String> = (0..g.int(0..=3usize)).map(|_| self.expr(g, &t, sc, 1)).collect();
            opts.push((format!("{n}([{}])", es.join(", ")), t));
        }
        opts.retain(|(_, t)| vars_inhabited(t, sc) && !contains_var(t));
        if opts.is_empty() { None } else { Some(g.pick(&opts).clone()) }
    }

    fn agen_tree(&mut self, g: &mut Source, k: usize) -> Tree {
        let (name, t, style) = self.agens[k].clone();
        let r = self.render(&t);
        let ann = match style {
            0 => format!("AsyncIterator[{r}]"),
            1 => format!("AsyncGenerator[{r}, None]"),
            _ => format!("AsyncIterable[{r}]"),
        };
        let mut sc = Scope::default();
        sc.bind("n", Ty::Int, false);
        let i = self.fresh("i");
        let e = self.expr(g, &t, &sc, 2);
        let mut body = vec![node(format!("for {i} in range(n):")).with(vec![node("await asyncio.sleep(0)"), node(format!("yield {e}"))])];
        if let Some((o, _, _)) = self.agens[..k].iter().find(|(_, u, _)| *u == t).cloned() {
            let y = self.fresh("y");
            body.push(node(format!("async for {y} in {o}(n):")).with(vec![node(format!("yield {y}"))]));
        }
        if let Some((o, _, _)) = self.gens.iter().find(|(_, u, _)| *u == t).cloned() {
            let y = self.fresh("y");
            body.push(node(format!("for {y} in {o}(n):")).with(vec![node(format!("yield {y}"))]));
        }
        node(format!("async def {name}(n: int) -> {ann}:")).with(body)
    }

    fn aiter_tree(&mut self, it: &(String, Ty), g: &mut Source) -> Tree {
        let (name, t) = it;
        let r = self.render(t);
        let selfr = if g.bool() { "Self".to_string() } else { format!("\"{name}\"") };
        node(format!("class {name}:")).with(vec![
            node(format!("def __init__(self, xs: list[{r}]) -> None:")).with(vec![node("self.xs = xs"), node("self.i = 0")]),
            node(format!("def __aiter__(self) -> {selfr}:")).with(vec![node("return self")]),
            node(format!("async def __anext__(self) -> {r}:")).with(vec![
                node("if self.i >= len(self.xs):").with(vec![node("raise StopAsyncIteration")]),
                node("self.i += 1"),
                node("return self.xs[self.i - 1]"),
            ]),
        ])
    }

    /// `async def co(): ...` (async for, async comprehensions, awaits) run by `asyncio.run`.
    fn async_stmt(&mut self, g: &mut Source, sc: &mut Scope, depth: usize) -> Vec<Tree> {
        let rt = self.ty_in(g, 1, sc);
        if contains_var(&rt) {
            return vec![];
        }
        let name = self.fresh("co");
        // A coroutine body sees outer bindings at their declared types.
        let mut isc = Scope { vars: sc.vars.iter().filter(|v| !v.path.contains(['.', '[']) && !v.mutable && !v.narrowed).cloned().collect(), frozen: vec![], in_loop: false, tvars: sc.tvars.clone(), iterating: sc.iterating, captured: vec![] };
        isc.captured = isc.vars.iter().map(|v| v.path.clone()).collect();
        let saved = self.in_async;
        self.in_async = true;
        let mut body = Vec::new();
        if let Some((src, t)) = self.aiterable_src(g, &isc) {
            let x = self.fresh("x");
            let mut bsc = isc.clone();
            bsc.in_loop = true;
            bsc.iterating = true;
            bsc.bind(x.clone(), t.clone(), false);
            let mut lb = vec![self.probe_line(&x)];
            lb.extend(self.block(g, &mut bsc, Some(&rt), depth - 1, (0, 2)));
            body.push(node(format!("async for {x} in {src}:")).with(lb));
            let v = self.fresh("v");
            let line = match g.int(0..=3u32) {
                0 => format!("{v} = await anext(aiter({src}), None)"),
                1 => format!("{v} = {{str({x}) async for {x} in {src}}}"),
                2 => format!("{v} = [{x} async for {x} in {src} if bool({x})]"),
                _ => format!("{v} = [{x} async for {x} in {src}]"),
            };
            body.push(node(line));
            body.push(self.probe_line(&v));
        }
        let t = self.ty_in(g, 1, &isc);
        if self.inference_stable(&t) {
            let (a, b) = (self.fresh("v"), self.fresh("v"));
            let e1 = self.expr(g, &t, &isc, 1);
            if g.bool() {
                body.push(node(format!("{a} = await asyncio.sleep(0, {e1})")));
                isc.bind(a.clone(), t.clone(), false);
                body.push(self.probe_line(&a));
            } else {
                let e2 = self.expr(g, &Ty::Int, &isc, 1);
                body.push(node(format!("{a}, {b} = await asyncio.gather(asyncio.sleep(0, {e1}), asyncio.sleep(0, {e2}))")));
                isc.bind(a.clone(), t.clone(), false);
                isc.bind(b.clone(), Ty::Int, false);
                body.push(self.probe_line(&a));
                body.push(self.probe_line(&b));
            }
        }
        body.extend(self.block(g, &mut isc, Some(&rt), depth - 1, (0, 2)));
        body.push(node(format!("return {}", self.expr(g, &rt, &isc, 2))));
        self.in_async = saved;
        self.async_used = true;
        let r = self.fresh("r");
        sc.bind(r.clone(), rt.clone(), false);
        vec![
            node(format!("async def {name}() -> {}:", self.render(&rt))).with(body),
            node(format!("{r} = asyncio.run({name}())")),
            self.probe_line(&r),
        ]
    }

    /// `for x in <iterable>:` with the element probed, over builtins, generators and user iterables.
    fn foreach_stmt(&mut self, g: &mut Source, sc: &mut Scope, ret: Option<&Ty>, depth: usize) -> Vec<Tree> {
        let Some((src, t)) = self.iterable_src(g, sc) else { return vec![] };
        let mut body_sc = sc.clone();
        body_sc.in_loop = true;
        body_sc.iterating = true;
        // The old `__getitem__` protocol works with `for` and comprehensions, not
        // with functions taking an `Iterable`.
        let old_style = self.iters.iter().any(|(n, _, st)| *st == 2 && src.contains(&format!("{n}(")));
        let x = self.fresh("x");
        let header;
        let mut probes = Vec::new();
        match (&t, if old_style { 3 } else { g.int(0..=3u32) }) {
            (Ty::Tuple(ts), 0) if ts.len() == 2 => {
                let (a, b) = (self.fresh("x"), self.fresh("x"));
                header = format!("for {a}, {b} in {src}:");
                body_sc.bind(a.clone(), ts[0].clone(), false);
                body_sc.bind(b.clone(), ts[1].clone(), false);
                probes.push(a);
                probes.push(b);
            }
            (_, 1) => {
                let i = self.fresh("i");
                header = format!("for {i}, {x} in enumerate({src}):");
                body_sc.bind(i.clone(), Ty::Int, false);
                body_sc.bind(x.clone(), t.clone(), false);
                probes.push(i);
                probes.push(x.clone());
            }
            (_, 2) => {
                let Some((src2, t2)) = self.iterable_src(g, sc) else { return vec![] };
                if self.iters.iter().any(|(n, _, st)| *st == 2 && src2.contains(&format!("{n}("))) {
                    return vec![];
                }
                let y = self.fresh("x");
                header = format!("for {x}, {y} in zip({src}, {src2}):");
                body_sc.bind(x.clone(), t.clone(), false);
                body_sc.bind(y.clone(), t2, false);
                probes.push(x.clone());
                probes.push(y);
            }
            _ => {
                header = format!("for {x} in {src}:");
                body_sc.bind(x.clone(), t.clone(), false);
                probes.push(x.clone());
            }
        }
        let mut body: Vec<Tree> = probes.iter().map(|p| self.probe_line(p)).collect();
        body.extend(self.block(g, &mut body_sc, ret, depth - 1, (0, 2)));
        let mut out = vec![node(header).with(body)];
        // Materialize: `list(...)`, a comprehension, or `next(iter(...), None)`.
        let r = self.fresh("v");
        let line = match g.int(0..=9u32) {
            0 if !old_style => format!("{r} = list({src})"),
            2 if !old_style => format!("{r} = next(iter({src}), None)"),
            3 if !old_style => format!("{r} = sorted({src}, key=str)"),
            4 => format!("{r} = sum(1 for {x} in {src})"),
            5 => format!("{r} = {{str({x}) for {x} in {src}}}"),
            6 if !old_style => format!("{r} = {{{x}: 1 for {x} in {src}}} if all(isinstance({x}, (int, str)) for {x} in {src}) else None"),
            7 => format!("{r} = any(bool({x}) for {x} in {src})"),
            8 => format!("{r} = [*{src}, *{src}]"),
            9 => format!("{r} = (*{src},)"),
            _ => format!("{r} = [{x} for {x} in {src}]"),
        };
        out.push(node(line));
        out.push(self.probe_line(&r));
        out
    }

    fn attr_write_stmt(&mut self, g: &mut Source, sc: &mut Scope) -> Vec<Tree> {
        let vs: Vec<(String, usize, Vec<Ty>)> = sc
            .vars
            .iter()
            .filter(|v| !v.path.contains(['[', '(']) && !v.path.starts_with("self.") && !sc.frozen.contains(&v.path) && !sc.captured.contains(&v.path))
            .filter_map(|v| match &v.ty { Ty::Class(c, a) => Some((v.path.clone(), *c, a.clone())), _ => None })
            .collect();
        if vs.is_empty() {
            return vec![];
        }
        let pvs: Vec<(String, usize)> = sc
            .vars
            .iter()
            .filter(|v| !v.path.contains(['[', '(']) && !v.path.starts_with("self.") && !sc.frozen.contains(&v.path) && !sc.captured.contains(&v.path))
            .filter_map(|v| match &v.ty { Ty::Proto(p) => Some((v.path.clone(), *p)), _ => None })
            .collect();
        if !pvs.is_empty() && g.bool_with(0.3) {
            let (p, pi) = g.pick(&pvs).clone();
            let ws: Vec<(String, Ty)> = self.protos[pi].attrs.iter().filter(|a| !a.2).map(|a| (a.0.clone(), a.1.clone())).collect();
            if ws.is_empty() {
                return vec![];
            }
            let (n, t) = g.pick(&ws).clone();
            let e = self.expr(g, &t, sc, 2);
            return vec![node(format!("{p}.{n} = {e}")), self.probe_line(&format!("{p}.{n}"))];
        }
        let (p, c, args) = g.pick(&vs).clone();
        let mut opts: Vec<(String, Ty)> = Vec::new();
        let writable_fields = match self.classes[c].kind {
            ClassKind::Plain | ClassKind::Dataclass { frozen: false, .. } | ClassKind::Pydantic => true,
            _ => false,
        };
        if writable_fields {
            // Not fields behind a property setter: a setter call does not invalidate
            // ty's assignment narrowing of the field.
            let behind: Vec<String> = self.all_extras(c).into_iter().filter_map(|(_, _, x)| if let Extra::Setter(f) = x { Some(f) } else { None }).collect();
            let pyd = self.classes[c].kind == ClassKind::Pydantic;
            for (n, t) in self.all_fields(c) {
                if !behind.contains(&n) && (!pyd || self.pyd_writable(c, &n)) {
                    opts.push((format!("{p}.{n}"), self.subst(c, &args, &t)));
                }
            }
        }
        for (n, t, x) in self.all_extras(c) {
            match x {
                // Through the class: an instance cannot assign a ClassVar.
                Extra::ClassVar => {
                    let owner = self.ancestors(c).into_iter().find(|&k| self.classes[k].extras.iter().any(|e| e.0 == n)).unwrap();
                    opts.push((format!("{}.{n}", self.classes[owner].name), t));
                }
                _ => opts.push((format!("{p}.{n}"), t)),
            }
        }
        if opts.is_empty() {
            return vec![];
        }
        let (target, t) = g.pick(&opts).clone();
        if contains_var(&t) || sc.frozen.contains(&target) {
            return vec![];
        }
        let e = self.expr(g, &t, sc, 2);
        let mut out = vec![node(format!("{target} = {e}"))];
        if self.probe_safe(&t) {
            out.push(self.probe_line(&target));
        }
        out
    }

    fn typed_dict_stmt(&mut self, g: &mut Source, sc: &mut Scope) -> Vec<Tree> {
        let vs: Vec<(String, usize)> = sc.vars.iter().filter_map(|v| match v.ty { Ty::TypedDict(i) if !v.path.contains('[') => Some((v.path.clone(), i)), _ => None }).collect();
        if vs.is_empty() {
            return vec![];
        }
        let (p, i) = g.pick(&vs).clone();
        let fs = self.td_fields(i);
        let (n, t, req) = g.pick(&fs).clone();
        if contains_var(&t) {
            return vec![];
        }
        // A narrowed key view (`if k in td:`) must survive: no deleting or popping
        // keys of a TypedDict that is narrowed in this block.
        let ro = self.td_readonly(i, &n) || sc.frozen.contains(&p) || sc.captured.contains(&p);
        let x = self.fresh("v");
        let r = self.render(&t);
        let mut out = Vec::new();
        let (line, xt) = match g.int(0..=10u32) {
            5 if !req && !ro => (format!("{x}: {} = {p}.pop({n:?}, None)", self.render(&Ty::opt(t.clone()))), Ty::opt(t.clone())),
            6 if !req && !ro => {
                out.push(node(format!("if {n:?} in {p}:")).with(vec![node(format!("del {p}[{n:?}]"))]));
                (format!("{x}: {} = {p}.get({n:?})", self.render(&Ty::opt(t.clone()))), Ty::opt(t.clone()))
            }
            7 if !ro => {
                // ty keeps a narrowing of `p[n]` (from `p = {...}` or `p[n] = ...`) across
                // `p.update(...)`: narrow/match.md "This deliberately documents the remaining
                // unsoundness. Mutating the subject ... we retain them". Re-assign `p[n]` to the
                // same value afterwards so the narrowing ty keeps is the runtime one.
                let e = self.expr(g, &t, sc, 1);
                let y = self.fresh("v");
                out.push(node(format!("{y}: {r} = {e}")));
                out.push(node(format!("{p}.update({{{n:?}: {y}}})")));
                out.push(node(format!("{p}[{n:?}] = {y}")));
                (format!("{x}: {} = {p}.get({n:?})", self.render(&Ty::opt(t.clone()))), Ty::opt(t.clone()))
            }
            8 if !req => {
                // Narrowing by key membership, then a plain read.
                let mut body = vec![node(format!("{x}: {r} = {p}[{n:?}]"))];
                let mut isc = sc.clone();
                isc.bind(x.clone(), t.clone(), false);
                body.push(self.probe_line(&x));
                let _ = isc;
                return vec![node(format!("if {n:?} in {p}:")).with(body)];
            }
            9 => {
                // A mapping pattern on the TypedDict.
                let body = vec![self.probe_line(&x)];
                BOUND.with(|b| b.borrow_mut().insert(x.clone(), t.clone()));
                return vec![node(format!("match {p}:")).with(vec![node(format!("case {{{n:?}: {x}}}:")).with(body), node("case _:").with(vec![node("pass")])])];
            }
            10 => {
                // Assignability to a base TypedDict.
                let Some(b) = self.tds[i].base else { return vec![] };
                let bfs = self.td_fields(b);
                let (bn, bt, breq) = g.pick(&bfs).clone();
                if contains_var(&bt) {
                    return vec![];
                }
                let y = self.fresh("v");
                out.push(node(format!("{y}: {} = {p}", self.tds[b].name)));
                let r2 = self.render(&Ty::opt(bt.clone()));
                if breq {
                    (format!("{x}: {} = {y}[{bn:?}]", self.render(&bt)), bt)
                } else {
                    (format!("{x}: {r2} = {y}.get({bn:?})"), Ty::opt(bt))
                }
            }
            0 if req => (format!("{x}: {r} = {p}[{n:?}]"), t.clone()),
            1 => (format!("{x}: {} = {p}.get({n:?})", self.render(&Ty::opt(t.clone()))), Ty::opt(t.clone())),
            2 => {
                let e = self.expr(g, &t, sc, 1);
                (format!("{x}: {r} = {p}.get({n:?}, {e})"), t.clone())
            }
            3 if !ro => {
                let e = self.expr(g, &t, sc, 1);
                (format!("{x}: {r} = {p}.setdefault({n:?}, {e})"), t.clone())
            }
            4 if !ro => {
                let e = self.expr(g, &t, sc, 1);
                out.push(node(format!("{p}[{n:?}] = {e}")));
                (format!("{x}: {r} = {p}[{n:?}]"), t.clone())
            }
            _ => (format!("{x}: {} = {p}.get({n:?})", self.render(&Ty::opt(t.clone()))), Ty::opt(t.clone())),
        };
        out.push(node(line));
        sc.bind(x.clone(), xt, false);
        out.push(self.probe_line(&x));
        out
    }

    fn gen_funcs(&mut self, g: &mut Source) {
        // Default values come before any body is generated (`has_async` unknown):
        // no module functions as callable values there.
        self.in_async = true;
        self.gen_funcs_inner(g);
        self.in_async = false;
    }

    fn gen_funcs_inner(&mut self, g: &mut Source) {
        for k in 0..g.int(1..=7usize) {
            if g.bool_with(0.12) {
                // A TypeIs/TypeGuard function over a class or scalar.
                let t = if !self.classes.is_empty() && g.bool() {
                    let c = g.index(self.classes.len());
                    if !self.classes[c].tparams.is_empty() { continue; }
                    Ty::Class(c, vec![])
                } else {
                    self.scalar(g)
                };
                let is_typeis = g.bool_with(0.7);
                self.funcs.push(Func { name: format!("g{k}"), tparams: vec![], params: vec![("x".into(), Ty::Object)], ret: Ty::Bool, guard: Some((t, is_typeis)), sig: Sig::default() });
                continue;
            }
            if g.bool_with(0.1) {
                // An overloaded function dispatching on disjoint parameter types.
                let mut pool = vec![Ty::Int, Ty::Str, Ty::Bytes, Ty::None];
                let n = g.int(2..=3usize);
                let mut ovs = Vec::new();
                for _ in 0..n {
                    let i = g.index(pool.len());
                    let pt = pool.remove(i);
                    ovs.push((pt, self.ty(g, 1, &[])));
                }
                let pu = Model::union_of(ovs.iter().map(|o| o.0.clone()).collect());
                let ru = Model::union_of(ovs.iter().map(|o| o.1.clone()).collect());
                self.funcs.push(Func { name: format!("f{k}"), tparams: vec![], params: vec![("x".into(), pu)], ret: ru, guard: None, sig: Sig { overloads: ovs, ..Sig::default() } });
                continue;
            }
            let mut sig = Sig::default();
            let tn = if g.bool_with(0.4) { sig.legacy = true; format!("T{k}") } else { "T".to_string() };
            let tv = Ty::Var(tn.clone());
            let tparams: Vec<(String, Option<Ty>)> = if g.bool_with(0.2) {
                let bound = if g.bool_with(0.3) && !self.classes.is_empty() {
                    let c = g.index(self.classes.len());
                    if self.classes[c].tparams.is_empty() { Some(Ty::Class(c, vec![])) } else { None }
                } else {
                    None
                };
                if bound.is_none() && g.bool_with(0.25) {
                    sig.constraints = vec![Ty::Int, Ty::Str];
                } else if bound.is_none() && g.bool_with(0.2) {
                    sig.tdefault = Some(self.scalar(g));
                }
                vec![(tn.clone(), bound)]
            } else {
                vec![]
            };
            let np = g.int(0..=3usize);
            let mut params: Vec<(String, Ty)> = (0..np).map(|i| (format!("p{i}"), self.ty(g, 2, &tparams))).collect();
            if !tparams.is_empty() && !params.iter().any(|(_, t)| *t == tv) {
                params.push((format!("p{np}"), tv.clone()));
            }
            let ret = if !tparams.is_empty() && g.bool_with(0.6) { tv.clone() } else { self.ty(g, 2, &[]) };
            if tparams.is_empty() {
                // Keyword-only parameters, defaults, `*args`, `**kwargs`.
                sig.kwonly = if g.bool_with(0.3) { g.int(0..=params.len()) } else { 0 };
                sig.posonly = if g.bool_with(0.25) { g.int(0..=params.len() - sig.kwonly) } else { 0 };
                let mut seen_default = false;
                for (i, (_, t)) in params.iter().enumerate() {
                    let kwo = i >= params.len() - sig.kwonly;
                    let d = if (seen_default && !kwo) || g.bool_with(0.25) {
                        seen_default = true;
                        Some(self.expr(g, t, &Scope::default(), 0))
                    } else {
                        None
                    };
                    sig.defaults.push(d);
                }
                if g.bool_with(0.2) {
                    sig.star = Some(self.scalar(g));
                }
                if g.bool_with(0.25) {
                    sig.deco = Some(*g.pick(&["deco_pep695", "deco_legacy"]));
                }
                if !self.tds.is_empty() && g.bool_with(0.15) {
                    sig.unpack = Some(g.index(self.tds.len()));
                } else if g.bool_with(0.15) {
                    sig.kwargs = Some(self.scalar(g));
                }
            }
            self.funcs.push(Func { name: format!("f{k}"), tparams, params, ret, guard: None, sig });
        }
    }

    fn func_tree(&mut self, g: &mut Source, fi: usize) -> Tree {
        let f = self.funcs[fi].clone();
        if let Some((t, is_typeis)) = &f.guard {
            let n = self.isinstance_name(t).unwrap();
            let w = if *is_typeis { "TypeIs" } else { "TypeGuard" };
            // `float` in a type expression is `int | float`: a guard for it must accept ints.
            let n = if *t == Ty::Float { "(int, float)".to_string() } else { n };
            return node(format!("def {}(x: object) -> {w}[{}]:", f.name, self.render(t))).with(vec![node(format!("return isinstance(x, {n})"))]);
        }
        if !f.sig.overloads.is_empty() {
            let mut lines = Vec::new();
            for (pt, rt) in &f.sig.overloads {
                lines.push("@overload".to_string());
                lines.push(format!("def {}(x: {}) -> {}: ...", f.name, self.render(pt), self.render(rt)));
            }
            let mut body = Vec::new();
            let sc = Scope::default();
            let later = self.funcs.split_off(fi);
            self.async_used = false;
            for (pt, rt) in &f.sig.overloads {
                let test = if *pt == Ty::None { "x is None".to_string() } else { format!("isinstance(x, {})", self.isinstance_name(pt).unwrap()) };
                body.push(node(format!("if {test}:")).with(vec![node(format!("return {}", self.expr(g, rt, &sc, 1)))]));
            }
            self.funcs.extend(later);
            self.funcs[fi].sig.has_async = self.async_used;
            body.push(node("raise TypeError"));
            let mut t = node(format!("def {}(x: {}) -> {}:", f.name, self.render(&f.params[0].1), self.render(&f.ret))).with(body);
            for l in lines.into_iter().rev() {
                t = node(l).then(t);
            }
            return t;
        }
        let tp_spec = |m: &Model, n: &String, b: &Option<Ty>| -> String {
            let mut s = n.clone();
            if let Some(b) = b {
                s = format!("{s}: {}", m.render(b));
            } else if !f.sig.constraints.is_empty() {
                s = format!("{s}: ({})", join(f.sig.constraints.iter().map(|c| m.render(c)), ", "));
            }
            if let Some(d) = &f.sig.tdefault {
                s = format!("{s} = {}", m.render(d));
            }
            s
        };
        let tps = if f.tparams.is_empty() || f.sig.legacy {
            String::new()
        } else {
            format!("[{}]", join(f.tparams.iter().map(|(n, b)| tp_spec(self, n, b)), ", "))
        };
        let mut parts: Vec<String> = Vec::new();
        let npos = f.params.len() - f.sig.kwonly;
        for (i, (n, t)) in f.params.iter().enumerate() {
            if i == npos {
                parts.push(if let Some(st) = &f.sig.star { format!("*args: {}", self.render(st)) } else { "*".into() });
            }
            let d = f.sig.defaults.get(i).cloned().flatten().map(|d| format!(" = {d}")).unwrap_or_default();
            parts.push(format!("{n}: {}{d}", self.render(t)));
            if i + 1 == f.sig.posonly {
                parts.push("/".into());
            }
        }
        if npos == f.params.len() {
            if let Some(st) = &f.sig.star {
                parts.push(format!("*args: {}", self.render(st)));
            }
        }
        if let Some(k) = &f.sig.kwargs {
            parts.push(format!("**kw: {}", self.render(k)));
        } else if let Some(u) = f.sig.unpack {
            parts.push(format!("**kw: Unpack[{}]", self.tds[u].name));
        }
        let sig = parts.join(", ");
        let mut sc = Scope { tvars: f.tparams.clone(), ..Scope::default() };
        for (n, t) in &f.params {
            sc.bind(n.clone(), t.clone(), false);
        }
        let mut pre = Vec::new();
        if let Some(st) = &f.sig.star {
            sc.bind("args", Ty::TupleVar(Box::new(st.clone())), false);
            pre.push(self.probe_line("args"));
        }
        if let Some(k) = &f.sig.kwargs {
            sc.bind("kw", Ty::Dict(Box::new(Ty::Str), Box::new(k.clone())), false);
        } else if let Some(u) = f.sig.unpack {
            sc.bind("kw", Ty::TypedDict(u), false);
        }
        // Earlier functions only (no recursion): hide later ones while generating the body.
        let later = self.funcs.split_off(fi);
        let mut body = pre;
        self.async_used = false;
        body.extend(self.block(g, &mut sc, Some(&f.ret), 3, (1, 6)));
        body.push(node(format!("return {}", self.expr(g, &f.ret, &sc, 2))));
        self.funcs.extend(later);
        self.funcs[fi].sig.has_async = self.async_used;
        let mut def = node(format!("def {}{tps}({sig}) -> {}:", f.name, self.render(&f.ret))).with(body);
        if let Some(d) = f.sig.deco {
            def = node(format!("@{d}")).then(def);
        }
        if f.sig.legacy && !f.tparams.is_empty() {
            let (n, b) = &f.tparams[0];
            let mut args = vec![format!("{n:?}")];
            args.extend(f.sig.constraints.iter().map(|c| self.render(c)));
            if let Some(b) = b {
                args.push(format!("bound={}", self.render(b)));
            }
            if let Some(d) = &f.sig.tdefault {
                args.push(format!("default={}", self.render(d)));
            }
            return node(format!("{n} = TypeVar({})", args.join(", "))).then(def);
        }
        def
    }

    fn main_tree(&mut self, g: &mut Source) -> Tree {
        let mut sc = Scope::default();
        let mut body = Vec::new();
        for fi in 0..self.funcs.len() {
            let f = self.funcs[fi].clone();
            for _ in 0..g.int(1..=3usize) {
                let r = self.fresh("r");
                let mut inst_ty = Ty::Object;
                let call = if let Some((t, _)) = &f.guard {
                    let o = if g.bool() { t.clone() } else { self.ty(g, 1, &[]) };
                    let e = self.expr(g, &o, &sc, 2);
                    format!("{}({e})", f.name)
                } else if !f.sig.overloads.is_empty() {
                    let (pt, rt) = g.pick(&f.sig.overloads).clone();
                    inst_ty = rt;
                    format!("{}({})", f.name, self.expr(g, &pt, &sc, 2))
                } else if !f.tparams.is_empty() {
                    // Instantiate T with a concrete type (respecting a bound).
                    let inst = match &f.tparams[0].1 {
                        Some(b) => b.clone(),
                        None if !f.sig.constraints.is_empty() => g.pick(&f.sig.constraints).clone(),
                        None => {
                            let mut t = self.ty(g, 1, &[]);
                            while !self.inference_stable(&t) {
                                t = self.ty(g, 1, &[]);
                            }
                            t
                        }
                    };
                    inst_ty = inst.clone();
                    let tn = f.tparams[0].0.clone();
                    let su = |t: &Ty| subst_var(t, &tn, &inst);
                    // ty solves `T` from a list literal of `Literal`-typed variables without
                    // the other arguments' context: keep those variables out of such calls.
                    let mut sc2 = sc.clone();
                    sc2.vars.retain(|v| !Model::members(&v.ty).iter().any(|m| matches!(m, Ty::Lit(_))));
                    let args: Vec<String> = f.params.iter().map(|(_, p)| { let pt = su(p); self.expr(g, &pt, &sc2, 2) }).collect();
                    format!("{}({})", f.name, args.join(", "))
                } else {
                    let args = self.call_args(g, &f, &sc, 2);
                    format!("{}({args})", f.name)
                };
                body.push(node(format!("{r} = {call}")));
                let rt = if f.guard.is_some() {
                    Ty::Bool
                } else if !f.sig.overloads.is_empty() {
                    inst_ty.clone()
                } else if !f.tparams.is_empty() {
                    // Bounded parameters are solved to the argument's class, which may be a subclass;
                    // `T` inside a union or container parameter may be solved wider than `inst`.
                    let v = Ty::Var(f.tparams[0].0.clone());
                    let ambiguous = f.params.iter().any(|(_, p)| contains_var(p) && *p != v);
                    if f.tparams[0].1.is_some() || ambiguous { Ty::Object } else { subst_var(&f.ret, &f.tparams[0].0, &inst_ty) }
                } else {
                    f.ret.clone()
                };
                // A guard's result has ty's TypeIs/TypeGuard type, and a stored one
                // narrows its argument where it is tested: neither is probed or reused.
                if f.guard.is_some() {
                    continue;
                }
                if self.probe_safe(&rt) && rt != Ty::Object {
                    body.push(self.probe_line(&r));
                }
                if rt != Ty::Object {
                    sc.bind(r, rt, false);
                }
            }
        }
        // A few statements over the results.
        body.extend(self.block(g, &mut sc, None, 2, (1, 4)));
        node("def main() -> None:").with(body)
    }

    pub fn program(g: &mut Source) -> Tree {
        let mut m = Model { classes: vec![], enums: vec![], protos: vec![], tds: vec![], funcs: vec![], aliases: vec![], gens: vec![], descs: vec![], iters: vec![], agens: vec![], aiters: vec![], seqs: vec![], in_async: false, async_used: false, rcount: Default::default(), no_alias: Default::default(), quoted: Default::default(), in_methods: false, probe: 0, fresh: 0, nesting: 0 };
        m.gen_decls(g);
        m.gen_funcs(g);
        let mut items: Vec<Tree> = Vec::new();
        if g.bool() {
            items.push(node("from __future__ import annotations"));
        }
        items.extend([
            node("import asyncio"),
            node("import collections"),
            node("import contextlib"),
            node("import typing"),
            node("import enum"),
            node("from collections.abc import AsyncGenerator, AsyncIterable, AsyncIterator, Callable, Generator, Iterable, Iterator"),
            node("from dataclasses import InitVar, dataclass, field"),
            node("from typing import Annotated, ClassVar, Self, Final, Generic, Literal, ParamSpec, TypeVarTuple, NamedTuple, TypeVar, Unpack, overload, NotRequired, Protocol, ReadOnly, Required, TypeAlias, TypeAliasType, TypedDict, TypeGuard, TypeIs, final, override, runtime_checkable, Never, NoReturn"),
            node("from _rt import _probe"),
        ]);
        if m.classes.iter().any(|c| c.kind == ClassKind::Pydantic) {
            items.push(node("from pydantic import BaseModel, ConfigDict, Field, Strict, computed_field"));
        }
        if m.classes.iter().any(|c| c.pyd.settings) {
            items.push(node("from pydantic_settings import BaseSettings, SettingsConfigDict"));
        }
        m.no_alias.set(true);
        for a in m.aliases.clone() {
            let r = m.render(&a.target);
            items.push(node(match (a.style, &a.tparam) {
                (_, Some(v)) => format!("type {}[{v}] = {r}", a.name),
                (0, None) => format!("type {} = {r}", a.name),
                (1, None) => format!("{}: TypeAlias = {r}", a.name),
                _ => format!("{} = TypeAliasType({:?}, {r})", a.name, a.name),
            }));
        }
        m.no_alias.set(false);
        items.push(node("def deco_pep695[**P, R](f: Callable[P, R]) -> Callable[P, R]:").with(vec![node("return f")]));
        items.push(node("P_ = ParamSpec(\"P_\")").then(node("R_ = TypeVar(\"R_\")").then(node("def deco_legacy(f: Callable[P_, R_]) -> Callable[P_, R_]:").with(vec![
            node("def w(*args: P_.args, **kwargs: P_.kwargs) -> R_:").with(vec![node("return f(*args, **kwargs)")]),
            node("return w"),
        ]))));
        items.push(node("def tupv[*Ts](*args: *Ts) -> tuple[*Ts]:").with(vec![node("return args")]));
        items.push(node("Ts_ = TypeVarTuple(\"Ts_\")").then(node("def tupv_legacy(*args: *Ts_) -> tuple[*Ts_]:").with(vec![node("return args")])));
        items.push(node("class Box_[T]:").with(vec![
            node("def __init__(self, v: T) -> None:").with(vec![node("self.v = v")]),
            node("def get(self) -> T:").with(vec![node("return self.v")]),
            node("def map[U](self, f: Callable[[T], U]) -> \"Box_[U]\":").with(vec![node("return Box_(f(self.v))")]),
            node("def pair[U](self, o: \"Box_[U]\") -> tuple[T, U]:").with(vec![node("return (self.v, o.v)")]),
        ]));
        items.push(node("class Getter_[T](Protocol):").with(vec![node("def get(self) -> T: ...")]));
        items.push(node("def take_[T](g: Getter_[T]) -> T:").with(vec![node("return g.get()")]));
        items.push(node("def apply_[T, U](f: Callable[[T], U], x: T) -> U:").with(vec![node("return f(x)")]));
        items.push(node("def compose_[A, B, C](f: Callable[[A], B], g: Callable[[B], C]) -> Callable[[A], C]:").with(vec![node("return lambda x: g(f(x))")]));
        items.push(node("def pick_[T: (int, str)](a: T, b: T) -> T:").with(vec![node("return b if a == b else a")]));
        items.push(node("def mx_[T: float](a: T, b: T) -> T:").with(vec![node("return a if a > b else b")]));
        items.push(node("def fail_(msg: str) -> NoReturn:").with(vec![node("raise ValueError(msg)")]));
        items.push(node("def fail_never_(msg: str) -> Never:").with(vec![node("raise ValueError(msg)")]));
        items.push(node("T = TypeVar(\"T\")"));
        items.push(node("U = TypeVar(\"U\")"));
        for (d, t) in m.descs.clone().into_iter().enumerate() {
            let r = m.render(&t);
            let dflt = m.expr(g, &t, &Scope::default(), 0);
            items.push(node(format!("class D{d}:")).with(vec![
                node("def __init__(self) -> None:").with(vec![node(format!("self.vals: dict[int, {r}] = {{}}"))]),
                node(format!("def __get__(self, obj: object, owner: object = None) -> {r}:")).with(vec![node(format!("return self.vals.get(id(obj), {dflt})"))]),
                node(format!("def __set__(self, obj: object, v: {r}) -> None:")).with(vec![node("self.vals[id(obj)] = v")]),
            ]));
        }
        for e in m.enums.clone() {
            let members = e
                .members
                .iter()
                .enumerate()
                .map(|(i, n)| if e.str_values { node(format!("{n} = {:?}", n.to_lowercase())) } else { node(format!("{n} = {}", i + 1)) })
                .collect();
            if e.functional {
                let spec = if e.str_values && e.flavour == "Enum" {
                    format!("[{}]", join(e.members.iter().map(|n| format!("({n:?}, {:?})", n.to_lowercase())), ", "))
                } else if g.bool() {
                    format!("{:?}", e.members.join(" "))
                } else {
                    format!("[{}]", join(e.members.iter().map(|n| format!("{n:?}")), ", "))
                };
                items.push(node(format!("{} = enum.{}({:?}, {spec})", e.name, e.flavour, e.name)));
                continue;
            }
            items.push(node(format!("class {}(enum.{}):", e.name, e.flavour)).with(members));
        }
        for p in m.protos.clone() {
            let mut ms: Vec<Tree> = p.methods.iter().map(|(n, t)| node(format!("def {n}(self) -> {}: ...", m.render(t)))).collect();
            for (n, t, ro) in &p.attrs {
                if *ro {
                    ms.push(node("@property").then(node(format!("def {n}(self) -> {}: ...", m.render(t)))));
                } else {
                    ms.push(node(format!("{n}: {}", m.render(t))));
                }
            }
            if ms.is_empty() {
                ms.push(node("pass"));
            }
            let t = node(format!("class {}(Protocol):", p.name)).with(ms);
            items.push(if p.runtime { node("@runtime_checkable").then(t) } else { t });
        }
        // Methods call no module function, so there is no recursion.
        let funcs = std::mem::take(&mut m.funcs);
        m.in_methods = true;
        for c in 0..m.classes.len() {
            let t = m.class_tree(g, c);
            items.push(t);
        }
        m.in_methods = false;
        // TypedDict fields may name classes (evaluated eagerly).
        for td in m.tds.clone() {
            let anns: Vec<(String, String)> = td.fields.iter().map(|(n, t, req)| {
                let r = m.render(t);
                let r = if *req == td.total { r } else if *req { format!("Required[{r}]") } else { format!("NotRequired[{r}]") };
                let r = if td.readonly.contains(n) { format!("ReadOnly[{r}]") } else { r };
                (n.clone(), r)
            }).collect();
            let tot = if td.total { String::new() } else { ", total=False".into() };
            if td.functional {
                let d = join(anns.iter().map(|(n, r)| format!("{n:?}: {r}")), ", ");
                items.push(node(format!("{} = TypedDict({:?}, {{{d}}}{tot})", td.name, td.name)));
            } else {
                let base = td.base.map(|b| m.tds[b].name.clone()).unwrap_or("TypedDict".into());
                let fs = anns.iter().map(|(n, r)| node(format!("{n}: {r}"))).collect();
                items.push(node(format!("class {}({base}{tot}):", td.name)).with(fs));
            }
        }
        for it in m.iters.clone() {
            items.push(m.iter_tree(g, &it));
        }
        for k in 0..m.gens.len() {
            let t = m.gen_tree(g, k);
            items.push(t);
        }
        for it in m.aiters.clone() {
            items.push(m.aiter_tree(&it, g));
        }
        for it in m.seqs.clone() {
            items.push(m.seq_tree(&it));
        }
        for k in 0..m.agens.len() {
            let t = m.agen_tree(g, k);
            items.push(t);
        }
        m.funcs = funcs;
        for fi in 0..m.funcs.len() {
            let t = m.func_tree(g, fi);
            items.push(t);
        }
        let mt = m.main_tree(g);
        items.push(mt);
        items.push(node("main()"));
        Tree::root(items)
    }
}

/// No user-defined type inside (an eager alias before the classes can name it).
fn builtin_only(t: &Ty) -> bool {
    match t {
        Ty::Class(..) | Ty::Enum(_) | Ty::Proto(_) | Ty::TypedDict(_) | Ty::Var(_) | Ty::Type(_) => false,
        Ty::Tuple(ts) | Ty::Union(ts) => ts.iter().all(builtin_only),
        Ty::TupleVar(t) | Ty::List(t) | Ty::Set(t) => builtin_only(t),
        Ty::Dict(k, v) => builtin_only(k) && builtin_only(v),
        Ty::Callable(ps, r) => ps.iter().all(builtin_only) && builtin_only(r),
        _ => true,
    }
}

fn vars_inhabited(t: &Ty, sc: &Scope) -> bool {
    match t {
        Ty::Var(_) => sc.vars.iter().any(|v| v.ty == *t),
        Ty::Tuple(ts) | Ty::Union(ts) => ts.iter().all(|t| vars_inhabited(t, sc)),
        Ty::TupleVar(t) | Ty::List(t) | Ty::Set(t) => vars_inhabited(t, sc),
        Ty::Dict(k, v) => vars_inhabited(k, sc) && vars_inhabited(v, sc),
        Ty::Callable(ps, r) => ps.iter().all(|t| vars_inhabited(t, sc)) && vars_inhabited(r, sc),
        Ty::Class(_, a) => a.iter().all(|t| vars_inhabited(t, sc)),
        _ => true,
    }
}

/// Whether `t` names a user class with index `k` or later (not defined yet).
fn mentions_class_from(t: &Ty, k: usize) -> bool {
    match t {
        Ty::Class(c, a) => *c >= k || a.iter().any(|t| mentions_class_from(t, k)),
        Ty::Type(c) => *c >= k,
        Ty::Tuple(ts) | Ty::Union(ts) => ts.iter().any(|t| mentions_class_from(t, k)),
        Ty::TupleVar(t) | Ty::List(t) | Ty::Set(t) => mentions_class_from(t, k),
        Ty::Dict(a, b) => mentions_class_from(a, k) || mentions_class_from(b, k),
        Ty::Callable(ps, r) => ps.iter().any(|t| mentions_class_from(t, k)) || mentions_class_from(r, k),
        _ => false,
    }
}

fn ts_len(t: &Ty) -> usize {
    if let Ty::Tuple(ts) = t { ts.len() } else { 0 }
}

fn contains_var(t: &Ty) -> bool {
    match t {
        Ty::Var(_) => true,
        Ty::Tuple(ts) | Ty::Union(ts) => ts.iter().any(contains_var),
        Ty::TupleVar(t) | Ty::List(t) | Ty::Set(t) => contains_var(t),
        Ty::Dict(k, v) => contains_var(k) || contains_var(v),
        Ty::Callable(ps, r) => ps.iter().any(contains_var) || contains_var(r),
        Ty::Class(_, a) => a.iter().any(contains_var),
        _ => false,
    }
}

fn subst_var(t: &Ty, n: &str, by: &Ty) -> Ty {
    let go = |t: &Ty| subst_var(t, n, by);
    match t {
        Ty::Var(m) if m == n => by.clone(),
        Ty::Tuple(ts) => Ty::Tuple(ts.iter().map(go).collect()),
        Ty::TupleVar(t) => Ty::TupleVar(Box::new(go(t))),
        Ty::List(t) => Ty::List(Box::new(go(t))),
        Ty::Set(t) => Ty::Set(Box::new(go(t))),
        Ty::Dict(k, v) => Ty::Dict(Box::new(go(k)), Box::new(go(v))),
        Ty::Union(ts) => Model::union_of(ts.iter().map(go).collect()),
        Ty::Callable(ps, r) => Ty::Callable(ps.iter().map(go).collect(), Box::new(go(r))),
        Ty::Class(i, a) => Ty::Class(*i, a.iter().map(go).collect()),
        _ => t.clone(),
    }
}
