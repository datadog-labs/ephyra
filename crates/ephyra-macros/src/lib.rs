// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! Procedural macros for [ephyra](https://crates.io/crates/ephyra).
//! Use them through the `ephyra` crate (`ephyra::Arbitrary`,
//! `#[ephyra::shrinker]`, and `#[ephyra::test]`); this crate is an
//! implementation detail.

mod generator_attr;
mod shrink_attr;

use {
    proc_macro::TokenStream,
    proc_macro2::TokenStream as TokenStream2,
    quote::{
        format_ident,
        quote,
    },
    syn::{
        Attribute,
        Data,
        DeriveInput,
        Expr,
        Fields,
        FnArg,
        GenericArgument,
        Ident,
        ItemFn,
        Meta,
        Pat,
        PathArguments,
        ReturnType,
        Token,
        Type,
        Visibility,
        parse_macro_input,
        punctuated::Punctuated,
        spanned::Spanned,
    },
};

/// Mark a predicate that needs a generator, or a generator that
/// `ephyra check` checks, optionally against its predicate.
///
/// ```ignore
/// #[ephyra::generator]
/// fn sorted(v: &[u8]) -> bool {
///     v.is_sorted()
/// }
///
/// #[ephyra::generator(sorted, metrics = sorted_metrics)]
/// fn sorted_vecs() -> impl Gen<Value = Vec<u8>> {
///     gens::vec(gens::int(0u8..=9)).map(|mut v| { v.sort(); v })
/// }
///
/// fn sorted_metrics(v: &Vec<u8>, _size: usize) -> Vec<(&'static str, f64)> {
///     vec![("len", v.len() as f64)]
/// }
/// ```
///
/// On a function returning `bool`, the attribute takes no arguments and
/// marks a predicate: one parameter, a plain binding of an elided shared
/// reference `&T`. The function is kept as it is. `ephyra check` lists
/// predicates that no generator names as work to do.
///
/// On a function with no parameters returning a generator (`impl Gen<Value =
/// T>`, `BoxGen<T>`, ...), it may name the predicate the values satisfy and a
/// `metrics` function `fn(&T, usize) -> Vec<(&'static str, f64)>` of a value
/// and the size it was drawn at. The function is kept, and an ignored
/// `__ephyra_check_<name>` test is added: `ephyra check` runs it over
/// thousands of inputs, checking that every value, shrink candidate,
/// first-candidate shrink chain step and mutant satisfies the predicate, that
/// shrink chains settle, and reporting the metrics. The predicate is called
/// with `&T`, so one over `&[E]` or `&str` accepts `Vec<E>` or `String`.
///
/// Generic, `async`, `unsafe`, `const` and `extern` functions and methods
/// are rejected.
#[proc_macro_attribute]
pub fn generator(args: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(args as generator_attr::Args);
    let func = parse_macro_input!(item as ItemFn);
    generator_attr::expand(args, func).unwrap_or_else(|e| e.to_compile_error()).into()
}

/// Derive a default generator, shrinker and mutator for a struct or enum.
///
/// The type must be `Clone`. Every field is generated with its type's
/// `Arbitrary` default, or with `#[ephyra(gen = <expression>)]`; the same
/// generator's `shrink` and `mutate` are used for that field, so candidates
/// stay in the field's range. Shrinking replaces one field at a time; enum
/// values also shrink to earlier variants that carry no data, so list the
/// simplest variant first. Mutation changes one field, or one time in eight
/// generates a fresh value (which can switch the variant).
///
/// `#[ephyra(support = <path>)]` on the type names a predicate
/// `fn(&Self) -> bool`; candidates and mutants that fail it are dropped.
/// Use it for invariants that span fields.
///
/// ```ignore
/// #[derive(Debug, Clone, ephyra::Arbitrary)]
/// #[ephyra(support = Roll::is_valid)]
/// struct Roll {
///     #[ephyra(gen = ephyra::gens::int(1u8..=6))]
///     die: u8,
///     player: String,
/// }
/// ```
///
/// Recursive types need `#[ephyra(max_depth = N)]` on the type, which bounds
/// how deep fields holding the type itself nest:
///
/// ```ignore
/// #[derive(Debug, Clone, ephyra::Arbitrary)]
/// #[ephyra(max_depth = 4)]
/// enum Expr { Lit(i8), Add(Box<Expr>, Box<Expr>) }
/// ```
///
/// Such a field is `Box<Self>`, `Vec<Self>` or `Option<Box<Self>>` (or has
/// its own `#[ephyra(gen = ...)]`). At depth `N` a variant with a `Box<Self>`
/// field is not chosen, a `Vec<Self>` is empty and an `Option<Box<Self>>` is
/// `None`, so some variant must have no `Box<Self>` field. A value shrinks
/// first to each of its children (`Add(a, b)` to `a` and to `b`), then field
/// by field, every field one level down; mutation keeps the bound. The value
/// says which variant it is, so its hooks always run on their own values.
/// Without `max_depth`, a recursive type has no bound: generation may not end.
#[proc_macro_derive(Arbitrary, attributes(ephyra))]
pub fn derive_arbitrary(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    expand_arbitrary(input).unwrap_or_else(|e| e.to_compile_error()).into()
}

