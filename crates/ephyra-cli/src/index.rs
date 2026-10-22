// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! A static index of a workspace's `#[ephyra::test]` and
//! `#[ephyra::generator]` functions, read from source with `syn` (no
//! build): what each one is, where it lives, and which generator serves which
//! predicate.
//!
//! The walk starts at every library and integration-test target, follows
//! `mod name;` declarations (honoring `#[path]`) and inline modules, and
//! recognizes the attributes by path, also inside `cfg_attr`. It does not
//! evaluate `cfg` or expand macros, and it resolves a generator's predicate by
//! the last segment of its path within the package: an ambiguous or unknown
//! name is reported, never guessed.

use std::{
    collections::{
        BTreeMap,
        BTreeSet,
    },
    fmt,
    fs,
    path::{
        Path,
        PathBuf,
    },
};

use {
    quote::ToTokens,
    syn::{
        Attribute,
        Expr,
        FnArg,
        GenericArgument,
        Item,
        ItemFn,
        Meta,
        Pat,
        PathArguments,
        ReturnType,
        Token,
        Type,
        TypeParamBound,
        punctuated::Punctuated,
        visit::Visit,
    },
};

use crate::serve::{
    Member,
    Target,
};

/// Every annotated function found, and what could not be read.
#[derive(Debug, Default)]
pub struct Index {
    pub items: Vec<Entry>,
    pub warnings: Vec<String>,
}

/// One annotated function.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub name: String,
    pub kind: Kind,
    pub package: String,
    /// Where the package's tests run.
    pub manifest_dir: PathBuf,
    /// The target the function was reached from (`lib` or `test`).
    pub target: Target,
    pub file: PathBuf,
    pub line: usize,
    /// The module path within the target, without the crate.
    pub module: Vec<String>,
}

/// What an annotated function is.
#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    /// `#[ephyra::test]`: its parameters as `(name, type)`, its strategy
    /// (`gen`, or the tuple of per-parameter `#[ephyra(gen = ...)]`s and
    /// defaults) and `metrics` expressions if given, whether its body uses
    /// `assume!`, and whether it can discard at all (it returns
    /// `TestResult`, or a type the index cannot tell).
    Property {
        parameters: Vec<(String, String)>,
        strategy: Option<String>,
        metrics: Option<String>,
        assumes: bool,
        discards: bool,
    },
    /// `#[ephyra::generator]` on `fn(&T) -> bool`.
    Predicate { input: String },
    /// `#[ephyra::generator]` on a generator function: the value type
    /// when the return type shows it, the predicate it names, and `metrics`.
    Generator { output: Option<String>, predicate: Option<String>, metrics: Option<String> },
}

impl Entry {
    /// Whether `ephyra check` has a check test to run for it: every
    /// generator, and a property with a strategy that can discard (as
    /// `#[ephyra::test]` decides).
    pub fn checked(&self) -> bool {
        matches!(
            self.kind,
            Kind::Generator { .. } | Kind::Property { strategy: Some(_), discards: true, .. }
        )
    }

    /// The name of its check test, without the module path.
    pub fn check_test(&self) -> String {
        format!("__ephyra_check_{}", self.name)
    }
}

impl fmt::Display for Entry {
    /// `name (file:line)`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({}:{})", self.name, self.file.display(), self.line)
    }
}

/// How generators and predicates pair up.
#[derive(Debug, Default)]
pub struct Pairing<'a> {
    /// The predicate each generator names, when it resolves to exactly one.
    pub predicates: BTreeMap<usize, usize>,
    /// Predicates no generator names.
    pub unserved: Vec<&'a Entry>,
    /// Generators whose predicate names none, or several, predicates.
    pub unresolved: Vec<(&'a Entry, String)>,
}

impl Index {
    /// Index every library and integration-test target of `members`.
    pub(crate) fn of(members: &[Member]) -> Index {
        let mut index = Index::default();
        let mut seen = BTreeSet::new();
        for member in members {
            for target in &member.targets {
                if !matches!(target.kind.as_str(), "lib" | "rlib" | "test") {
                    continue;
                }
                let mut walk = Walk { member, target, seen: &mut seen, index: &mut index };
                let children = target.src_path.parent().unwrap_or(Path::new(""));
                walk.file(&target.src_path, &[], children);
            }
        }
        index
    }