/// Convert a classical eager shrinker into Ephyra's lazy visitor form.
///
/// Write the shrinker naturally as `fn(&T) -> Vec<T>`. The generated function
/// instead accepts a candidate visitor and returns `ControlFlow<()>`, so it can
/// stop as soon as the runner accepts a candidate without allocating the rest.
/// Candidate order is preserved, but once a candidate is accepted, later
/// candidate expressions and iterator elements are not evaluated, so they
/// should have no side effects that matter.
///
/// ```ignore
/// #[ephyra::shrinker]
/// fn shrink_number(value: &u32) -> Vec<u32> {
///     vec![0, value / 2]
/// }
/// ```
///
/// # Accepted language
///
/// The macro translates a small language for building one `Vec<T>` of shrink
/// candidates, not arbitrary Rust. Every return path, `return` included, ends
/// in a producer:
///
/// - `Vec::new()` or `Vec::with_capacity(…)`;
/// - `vec![a, b, …]`, but not `vec![a; n]`, whose streaming would change when
///   `a` is cloned;
/// - an iterator expression ending in `.collect()`;
/// - a local Vec initialized by one of those producers and returned as its
///   block's final expression. Candidates may then only be added to it, with
///   `.push(candidate)` and `.extend(iterator)`; it needs `mut` only for that.
///
/// `if`, `match`, `for`, `while` and `loop` may surround these operations.
/// Direct recursion streams through the visitor instead of collecting when it
/// is written `candidates.extend(shrink(child))` or
/// `candidates.extend(shrink(child).into_iter().map(|child| rebuild(child)))`;
/// qualified calls such as `Self::shrink(child)` are recognized by their last
/// path segment. A recursive call inside a longer iterator pipeline is
/// rejected: split the pipeline into successive `extend`s.
///
/// # Rejected language
///
/// The local candidate Vec may not be read (`len`, indexing, iteration,
/// comparison), reordered or filtered (`sort`, `retain`, `dedup`), aliased,
/// borrowed, cloned, moved, passed to a helper, used in a macro, captured by
/// a closure or async block, or returned early. Each of these, and a return
/// type other than `Vec<T>`, gets a diagnostic at the offending code naming
/// the broken rule instead of a Rust type error in the transformed function.
/// Mistakes that are Rust errors anyway, such as `return;` or a final `if`
/// without `else`, stay rustc errors.
///
/// # QuickCheck compatibility
///
/// Each shape used by QuickCheck's built-in [`Arbitrary::shrink`
/// implementations](https://docs.rs/quickcheck/latest/src/quickcheck/arbitrary.rs.html)
/// has a form in this language. QuickCheck returns
/// `Box<dyn Iterator<Item = T>>`, which the macro rejects: collect the
/// candidates into a `Vec<T>` first.
///
/// | QuickCheck shape | Examples | Macro form |
/// |---|---|---|
/// | Empty or singleton | default, `bool` | `Vec::new()`, `vec![x]` |
/// | Branch-selected producer | `Option`, `Result`, `Bound` | final `match` or `if` |
/// | Mapped child shrinker | wrappers, ranges, strings, collections | `.map(…).collect()` or candidate Vec `.extend(…)` |
/// | Chained child shrinkers | tuples, `Option` | successive `.extend(…)` or `.chain(…).collect()` |
/// | `filter`, `filter_map` | floats, `char`, `CString` | iterator `.collect()` |
/// | `flat_map` | arrays, `SystemTime` | iterator `.collect()` |
/// | Conditional accumulation | `PathBuf` | `mut` candidate Vec with `push` |
/// | Custom iterator state machine | numbers, `VecShrinker` | the iterator's `.collect()` or `.extend(…)`, streamed as is |
///
/// Proptest's stateful `ValueTree` shrinking is not a sequence of candidates,
/// so it has no counterpart here.
#[proc_macro_attribute]
pub fn shrinker(arguments: TokenStream, input: TokenStream) -> TokenStream {
    parse_macro_input!(arguments as syn::parse::Nothing);
    let function = parse_macro_input!(input as ItemFn);
    shrink_attr::expand(function).unwrap_or_else(|error| error.to_compile_error()).into()
}

/// The value of the first `#[ephyra(key = value)]` attribute, if any. A
/// `#[ephyra(...)]` attribute with another key is an error saying `expected`.
fn ephyra_attr(attrs: &[Attribute], key: &str, expected: &str) -> syn::Result<Option<Expr>> {
    let Some(attr) = attrs.iter().find(|attr| attr.path().is_ident("ephyra")) else {
        return Ok(None);
    };
    let meta: syn::MetaNameValue = attr.parse_args()?;
    if meta.path.is_ident(key) {
        Ok(Some(meta.value))
    } else {
        Err(syn::Error::new(meta.path.span(), expected))
    }
}