    /// Pair every generator with the predicate it names.
    pub fn pairing(&self) -> Pairing<'_> {
        let mut pairing = Pairing::default();
        let mut served = BTreeSet::new();
        for (at, entry) in self.items.iter().enumerate() {
            let Kind::Generator { predicate: Some(path), .. } = &entry.kind else { continue };
            let wanted = path.rsplit("::").next().unwrap_or(path).trim();
            let matches: Vec<usize> = self
                .items
                .iter()
                .enumerate()
                .filter(|(_, other)| {
                    matches!(other.kind, Kind::Predicate { .. })
                        && other.package == entry.package
                        && other.name == wanted
                })
                .map(|(at, _)| at)
                .collect();
            match matches[..] {
                [predicate] => {
                    pairing.predicates.insert(at, predicate);
                    served.insert(predicate);
                },
                [] => {
                    pairing.unresolved.push((
                        entry,
                        format!(
                            "names `{path}`, which is no #[ephyra::generator] predicate in {}",
                            entry.package
                        ),
                    ));
                },
                _ => {
                    pairing.unresolved.push((
                        entry,
                        format!(
                            "names `{path}`, which matches {} predicates in {}",
                            matches.len(),
                            entry.package
                        ),
                    ));
                },
            }
        }
        pairing.unserved = self
            .items
            .iter()
            .enumerate()
            .filter(|(at, entry)| {
                matches!(entry.kind, Kind::Predicate { .. }) && !served.contains(at)
            })
            .map(|(_, entry)| entry)
            .collect();
        pairing
    }
}

/// The walk of one target's module tree.
struct Walk<'a> {
    member: &'a Member,
    target: &'a Target,
    /// Files already walked, by any target: the first one keeps them.
    seen: &'a mut BTreeSet<PathBuf>,
    index: &'a mut Index,
}

impl Walk<'_> {
    /// Walk `file`, the module at `module`, whose `mod name;` children live
    /// in `children`.
    fn file(&mut self, file: &Path, module: &[String], children: &Path) {
        if !self.seen.insert(file.to_path_buf()) {
            return;
        }
        let parsed = fs::read_to_string(file)
            .map_err(|e| e.to_string())
            .and_then(|source| syn::parse_file(&source).map_err(|e| e.to_string()));
        match parsed {
            Ok(parsed) => self.items(&parsed.items, file, module, children),
            Err(e) => self.index.warnings.push(format!("could not read {}: {e}", file.display())),
        }
    }

    fn items(&mut self, items: &[Item], file: &Path, module: &[String], children: &Path) {
        for item in items {
            match item {
                Item::Fn(function) => self.function(function, file, module),
                Item::Mod(declaration) => {
                    let name = declaration.ident.to_string();
                    let mut inner = module.to_vec();
                    inner.push(name.clone());
                    let path = path_attribute(&declaration.attrs);
                    if let Some((_, items)) = &declaration.content {
                        let children = match &path {
                            Some(path) => children.join(path),
                            None => children.join(&name),
                        };
                        self.items(items, file, &inner, &children);
                    } else {
                        let base = file.parent().unwrap_or(Path::new(""));
                        let found = match &path {
                            Some(path) => Some(base.join(path)),
                            None => {
                                [
                                    children.join(format!("{name}.rs")),
                                    children.join(&name).join("mod.rs"),
                                ]
                                .into_iter()
                                .find(|candidate| candidate.exists())
                            },
                        };
                        match found {
                            Some(found) => {
                                // `a.rs` keeps its children in `a/`; `a/mod.rs`
                                // and `#[path]` files in their own directory.
                                let next = if path.is_none()
                                    && found.file_name() != Some("mod.rs".as_ref())
                                {
                                    children.join(&name)
                                } else {
                                    found.parent().unwrap_or(Path::new("")).to_path_buf()
                                };
                                self.file(&found, &inner, &next);
                            },
                            None => {
                                self.index
                                    .warnings
                                    .push(format!("{}: no file for `mod {name};`", file.display()));
                            },
                        }
                    }
                },
                _ => {},
            }
        }
    }

    fn function(&mut self, function: &ItemFn, file: &Path, module: &[String]) {
        let Some((which, args)) = ephyra_attribute(&function.attrs) else { return };
        let kind = match which {
            Which::Test => property(function, &args),
            Which::Generator => generator(function, &args),
        };
        let line = function.sig.ident.span().start().line;
        if matches!(kind, Kind::Predicate { .. }) && always_true(function) {
            self.index.warnings.push(format!(
                "{}:{line}: predicate `{}` always returns true; a generator whose every value is \
                 valid needs no predicate: put a bare #[ephyra::generator] on the generator",
                file.display(),
                function.sig.ident
            ));
        }
        self.index.items.push(Entry {
            name: function.sig.ident.to_string(),
            kind,
            package: self.member.name.clone(),
            manifest_dir: self.member.manifest_dir.clone(),
            target: self.target.clone(),
            file: file.to_path_buf(),
            line,
            module: module.to_vec(),
        });
    }
}

#[derive(Clone, Copy)]
enum Which {
    Test,
    Generator,
}

/// The first `ephyra::test` or `ephyra::generator` attribute, also
/// inside `cfg_attr(condition, ...)`, with its arguments.
fn ephyra_attribute(attrs: &[Attribute]) -> Option<(Which, Vec<Meta>)> {
    attrs.iter().find_map(|attribute| {
        if attribute.path().is_ident("cfg_attr") {
            let nested =
                attribute.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated).ok()?;
            nested.iter().skip(1).find_map(recognize)
        } else {
            recognize(&attribute.meta)
        }
    })
}

fn recognize(meta: &Meta) -> Option<(Which, Vec<Meta>)> {
    let segments: Vec<String> =
        meta.path().segments.iter().map(|segment| segment.ident.to_string()).collect();
    let which = match segments.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        ["ephyra", "test"] => Which::Test,
        ["ephyra", "generator"] => Which::Generator,
        _ => return None,
    };
    let args = match meta {
        Meta::List(list) => {
            list.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
                .map(|args| args.into_iter().collect())
                .unwrap_or_default()
        },
        _ => Vec::new(),
    };
    Some((which, args))
}

/// The value of `key = value` among `args`, as source text.
fn value_of(args: &[Meta], key: &str) -> Option<String> {
    args.iter().find_map(|meta| {
        match meta {
            Meta::NameValue(pair) if pair.path.is_ident(key) => Some(tidy(&pair.value)),
            _ => None,
        }
    })
}

fn property(function: &ItemFn, args: &[Meta]) -> Kind {
    let mut parameters = Vec::new();
    // Each parameter's `#[ephyra(gen = ...)]`, or its default.
    let mut generators: Vec<Result<String, String>> = Vec::new();
    for input in &function.sig.inputs {
        let FnArg::Typed(typed) = input else { continue };
        let Pat::Ident(binding) = &*typed.pat else { continue };
        let ty = tidy(&typed.ty);
        let own = typed.attrs.iter().filter(|a| a.path().is_ident("ephyra")).find_map(|a| {
            let pair: syn::MetaNameValue = a.parse_args().ok()?;
            pair.path.is_ident("gen").then(|| tidy(&pair.value))
        });
        generators.push(own.ok_or_else(|| format!("any::<{ty}>()")));
        parameters.push((binding.ident.to_string(), ty));
    }
    let per_parameter = generators.iter().any(Result::is_ok).then(|| {
        let generators: Vec<String> =
            generators.into_iter().map(|g| g.unwrap_or_else(|d| d)).collect();
        match &generators[..] {
            [only] => only.clone(),
            _ => format!("({})", generators.join(", ")),
        }
    });
    let mut assumes = Assumes(false);
    assumes.visit_block(&function.block);
    Kind::Property {
        parameters,
        strategy: value_of(args, "gen").or(per_parameter),
        metrics: value_of(args, "metrics"),
        assumes: assumes.0,
        discards: can_discard(&function.sig.output),
    }
}