/// The generator expression for one field: a `#[ephyra(gen = ...)]`
/// override or the type's `Arbitrary`.
fn field_generator(field: &syn::Field) -> syn::Result<TokenStream2> {
    let ty = &field.ty;
    Ok(match ephyra_attr(&field.attrs, "gen", "expected `#[ephyra(gen = <expression>)]`")? {
        Some(expr) => quote!((#expr)),
        None => quote!(<#ty as ::ephyra::Arbitrary>::arbitrary()),
    })
}

/// The type-level `#[ephyra(...)]` settings of a derived type.
#[derive(Default)]
struct TypeAttrs {
    /// `support = <path>`: a predicate candidates and mutants must pass.
    support: Option<Expr>,
    /// `max_depth = <n>`: how deep fields holding the type itself may nest.
    max_depth: Option<syn::LitInt>,
}

/// The type's `#[ephyra(support = <path>, max_depth = <n>)]` settings, from
/// any number of `#[ephyra(...)]` attributes, each key at most once.
fn type_attrs(input: &DeriveInput) -> syn::Result<TypeAttrs> {
    const EXPECTED: &str =
        "expected `#[ephyra(support = <path to fn(&Self) -> bool>, max_depth = <n>)]`";
    let mut attrs = TypeAttrs::default();
    for attr in input.attrs.iter().filter(|attr| attr.path().is_ident("ephyra")) {
        let pairs =
            attr.parse_args_with(Punctuated::<syn::MetaNameValue, Token![,]>::parse_terminated)?;
        for pair in pairs {
            if pair.path.is_ident("support") && attrs.support.is_none() {
                attrs.support = Some(pair.value);
            } else if pair.path.is_ident("max_depth") && attrs.max_depth.is_none() {
                match pair.value {
                    Expr::Lit(syn::ExprLit { lit: syn::Lit::Int(n), .. }) => {
                        attrs.max_depth = Some(n)
                    },
                    other => {
                        return Err(syn::Error::new_spanned(
                            other,
                            "`max_depth` takes an integer literal",
                        ));
                    },
                }
            } else {
                return Err(syn::Error::new(pair.path.span(), EXPECTED));
            }
        }
    }
    Ok(attrs)
}

/// How a field holds the type it belongs to, for `max_depth`.
#[derive(Clone, Copy)]
enum Recursion {
    /// `Box<Self>`: a variant with one cannot be built at depth 0.
    Boxed,
    /// `Vec<Self>`: empty at depth 0.
    Vector,
    /// `Option<Box<Self>>`: `None` at depth 0.
    OptionBoxed,
}

/// Whether `ty` is `Self` or the type `name` (any generic arguments).
fn is_self(ty: &Type, name: &Ident) -> bool {
    let Type::Path(path) = ty else { return false };
    path.qself.is_none()
        && path.path.segments.last().is_some_and(|last| last.ident == "Self" || last.ident == *name)
}

/// `Wrapper<inner>`'s inner type, for a one-argument wrapper named `wrapper`.
fn wrapped<'a>(ty: &'a Type, wrapper: &str) -> Option<&'a Type> {
    let Type::Path(path) = ty else { return None };
    let last = path.path.segments.last()?;
    if last.ident != wrapper {
        return None;
    }
    let PathArguments::AngleBracketed(arguments) = &last.arguments else { return None };
    match arguments.args.first()? {
        GenericArgument::Type(inner) if arguments.args.len() == 1 => Some(inner),
        _ => None,
    }
}

/// Whether the tokens of `ty` mention `Self` or `name` anywhere.
fn mentions_self(ty: &Type, name: &Ident) -> bool {
    fn walk(tokens: TokenStream2, name: &Ident) -> bool {
        tokens.into_iter().any(|tree| {
            match tree {
                proc_macro2::TokenTree::Ident(ident) => ident == "Self" || ident == *name,
                proc_macro2::TokenTree::Group(group) => walk(group.stream(), name),
                _ => false,
            }
        })
    }
    walk(quote!(#ty), name)
}

/// The recursion of a field of type `ty` in the type `name`: `Ok(None)` for a
/// field that does not hold the type, an error for one that holds it in a
/// shape `max_depth` cannot bound.
fn recursion(ty: &Type, name: &Ident) -> syn::Result<Option<Recursion>> {
    if wrapped(ty, "Box").is_some_and(|inner| is_self(inner, name)) {
        return Ok(Some(Recursion::Boxed));
    }
    if wrapped(ty, "Vec").is_some_and(|inner| is_self(inner, name)) {
        return Ok(Some(Recursion::Vector));
    }
    if wrapped(ty, "Option")
        .and_then(|inner| wrapped(inner, "Box"))
        .is_some_and(|inner| is_self(inner, name))
    {
        return Ok(Some(Recursion::OptionBoxed));
    }
    if mentions_self(ty, name) {
        return Err(syn::Error::new_spanned(
            ty,
            "with `max_depth`, a field holding the type itself must be `Box<Self>`, \
             `Vec<Self>` or `Option<Box<Self>>`; give any other one \
             `#[ephyra(gen = <expression>)]`",
        ));
    }
    Ok(None)
}

/// The depth-aware generator of a recursive field: the type's own generator
/// one level down, inside the field's wrapper.
fn recursive_generator(recursion: Recursion) -> TokenStream2 {
    let boxed = quote! {
        Self::__ephyra_level(depth.saturating_sub(1)).iso(
            ::std::boxed::Box::new,
            |boxed: &::std::boxed::Box<Self>| ::core::clone::Clone::clone(&**boxed),
        )
    };
    match recursion {
        Recursion::Boxed => quote!((#boxed)),
        Recursion::Vector => {
            quote! {
                (if depth == 0 {
                    ::ephyra::gens::constant(::std::vec::Vec::new()).boxed()
                } else {
                    ::ephyra::gens::vec(Self::__ephyra_level(depth - 1)).boxed()
                })
            }
        },
        Recursion::OptionBoxed => {
            quote! {
                (if depth == 0 {
                    ::ephyra::gens::constant(::core::option::Option::None).boxed()
                } else {
                    ::ephyra::gens::option(#boxed).boxed()
                })
            }
        },
    }
}

/// One shape of value: a struct or an enum variant, with how to build it,
/// take it apart, and reach its fields.
struct Shape {
    /// Path used to construct the value (`Type` or `Type::Variant`).
    path: TokenStream2,
    fields: Vec<ShapeField>,
    style: Style,
}

enum Style {
    Named,
    Unnamed,
    Unit,
}

struct ShapeField {
    /// Field name, or a synthetic name for tuple fields.
    binding: Ident,
    generator: TokenStream2,
    /// How the field holds the type itself, under `max_depth`.
    recursion: Option<Recursion>,
}

/// `recursive_in`: the type's name when it has `max_depth`, so that fields
/// holding the type itself get depth-aware generators.
fn shape(path: TokenStream2, fields: &Fields, recursive_in: Option<&Ident>) -> syn::Result<Shape> {
    let style = match fields {
        Fields::Named(_) => Style::Named,
        Fields::Unnamed(_) => Style::Unnamed,
        Fields::Unit => Style::Unit,
    };
    let fields = fields
        .iter()
        .enumerate()
        .map(|(i, f)| {
            let binding = f.ident.clone().unwrap_or_else(|| format_ident!("field_{i}"));
            let overridden =
                ephyra_attr(&f.attrs, "gen", "expected `#[ephyra(gen = <expression>)]`")?.is_some();
            let recursion = match recursive_in {
                Some(name) if !overridden => recursion(&f.ty, name)?,
                _ => None,
            };
            let generator = match recursion {
                Some(recursion) => recursive_generator(recursion),
                None => field_generator(f)?,
            };
            Ok(ShapeField { binding, generator, recursion })
        })
        .collect::<syn::Result<_>>()?;
    Ok(Shape { path, fields, style })
}

impl Shape {
    /// `Path { a: <a>, b: <b> }` / `Path(<a>, <b>)` / `Path` from expressions.
    fn construct(&self, values: impl Iterator<Item = TokenStream2>) -> TokenStream2 {
        let path = &self.path;
        match self.style {
            Style::Named => {
                let inits = self.fields.iter().zip(values).map(|(f, v)| {
                    let name = &f.binding;
                    quote!(#name: #v)
                });
                quote!(#path { #(#inits),* })
            },
            Style::Unnamed => quote!(#path(#(#values),*)),
            Style::Unit => quote!(#path),
        }
    }

    /// A pattern that matches this shape without binding anything.
    fn wildcard(&self) -> TokenStream2 {
        self.construct(self.fields.iter().map(|_| quote!(_)))
    }

    /// The value rebuilt from the fields bound by `pattern()`, with field `i`
    /// replaced by `replacement`.
    fn rebuild(&self, i: usize, replacement: TokenStream2) -> TokenStream2 {
        self.construct(self.fields.iter().enumerate().map(|(j, other)| {
            let name = &other.binding;
            if i == j { replacement.clone() } else { quote!(::core::clone::Clone::clone(#name)) }
        }))
    }

    /// A pattern binding every field to its `binding` name, in shorthand:
    /// `construct`'s `name: name` would trip rustc's
    /// `non_shorthand_field_patterns` lint, as the names carry user spans.
    fn pattern(&self) -> TokenStream2 {
        let path = &self.path;
        let names = self.fields.iter().map(|f| &f.binding);
        match self.style {
            Style::Named => quote!(#path { #(#names),* }),
            Style::Unnamed => quote!(#path(#(#names),*)),
            Style::Unit => quote!(#path),
        }
    }

    fn generate(&self) -> TokenStream2 {
        self.construct(self.fields.iter().map(|f| {
            let generator = &f.generator;
            quote!(g.draw(&#generator))
        }))
    }

    /// Candidates that replace one field by one of its shrinks, given the
    /// fields are bound by `pattern()`.
    fn shrink_arm(&self) -> TokenStream2 {
        let pattern = self.pattern();
        let per_field = self.fields.iter().enumerate().map(|(i, f)| {
            let generator = &f.generator;
            let binding = &f.binding;
            let rebuilt = self.rebuild(i, quote!(candidate));
            quote! {
                #generator.shrink(#binding, &mut |candidate| emit(#rebuilt))?;
            }
        });
        // A node's children are candidates for the node: the step that makes
        // a recursive value shrink fast.
        let children = self.fields.iter().filter_map(|f| {
            let binding = &f.binding;
            Some(match f.recursion? {
                Recursion::Boxed => quote!(emit(::core::clone::Clone::clone(&**#binding))?;),
                Recursion::Vector => {
                    quote! {
                        for child in #binding.iter() {
                            emit(::core::clone::Clone::clone(child))?;
                        }
                    }
                },
                Recursion::OptionBoxed => {
                    quote! {
                        if let ::core::option::Option::Some(child) = #binding {
                            emit(::core::clone::Clone::clone(&**child))?;
                        }
                    }
                },
            })
        });
        quote! {
            #pattern => {
                #(#children)*
                #(#per_field)*
                ::core::ops::ControlFlow::Continue(())
            }
        }
    }

    /// Whether the shape needs a level below it (a `Box<Self>` field), so it
    /// cannot be built at depth 0.
    fn needs_depth(&self) -> bool {
        self.fields.iter().any(|f| matches!(f.recursion, Some(Recursion::Boxed)))
    }

    /// One mutated field, chosen at random, given the fields are bound.
    fn mutate_arm(&self) -> TokenStream2 {
        let pattern = self.pattern();
        let count = self.fields.len();
        if count == 0 {
            return quote!(#pattern => ::core::clone::Clone::clone(value),);
        }
        let mutate_field = |i: usize, f: &ShapeField| {
            let generator = &f.generator;
            let binding = &f.binding;
            let rebuilt = self.rebuild(i, quote!(mutated));
            quote! {{
                let mutated = #generator.mutate(#binding, g);
                #rebuilt
            }}
        };
        // With one field there is nothing to choose, so no draw is spent on it.
        if count == 1 {
            let only = mutate_field(0, &self.fields[0]);
            return quote!(#pattern => #only,);
        }
        let per_field = self.fields.iter().enumerate().map(|(i, f)| {
            let body = mutate_field(i, f);
            quote!(#i => #body,)
        });
        quote! {
            #pattern => {
                let which = g.index(#count);
                match which {
                    #(#per_field)*
                    _ => unreachable!(),
                }
            }
        }
    }
}

fn expand_arbitrary(input: DeriveInput) -> syn::Result<TokenStream2> {
    let name = &input.ident;
    let TypeAttrs { support, max_depth } = type_attrs(&input)?;
    let recursive_in = max_depth.as_ref().map(|_| name);
    let mut generics = input.generics.clone();
    for param in generics.type_params_mut() {
        param.bounds.push(syn::parse_quote!(::ephyra::Arbitrary));
        param.bounds.push(syn::parse_quote!(::core::clone::Clone));
    }
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

    let shapes: Vec<Shape> = match &input.data {
        Data::Struct(data) => vec![shape(quote!(#name), &data.fields, recursive_in)?],
        Data::Enum(data) => {
            if data.variants.is_empty() {
                return Err(syn::Error::new_spanned(
                    name,
                    "cannot derive Arbitrary for an enum with no variants",
                ));
            }
            data.variants
                .iter()
                .map(|variant| {
                    let variant_name = &variant.ident;
                    shape(quote!(#name::#variant_name), &variant.fields, recursive_in)
                })
                .collect::<syn::Result<Vec<_>>>()?
        },
        Data::Union(_) => {
            return Err(syn::Error::new_spanned(name, "cannot derive Arbitrary for a union"));
        },
    };

    // A uniform choice among `shapes`.
    let choose = |shapes: &[&Shape]| {
        if shapes.len() == 1 {
            return shapes[0].generate();
        }
        let count = shapes.len();
        let arms = shapes.iter().enumerate().map(|(index, s)| {
            let value = s.generate();
            quote!(#index => #value)
        });
        quote! {
            match g.int(0usize..#count) {
                #(#arms,)*
                _ => ::core::unreachable!(),
            }
        }
    };
    let all: Vec<&Shape> = shapes.iter().collect();
    let generate = if max_depth.is_some() && shapes.iter().any(Shape::needs_depth) {
        // At depth 0, only the shapes that need no level below.
        let shallow: Vec<&Shape> = shapes.iter().filter(|s| !s.needs_depth()).collect();
        if shallow.is_empty() {
            return Err(syn::Error::new_spanned(
                name,
                "with `max_depth`, some variant must have no `Box<Self>` field, to end the \
                 recursion",
            ));
        }
        let (deep, shallow) = (choose(&all), choose(&shallow));
        quote!(if depth == 0 { #shallow } else { #deep })
    } else {
        choose(&all)
    };

    // Earlier data-less variants are candidates for every later variant.
    let shrink_arms = shapes.iter().map(Shape::shrink_arm);
    let mutate_arms = shapes.iter().map(Shape::mutate_arm);
    let variant_index_arms = shapes.iter().enumerate().map(|(i, s)| {
        let wildcard = s.wildcard();
        quote!(#wildcard => #i)
    });
    let unit_candidates = if !shapes.iter().any(|s| matches!(s.style, Style::Unit)) {
        quote!()
    } else {
        let unit_index_arms =
            shapes.iter().enumerate().filter(|(_, s)| matches!(s.style, Style::Unit)).map(
                |(i, s)| {
                    let path = &s.path;
                    quote!(if #i < index { emit(#path)?; })
                },
            );
        quote! {
            let index = match value { #(#variant_index_arms,)* };
            #(#unit_index_arms)*
        }
    };
    let (check_candidate, check_mutant) = support
        .map(|pred| {
            (
                quote!(if !(#pred)(&candidate) { return ::core::ops::ControlFlow::Continue(()); }),
                quote!(if !(#pred)(&mutated) { return ::core::clone::Clone::clone(value); }),
            )
        })
        .unzip();
    // An enum sometimes switches variant by drawing a fresh value; a struct
    // has only its fields to mutate.
    // A fresh value is drawn at this value's own depth, under `max_depth`.
    let fresh = match max_depth {
        Some(_) => quote!(Self::__ephyra_level(depth).generate(g)),
        None => quote!(<Self as ::ephyra::Arbitrary>::arbitrary().generate(g)),
    };
    let mutate_value = if shapes.len() > 1 {
        quote! {
            if g.bool_with(0.125) {
                #fresh
            } else {
                match value { #(#mutate_arms)* }
            }
        }
    } else {
        quote!(match value { #(#mutate_arms)* })
    };

    let body = quote! {
        use ::ephyra::{Gen as _, GenExt as _};
        let generator = move |g: &mut ::ephyra::Source| -> Self { #generate };
        ::ephyra::BoxGen::new(
            generator
                .with_shrink(move |value: &Self, visitor| {
                    let mut emit = |candidate: Self| {
                        #check_candidate
                        visitor(candidate)
                    };
                    #unit_candidates
                    match value { #(#shrink_arms)* }
                })
                .with_mutate(move |value: &Self, g| {
                    let mutated: Self = #mutate_value;
                    #check_mutant
                    mutated
                }),
        )
    };
    Ok(match max_depth {
        None => {
            quote! {
                #[automatically_derived]
                impl #impl_generics ::ephyra::Arbitrary for #name #ty_generics #where_clause {
                    fn arbitrary() -> ::ephyra::BoxGen<Self> {
                        #body
                    }
                }
            }
        },
        Some(max_depth) => {
            quote! {
                #[automatically_derived]
                impl #impl_generics ::ephyra::Arbitrary for #name #ty_generics #where_clause {
                    fn arbitrary() -> ::ephyra::BoxGen<Self> {
                        Self::__ephyra_level(#max_depth)
                    }
                }

                #[automatically_derived]
                impl #impl_generics #name #ty_generics #where_clause {
                    /// The generator of values at most `depth` levels deep.
                    #[doc(hidden)]
                    fn __ephyra_level(depth: usize) -> ::ephyra::BoxGen<Self> {
                        let _ = depth;
                        #body
                    }
                }
            }
        },
    })
}

/// Turn a function with typed parameters into a `#[test]` that checks it as
/// a property over `Arbitrary` inputs.
///
/// ```ignore
/// #[ephyra::test]
/// fn sorting_is_idempotent(mut v: Vec<i32>) -> bool {
///     v.sort();
///     let once = v.clone();
///     v.sort();
///     v == once
/// }
///
/// #[ephyra::test(tests = 1000, seed = 7)]
/// fn addition_commutes(a: u8, b: u8) -> bool {
///     u16::from(a) + u16::from(b) == u16::from(b) + u16::from(a)
/// }
/// ```
///
/// Accepted arguments: `tests = N`, `seed = N`, `max_size = N`,
/// `failures = N` (collect up to N distinct failures, see
/// `Config::failures`), `config = <expression>` for a complete `Config`,
/// `search = <expression>`,
/// `gen = <expression>` and `metrics = <path>`. A search expression is
/// called with `&mut Run` after replaying remembered failures and is
/// responsible for driving the run; for example,
/// `|run| loops::fuzz(run, &mut loops::Fifo::new(), 1_000)`. Ephyra still
/// owns final shrinking, regression storage and reporting. The function may
/// return anything `Testable`. Other attributes (such as `#[ignore]`) are
/// kept.
///
/// With coverage on, an empty `Config::coverage.filter` becomes the
/// package name (dashes as underscores), which covers the library when the
/// test sits in that package. A test in a separate crate covers only that
/// crate unless the filter names the crate under test:
/// `config.coverage.filter = "my_lib".into()` or
/// `EPHYRA_COVERAGE_FILTER=my_lib` (`crate` or `crate::module`).
///
/// Inputs come from the parameters' `Arbitrary` generators unless a
/// strategy is given: `#[ephyra(gen = <expression>)]` on a parameter
/// replaces that parameter's generator, and `gen` replaces them all with a
/// generator of the parameter's type, or of the tuple of all parameters, so
/// it can keep relations such as `lo < hi` that the property `assume!`s.
///
/// A property that can discard inputs (it returns `TestResult`, as `assume!`
/// needs, or a `Result` of one) and has a strategy also gets an ignored
/// `__ephyra_check_<name>` test that `ephyra check` runs over thousands of
/// inputs: shrink candidates, first-candidate shrink chain steps and mutants
/// of the strategy must not be discarded by the property, shrink chains must
/// settle, and `metrics`, a function `fn(&T, usize) -> Vec<(&'static str,
/// f64)>` of an input and its size, is reported. A property returning
/// `bool`, `()` or a `Result` of these never discards, so its strategy has
/// nothing to check beyond its generators' own checks, and `metrics` belong
/// on those generators. A `Result` without type arguments (an alias such as
/// `fmt::Result`) counts as one that never discards, so write
/// `Result<TestResult, E>` out.
#[proc_macro_attribute]
pub fn test(args: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(args with Punctuated::<Meta, Token![,]>::parse_terminated);
    let func = parse_macro_input!(item as ItemFn);
    expand_test(args, func).unwrap_or_else(|e| e.to_compile_error()).into()
}

fn expand_test(args: Punctuated<Meta, Token![,]>, func: ItemFn) -> syn::Result<TokenStream2> {
    let mut idents = Vec::new();
    let mut types = Vec::new();
    // `#[ephyra(gen = ...)]` on a parameter: its own strategy.
    let mut parameter_strategies = Vec::new();
    for arg in &func.sig.inputs {
        match arg {
            FnArg::Typed(typed) => {
                let ident = match &*typed.pat {
                    Pat::Ident(pat) => pat.ident.clone(),
                    other => {
                        return Err(syn::Error::new(
                            other.span(),
                            "ephyra::test parameters must be plain identifiers",
                        ));
                    },
                };
                idents.push(ident);
                types.push((*typed.ty).clone());
                parameter_strategies.push(ephyra_attr(
                    &typed.attrs,
                    "gen",
                    "expected `#[ephyra(gen = <expression>)]` on a parameter",
                )?);
            },
            FnArg::Receiver(receiver) => {
                return Err(syn::Error::new(
                    receiver.span(),
                    "ephyra::test cannot be applied to methods",
                ));
            },
        }
    }
    if idents.is_empty() {
        return Err(syn::Error::new(
            func.sig.span(),
            "ephyra::test needs at least one parameter to generate",
        ));
    }
    if idents.len() > 8 {
        return Err(syn::Error::new(
            func.sig.span(),
            "ephyra::test supports up to 8 parameters; group them in a struct",
        ));
    }

    let mut config = quote!(::ephyra::Config::default());
    let (mut search, mut strategy, mut metrics) = (None, None, None);
    for meta in args {
        let Meta::NameValue(pair) = meta else {
            return Err(syn::Error::new(
                meta.span(),
                "expected `key = value` (tests, seed, max_size, failures, config, search, gen, \
                 metrics)",
            ));
        };
        let value: &Expr = &pair.value;
        let key = pair.path.get_ident().map(ToString::to_string).unwrap_or_default();
        let slot = match key.as_str() {
            "search" => &mut search,
            "gen" => &mut strategy,
            "metrics" => &mut metrics,
            _ => {
                config = match key.as_str() {
                    "tests" => quote!(#config.tests(#value)),
                    "seed" => quote!(#config.seed(#value)),
                    "max_size" => quote!(#config.max_size(#value)),
                    "failures" => quote!(#config.failures(#value)),
                    "config" => quote!(#value),
                    _ => {
                        return Err(syn::Error::new(
                            pair.path.span(),
                            "unknown ephyra::test argument; expected tests, seed, max_size, \
                             failures, config, search, gen or metrics",
                        ));
                    },
                };
                continue;
            },
        };
        if slot.replace(pair.value.clone()).is_some() {
            return Err(syn::Error::new(pair.path.span(), format!("duplicate `{key}` argument")));
        }
    }
    let per_parameter = parameter_strategies.iter().any(Option::is_some);
    if per_parameter && strategy.is_some() {
        return Err(syn::Error::new(
            func.sig.span(),
            "give either `gen = <expression>` for the whole input or \
             `#[ephyra(gen = <expression>)]` on parameters, not both",
        ));
    }
    // A property that cannot discard has no domain beyond its generators,
    // whose own checks cover them.
    let checked = (per_parameter || strategy.is_some()) && can_discard(&func.sig.output);
    if metrics.is_some() && !checked {
        return Err(syn::Error::new(
            func.sig.span(),
            "`metrics` go with the check of a strategy the property can discard from \
             (it returns `TestResult` or a `Result` of one); put them on the generator \
             instead: #[ephyra::generator(metrics = <fn>)]",
        ));
    }

    let name = &func.sig.ident;
    let vis = &func.vis;
    let attrs = &func.attrs;
    let mut inner = func.clone();
    inner.attrs.clear();
    inner.vis = Visibility::Inherited;
    inner.sig.ident = format_ident!("__ephyra_property");
    for input in &mut inner.sig.inputs {
        if let FnArg::Typed(typed) = input {
            typed.attrs.retain(|attribute| !attribute.path().is_ident("ephyra"));
        }
    }

    // One parameter is generated as itself, several as a tuple.
    let parameter_generators: Vec<TokenStream2> = types
        .iter()
        .zip(&parameter_strategies)
        .map(|(ty, strategy)| {
            match strategy {
                Some(strategy) => quote!((#strategy)),
                None => quote!(::ephyra::any::<#ty>()),
            }
        })
        .collect();
    let (generator, property) = match &idents[..] {
        [ident] => (parameter_generators[0].clone(), quote!(|#ident| __ephyra_property(#ident))),
        _ => {
            (
                quote!((#(#parameter_generators,)*)),
                quote!(|(#(#idents,)*)| __ephyra_property(#(#idents),*)),
            )
        },
    };
    let generator = strategy.as_ref().map_or(generator, |strategy| quote!((#strategy)));

    let report = if let Some(search) = search {
        quote! {
            fn __ephyra_search<G, P, S, O>(run: &mut ::ephyra::Run<G, P>, search: S)
            where
                G: ::ephyra::Gen,
                S: ::core::ops::FnOnce(&mut ::ephyra::Run<G, P>) -> O,
            {
                let _ = search(run);
            }

            let mut __run = ::ephyra::Run::new(&__config, #generator, #property);
            if !__run.remembered() {
                __ephyra_search(&mut __run, #search);
            }
            __run.finish()
        }
    } else {
        quote!(::ephyra::check(&__config, #generator, #property))
    };

    let check = checked.then(|| {
        let check = format_ident!("__ephyra_check_{name}");
        let cfg = check_test_cfgs(attrs);
        let metrics = metrics_or_none(metrics.as_ref());
        quote! {
            #(#cfg)*
            #[test]
            #[ignore = "run by `ephyra check`"]
            fn #check() {
                #inner
                let __config: ::ephyra::Config = #config;
                let __report = ::ephyra::__check_strategy(
                    &__config.tests(10_000).name(stringify!(#name)),
                    #generator,
                    #property,
                    #metrics,
                );
                ::std::println!("{__report}");
                __report.assert_passed();
            }
        }
    });

    Ok(quote! {
        #(#attrs)*
        #[test]
        #vis fn #name() {
            #inner
            let __config = {
                let mut __config: ::ephyra::Config = #config;
                if __config.coverage.filter.is_empty() {
                    __config.coverage.filter = ::core::env!("CARGO_PKG_NAME").replace('-', "_");
                }
                __config
            };
            let __report = { #report };
            __report.assert_passed();
        }

        #check
    })
}

// ---- shared by `#[test]` and `#[generator]` ------------------------------

/// The attributes a check test keeps: only plain `cfg`s. Anything else, such
/// as `should_panic` (also through `cfg_attr`), would make its exit status
/// unreliable.
fn check_test_cfgs(attrs: &[Attribute]) -> impl Iterator<Item = &Attribute> {
    attrs.iter().filter(|attribute| attribute.path().is_ident("cfg"))
}

/// Whether a property returning `output` can discard an input. Only
/// `TestResult` can (`assume!` returns one), so `()` and `bool` properties
/// never do; a type it cannot tell is assumed to. A `Result` can if its
/// first type argument, the `Ok` value, can, and is assumed not to without
/// type arguments (an alias such as `fmt::Result`).
fn can_discard(output: &ReturnType) -> bool {
    let ReturnType::Type(_, ty) = output else { return false };
    type_can_discard(ty)
}

fn type_can_discard(ty: &Type) -> bool {
    match ty {
        Type::Tuple(tuple) => !tuple.elems.is_empty(),
        Type::Path(path) if path.qself.is_none() => {
            let Some(last) = path.path.segments.last() else { return true };
            if last.ident != "Result" {
                return last.ident != "bool";
            }
            let PathArguments::AngleBracketed(arguments) = &last.arguments else { return false };
            let ok = arguments.args.iter().find_map(|argument| {
                match argument {
                    GenericArgument::Type(ty) => Some(ty),
                    _ => None,
                }
            });
            ok.is_some_and(type_can_discard)
        },
        _ => true,
    }
}

/// A check's metrics function, or one that measures nothing.
fn metrics_or_none(metrics: Option<&Expr>) -> TokenStream2 {
    match metrics {
        Some(metrics) => quote!(#metrics),
        None => quote!(|_, _| ::std::vec::Vec::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expand(args: &str, item: &str) -> syn::Result<String> {
        let args =
            syn::parse::Parser::parse_str(Punctuated::<Meta, Token![,]>::parse_terminated, args)?;
        expand_test(args, syn::parse_str(item)?).map(|tokens| tokens.to_string())
    }

    // `test` here would be this crate's own macro.
    #[::core::prelude::v1::test]
    fn only_properties_that_can_discard_get_a_strategy_check() {
        let discarding =
            expand("gen = pairs()", "fn p(a: u8, b: u8) -> TestResult { todo!() }").unwrap();
        assert!(discarding.contains("__ephyra_check_p"));
        for output in ["-> Result<TestResult, String>", "-> io::Result<Result<TestResult, String>>"]
        {
            let item = format!("fn p(a: u8, b: u8) {output} {{ todo!() }}");
            let expanded = expand("gen = pairs(), metrics = m", &item).unwrap();
            assert!(expanded.contains("__ephyra_check_p"), "{output}");
        }
        for output in [
            "-> bool",
            "",
            "-> ()",
            "-> Result<(), TestResult>",
            "-> Result<Result<bool, String>, String>",
            "-> fmt::Result",
        ] {
            let item = format!("fn p(a: u8, b: u8) {output} {{ todo!() }}");
            let expanded = expand("gen = pairs()", &item).unwrap();
            assert!(!expanded.contains("__ephyra_check_p"), "{output}");
            assert!(expand("gen = pairs(), metrics = m", &item).is_err(), "{output}");
        }
    }

    // `test` here would be this crate's own macro.
    #[::core::prelude::v1::test]
    fn parameters_take_strategies_of_their_own() {
        let expanded = expand(
            "",
            "fn p(n: u16, #[ephyra(gen = words())] s: String) -> TestResult { todo!() }",
        )
        .unwrap();
        assert!(expanded.contains(":: ephyra :: any :: < u16 > ()"), "{expanded}");
        assert!(expanded.contains("(words ())"), "{expanded}");
        assert!(expanded.contains("__ephyra_check_p"), "{expanded}");
        assert!(!expanded.contains("# [ephyra"), "the attribute is removed: {expanded}");
        assert!(expand("gen = both()", "fn p(#[ephyra(gen = one())] s: String) {}").is_err());
    }
}