/// Whether a property returning `output` can discard an input, as
/// `#[ephyra::test]` decides: not when it returns `()`, `bool` or
/// `Result`.
fn can_discard(output: &ReturnType) -> bool {
    let ReturnType::Type(_, ty) = output else { return false };
    match &**ty {
        Type::Tuple(tuple) => !tuple.elems.is_empty(),
        Type::Path(path) if path.qself.is_none() => {
            path.path
                .segments
                .last()
                .is_none_or(|last| last.ident != "bool" && last.ident != "Result")
        },
        _ => true,
    }
}

/// Whether a function's body is just `true`.
fn always_true(function: &ItemFn) -> bool {
    matches!(
        &function.block.stmts[..],
        [syn::Stmt::Expr(Expr::Lit(syn::ExprLit { lit: syn::Lit::Bool(b), .. }), None)] if b.value
    )
}

fn generator(function: &ItemFn, args: &[Meta]) -> Kind {
    let returns_bool = matches!(&function.sig.output, ReturnType::Type(_, ty)
        if matches!(&**ty, Type::Path(p) if p.path.is_ident("bool")));
    if returns_bool {
        let input = function.sig.inputs.first().and_then(|input| {
            match input {
                FnArg::Typed(typed) => {
                    Some(match &*typed.ty {
                        Type::Reference(reference) => tidy(&reference.elem),
                        other => tidy(other),
                    })
                },
                FnArg::Receiver(_) => None,
            }
        });
        return Kind::Predicate { input: input.unwrap_or_default() };
    }
    let predicate = args.iter().find_map(|meta| {
        match meta {
            Meta::Path(path) => Some(tidy(path)),
            _ => None,
        }
    });
    let output = match &function.sig.output {
        ReturnType::Type(_, ty) => generated_type(ty),
        ReturnType::Default => None,
    };
    Kind::Generator { output, predicate, metrics: value_of(args, "metrics") }
}

/// `T` in `impl Gen<Value = T>` or `BoxGen<T>`.
fn generated_type(ty: &Type) -> Option<String> {
    match ty {
        Type::ImplTrait(bounds) => {
            bounds.bounds.iter().find_map(|bound| {
                let TypeParamBound::Trait(bound) = bound else { return None };
                let last = bound.path.segments.last()?;
                if last.ident != "Gen" {
                    return None;
                }
                let PathArguments::AngleBracketed(args) = &last.arguments else { return None };
                args.args.iter().find_map(|arg| {
                    match arg {
                        GenericArgument::AssocType(assoc) if assoc.ident == "Value" => {
                            Some(tidy(&assoc.ty))
                        },
                        _ => None,
                    }
                })
            })
        },
        Type::Path(path) => {
            let last = path.path.segments.last()?;
            if last.ident != "BoxGen" {
                return None;
            }
            let PathArguments::AngleBracketed(args) = &last.arguments else { return None };
            match args.args.first()? {
                GenericArgument::Type(ty) => Some(tidy(ty)),
                _ => None,
            }
        },
        _ => None,
    }
}

/// Whether a body invokes `assume!`.
struct Assumes(bool);

impl<'ast> Visit<'ast> for Assumes {
    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        if mac.path.segments.last().is_some_and(|segment| segment.ident == "assume") {
            self.0 = true;
        }
    }
}

/// The `#[path = "..."]` of a module declaration.
fn path_attribute(attrs: &[Attribute]) -> Option<String> {
    attrs.iter().find_map(|attribute| {
        let Meta::NameValue(pair) = &attribute.meta else { return None };
        if !pair.path.is_ident("path") {
            return None;
        }
        match &pair.value {
            Expr::Lit(syn::ExprLit { lit: syn::Lit::Str(path), .. }) => Some(path.value()),
            _ => None,
        }
    })
}

/// Tokens as compact source text: `Vec<u8>`, not `Vec < u8 >`.
fn tidy(tokens: &impl ToTokens) -> String {
    let text = tokens.to_token_stream().to_string();
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == ' ' {
            let previous = out.chars().last();
            let next = chars.peek().copied();
            let tight_after = matches!(previous, Some('<' | '&' | '(' | '[' | ':' | '!'));
            let tight_before = matches!(next, Some('<' | '>' | ',' | ')' | ']' | ':' | '(' | '!'));
            if tight_after || tight_before {
                continue;
            }
        }
        out.push(c);
    }
    out
}

impl Kind {
    /// A one-line description for listings.
    pub fn describe(&self) -> String {
        match self {
            Kind::Property { parameters, strategy, discards, .. } => {
                let types: Vec<&str> = parameters.iter().map(|(_, ty)| ty.as_str()).collect();
                let types = types.join(", ");
                match strategy {
                    Some(strategy) if *discards => {
                        format!("property over ({types}) with strategy `{strategy}`")
                    },
                    Some(strategy) => {
                        format!(
                            "property over ({types}) with strategy `{strategy}` (never discards, so not checked)"
                        )
                    },
                    None => format!("property over ({types})"),
                }
            },
            Kind::Predicate { input } => format!("predicate over {input}"),
            Kind::Generator { output, predicate, .. } => {
                let output = output.as_deref().unwrap_or("?");
                match predicate {
                    Some(predicate) => format!("generator of {output} for `{predicate}`"),
                    None => format!("generator of {output}"),
                }
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &Path, file: &str, source: &str) {
        let path = root.join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, source).unwrap();
    }

    fn target(root: &Path, kind: &str, name: &str, file: &str) -> Target {
        Target { kind: kind.into(), name: name.into(), src_path: root.join(file) }
    }

    fn fixture() -> (tempfile::TempDir, Index) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(
            root,
            "src/lib.rs",
            "mod a;\n#[path = \"other/b_file.rs\"]\nmod b;\nmod inline { mod c; }\nmod broken;\n\n\
             #[ephyra::generator]\nfn sorted(v: &[u8]) -> bool { v.is_sorted() }\n",
        );
        write(
            root,
            "src/a.rs",
            "mod nested;\n#[ephyra::generator(crate::sorted, metrics = m)]\n\
             fn sorted_bytes() -> impl ephyra::Gen<Value = Vec<u8>> { todo!() }\n",
        );
        write(
            root,
            "src/a/nested.rs",
            "#[cfg_attr(feature = \"derive\", ephyra::generator)]\nfn positive(v: &u8) -> bool { *v > 0 }\n",
        );
        write(
            root,
            "src/other/b_file.rs",
            "#[ephyra::generator(missing)]\nfn orphan() -> gens::BoxGen<u8> { todo!() }\n",
        );
        write(
            root,
            "src/inline/c.rs",
            "#[ephyra::test(tests = 5, gen = pairs(), metrics = gap)]\n\
             fn prop(lo: u8, hi: u8) -> TestResult { ephyra::assume!(lo < hi); TestResult::Pass }\n",
        );
        write(root, "src/broken.rs", "fn (\n");
        write(
            root,
            "tests/t.rs",
            "mod common;\n#[ephyra::test]\nfn plain(v: Vec<u8>) -> bool { v.len() < 1000 }\n\
             #[ephyra::test(gen = words())]\nfn never_discards(w: String) -> bool { w.len() < 99 }\n\
             #[ephyra::test]\nfn per_parameter(n: u16, #[ephyra(gen = words())] s: String) -> TestResult { todo!() }\n\
             #[ephyra::generator]\nfn anything(_value: &u8) -> bool {\n    true\n}\n",
        );
        write(root, "tests/t2.rs", "mod common;\n");
        write(
            root,
            "tests/common/mod.rs",
            "#[ephyra::generator]\nfn shared(v: &u8) -> bool { true }\n",
        );
        let member = Member {
            name: "demo".into(),
            manifest_dir: root.to_path_buf(),
            targets: vec![
                target(root, "lib", "demo", "src/lib.rs"),
                target(root, "test", "t", "tests/t.rs"),
                target(root, "test", "t2", "tests/t2.rs"),
                target(root, "bin", "demo", "src/main.rs"),
            ],
            ..Member::default()
        };
        let index = Index::of(&[member]);
        (dir, index)
    }

    fn entry<'a>(index: &'a Index, name: &str) -> &'a Entry {
        let found: Vec<&Entry> = index.items.iter().filter(|entry| entry.name == name).collect();
        assert_eq!(found.len(), 1, "{name}: {:?}", index.items);
        found[0]
    }

    #[test]
    fn the_walk_follows_modules_path_attributes_and_inline_modules() {
        let (_dir, index) = fixture();
        assert_eq!(entry(&index, "sorted").module, Vec::<String>::new());
        assert_eq!(entry(&index, "sorted").line, 8);
        assert_eq!(entry(&index, "sorted_bytes").module, ["a"]);
        assert_eq!(entry(&index, "positive").module, ["a", "nested"]);
        assert_eq!(entry(&index, "orphan").module, ["b"]);
        assert_eq!(entry(&index, "prop").module, ["inline", "c"]);
        assert!(entry(&index, "prop").file.ends_with("src/inline/c.rs"));
        assert_eq!(entry(&index, "plain").target.name, "t");
        // A module two test targets share is indexed once, for the first.
        assert_eq!(entry(&index, "shared").target.name, "t");
        assert_eq!(entry(&index, "shared").module, ["common"]);
        assert_eq!(index.warnings.len(), 3, "{:?}", index.warnings);
        assert!(index.warnings.iter().any(|w| w.contains("broken.rs")));
        assert!(
            index.warnings.iter().any(|w| w.contains("predicate `anything` always returns true")),
            "{:?}",
            index.warnings
        );
    }

    #[test]
    fn functions_are_classified_from_their_signatures_and_arguments() {
        let (_dir, index) = fixture();
        assert_eq!(entry(&index, "sorted").kind, Kind::Predicate { input: "[u8]".into() });
        assert_eq!(entry(&index, "positive").kind, Kind::Predicate { input: "u8".into() });
        assert_eq!(
            entry(&index, "sorted_bytes").kind,
            Kind::Generator {
                output: Some("Vec<u8>".into()),
                predicate: Some("crate::sorted".into()),
                metrics: Some("m".into()),
            }
        );
        assert_eq!(
            entry(&index, "orphan").kind,
            Kind::Generator {
                output: Some("u8".into()),
                predicate: Some("missing".into()),
                metrics: None
            }
        );
        assert_eq!(
            entry(&index, "prop").kind,
            Kind::Property {
                parameters: vec![("lo".into(), "u8".into()), ("hi".into(), "u8".into())],
                strategy: Some("pairs()".into()),
                metrics: Some("gap".into()),
                assumes: true,
                discards: true,
            }
        );
        let checked: Vec<&str> =
            index.items.iter().filter(|e| e.checked()).map(|e| e.name.as_str()).collect();
        // `never_discards` has a strategy but returns `bool`: nothing to check.
        let Kind::Property { strategy, discards, .. } = &entry(&index, "never_discards").kind
        else {
            panic!()
        };
        assert_eq!((strategy.as_deref(), *discards), (Some("words()"), false));
        let Kind::Property { strategy, .. } = &entry(&index, "per_parameter").kind else {
            panic!()
        };
        assert_eq!(strategy.as_deref(), Some("(any::<u16>(), words())"));
        assert_eq!(checked.len(), 4, "{checked:?}");
        for name in ["sorted_bytes", "orphan", "prop", "per_parameter"] {
            assert!(checked.contains(&name), "{checked:?}");
        }
    }

    #[test]
    fn generators_pair_with_the_predicates_they_name() {
        let (_dir, index) = fixture();
        let pairing = index.pairing();
        let name = |at: &usize| index.items[*at].name.as_str();
        let paired: Vec<(&str, &str)> = pairing
            .predicates
            .iter()
            .map(|(generator, predicate)| (name(generator), name(predicate)))
            .collect();
        assert_eq!(paired, [("sorted_bytes", "sorted")]);
        let mut unserved: Vec<&str> = pairing.unserved.iter().map(|e| e.name.as_str()).collect();
        unserved.sort();
        assert_eq!(unserved, ["anything", "positive", "shared"]);
        assert_eq!(pairing.unresolved.len(), 1);
        assert_eq!(pairing.unresolved[0].0.name, "orphan");
        assert!(pairing.unresolved[0].1.contains("`missing`"));
    }

    #[test]
    fn tidy_prints_types_compactly() {
        let ty: Type = syn::parse_quote!(&'static [Vec<(u8, String)>]);
        assert_eq!(tidy(&ty), "&'static [Vec<(u8, String)>]");
        let path: syn::Path = syn::parse_quote!(crate::a::sorted);
        assert_eq!(tidy(&path), "crate::a::sorted");
    }
}
