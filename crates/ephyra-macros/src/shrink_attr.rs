//! Expansion of classical eager shrinkers into Ephyra's visitor API.

use {
    proc_macro2::{
        Ident,
        Span,
        TokenStream,
        TokenTree,
    },
    quote::quote,
    std::{
        collections::HashSet,
        fmt::Display,
    },
    syn::{
        Block,
        Expr,
        ExprCall,
        ItemFn,
        Pat,
        ReturnType,
        Stmt,
        Type,
        parse::Parser,
        parse_quote,
        parse2,
        punctuated::Punctuated,
        spanned::Spanned,
        visit::Visit,
        visit_mut::VisitMut,
    },
};

pub(crate) fn expand(mut function: ItemFn) -> syn::Result<TokenStream> {
    if function.sig.asyncness.is_some() || function.sig.constness.is_some() {
        return Err(error(
            function.sig.span(),
            "cannot transform `async fn` or `const fn` functions; it only supports ordinary `fn` functions",
        ));
    }
    let candidate = vec_element(&function.sig.output).cloned().ok_or_else(|| {
        error(
            function.sig.output.span(),
            "transforms functions written as `fn(...) -> Vec<T>`, but this function returns a different type; if this is a QuickCheck-style iterator shrinker, collect its candidates into a `Vec<T>` first",
        )
    })?;
    let shrinker = Shrinker {
        function: function.sig.ident.clone(),
        visitor: fresh_internal_ident(&function, "__ephyra_shrinker_visitor"),
    };
    let visitor = &shrinker.visitor;
    function.sig.inputs.push(parse_quote! {
        #visitor: &mut dyn FnMut(#candidate) -> ::std::ops::ControlFlow<()>
    });
    function.sig.output = parse_quote!(-> ::std::ops::ControlFlow<()>);

    let mut returns = ReturnRewriter { shrinker: &shrinker, error: None };
    returns.visit_block_mut(&mut function.block);
    if let Some(error) = returns.error {
        return Err(error);
    }
    shrinker.transform_block(&mut function.block, None)?;
    shrinker.reject_unlowered_recursion(&function.block)?;
    Ok(quote!(#function))
}

/// A diagnostic at `span`, prefixed with the macro's name.
fn error(span: Span, message: impl Display) -> syn::Error {
    syn::Error::new(span, format!("the `#[ephyra::shrinker]` macro {message}"))
}

fn fresh_internal_ident(function: &ItemFn, base: &str) -> Ident {
    struct UsedIdentifiers(HashSet<String>);

    impl<'ast> Visit<'ast> for UsedIdentifiers {
        fn visit_ident(&mut self, ident: &'ast Ident) {
            self.0.insert(ident.to_string());
        }
    }

    let mut used = UsedIdentifiers(HashSet::new());
    used.visit_item_fn(function);
    let mut candidate = base.to_string();
    let mut suffix = 0usize;
    while used.0.contains(&candidate) {
        suffix += 1;
        candidate = format!("{base}_{suffix}");
    }
    Ident::new(&candidate, Span::mixed_site())
}

fn vec_element(output: &ReturnType) -> Option<&Type> {
    let ReturnType::Type(_, ty) = output else { return None };
    let Type::Path(path) = ty.as_ref() else { return None };
    let segment = path.path.segments.last()?;
    if segment.ident != "Vec" {
        return None;
    }
    let syn::PathArguments::AngleBracketed(arguments) = &segment.arguments else { return None };
    arguments.args.iter().find_map(|argument| {
        if let syn::GenericArgument::Type(ty) = argument { Some(ty) } else { None }
    })
}

/// The function being transformed: its name, which recursive calls are
/// recognized by, and the visitor parameter candidates are streamed to.
struct Shrinker {
    function: Ident,
    visitor: Ident,
}

/// The local Vec a block builds and returns as its candidates.
#[derive(Clone)]
struct CandidateVec {
    name: Ident,
    mutable: bool,
}

impl Shrinker {
    /// Stream the candidates `block` returns. `inherited` is the candidate Vec
    /// of an enclosing block that returns this block's value.
    fn transform_block(
        &self,
        block: &mut Block,
        inherited: Option<&CandidateVec>,
    ) -> syn::Result<()> {
        let expected = match block.stmts.last() {
            Some(Stmt::Expr(Expr::Path(path), _)) => path.path.get_ident().cloned(),
            _ => None,
        };
        let mut accumulator = inherited.cloned();
        let statements = std::mem::take(&mut block.stmts);
        let count = statements.len();
        for (index, mut statement) in statements.into_iter().enumerate() {
            if let Stmt::Local(local) = &statement
                && let Some(pattern) = pat_ident(&local.pat)
                && expected.as_ref() == Some(&pattern.ident)
            {
                let name = &pattern.ident;
                if accumulator.is_some() {
                    return Err(error(
                        local.span(),
                        "found a second Vec being built as this block's return value; build and return one shrink-candidate Vec per block",
                    ));
                }
                let initial = local.init.as_ref().map(|initial| initial.expr.as_ref());
                let candidates = match initial {
                    Some(initial) => self.candidates(initial)?,
                    None => None,
                };
                let Some(candidates) = candidates else {
                    return Err(error(
                        initial.map_or_else(|| local.span(), |initial| initial.span()),
                        format!(
                            "sees that `{name}` is returned as the shrink-candidate Vec, but it cannot stream candidates created by this initializer; initialize `{name}` with `vec![...]`, `Vec::new()`, `Vec::with_capacity(...)`, or an iterator `.collect()`"
                        ),
                    ));
                };
                block.stmts.extend(candidates);
                accumulator = Some(CandidateVec {
                    name: name.clone(),
                    mutable: pattern.mutability.is_some(),
                });
                continue;
            }
            if index + 1 == count
                && let Stmt::Expr(expression, _) = &mut statement
            {
                self.transform_tail(expression, accumulator.as_ref())?;
            } else if let Some(accumulator) = &accumulator {
                let mut uses = AccumulatorUses { shrinker: self, accumulator, error: None };
                uses.visit_stmt_mut(&mut statement);
                if let Some(error) = uses.error {
                    return Err(error);
                }
            }
            block.stmts.push(statement);
        }
        Ok(())
    }

    fn transform_tail(
        &self,
        expression: &mut Expr,
        accumulator: Option<&CandidateVec>,
    ) -> syn::Result<()> {
        if accumulator.is_some_and(|candidate_vec| is_path(expression, &candidate_vec.name)) {
            *expression = parse_quote!(::std::ops::ControlFlow::Continue(()));
            return Ok(());
        }
        if let Some(statements) = self.candidates(expression)? {
            *expression = parse_quote!({
                #(#statements)*
                ::std::ops::ControlFlow::Continue(())
            });
            return Ok(());
        }
        match expression {
            Expr::Block(block) => self.transform_block(&mut block.block, accumulator),
            Expr::Match(matched) => {
                for arm in &mut matched.arms {
                    self.transform_tail(&mut arm.body, accumulator)?;
                }
                Ok(())
            },
            Expr::If(condition) => {
                self.transform_block(&mut condition.then_branch, accumulator)?;
                match &mut condition.else_branch {
                    Some((_, otherwise)) => self.transform_tail(otherwise, accumulator),
                    None => Ok(()),
                }
            },
            Expr::Return(_) => Ok(()),
            _ => {
                Err(error(
                    expression.span(),
                    "cannot determine which shrink candidates this final expression returns; return `vec![...]`, an empty `Vec`, an iterator `.collect()` expression, or a locally built candidate Vec",
                ))
            },
        }
    }

    /// The statements that stream the candidates of a producer: `Vec::new()`,
    /// `Vec::with_capacity(…)`, an iterator `.collect()` or `vec![…]`. `None`
    /// if `expression` is none of these.
    fn candidates(&self, expression: &Expr) -> syn::Result<Option<Vec<Stmt>>> {
        if let Some(capacity) = empty_vec_capacity(expression) {
            return Ok(Some(
                capacity.map(|capacity| parse_quote!(let _ = #capacity;)).into_iter().collect(),
            ));
        }
        match expression {
            Expr::MethodCall(call) if call.method == "collect" && call.args.is_empty() => {
                let emission = self.emit_extend(&call.receiver)?;
                Ok(Some(vec![parse_quote!(#emission;)]))
            },
            Expr::Macro(mac) if mac.mac.path.is_ident("vec") => {
                if mac.mac.tokens.clone().into_iter().any(
                    |token| matches!(token, TokenTree::Punct(ref punct) if punct.as_char() == ';'),
                ) {
                    return Err(error(
                        mac.span(),
                        "cannot transform `vec![candidate; count]` because streaming would change when `candidate` is cloned; use an explicit loop with `candidates.push(...)`",
                    ));
                }
                let parser = Punctuated::<Expr, syn::Token![,]>::parse_terminated;
                let values = parser.parse2(mac.mac.tokens.clone())?;
                let visitor = &self.visitor;
                Ok(Some(values.into_iter().map(|value| parse_quote!(#visitor(#value)?;)).collect()))
            },
            _ => Ok(None),
        }
    }

    /// Stream the candidates of `candidates.extend(expression)`. A recursive
    /// call passes the visitor on instead of collecting.
    fn emit_extend(&self, expression: &Expr) -> syn::Result<Expr> {
        let visitor = &self.visitor;
        if let Some((call, closure)) = recursive_call(expression, &self.function) {
            let mut call = call.clone();
            call.args.push(match closure {
                Some(closure) => {
                    let (inputs, body) = (&closure.inputs, &closure.body);
                    parse_quote!(&mut |#inputs| #visitor(#body))
                },
                None => parse_quote!(#visitor),
            });
            return Ok(parse_quote!(#call?));
        }
        parse2(quote!({
            for candidate in #expression {
                #visitor(candidate)?;
            }
        }))
    }

    fn reject_unlowered_recursion(&self, block: &Block) -> syn::Result<()> {
        struct UnloweredRecursion<'a> {
            shrinker: &'a Shrinker,
            error: Option<syn::Error>,
        }

        impl<'ast> Visit<'ast> for UnloweredRecursion<'_> {
            fn visit_expr_call(&mut self, call: &'ast ExprCall) {
                let Shrinker { function, visitor } = self.shrinker;
                if calls(call, function)
                    && !call.args.iter().any(|argument| expression_uses_path(argument, visitor))
                {
                    self.error = Some(error(
                        call.span(),
                        format!(
                            "cannot stream this recursive call to `{function}` inside a longer iterator expression; use `candidates.extend({function}(child))` or `candidates.extend({function}(child).into_iter().map(...))`, and split additional iterator operations into separate `extend` statements"
                        ),
                    ));
                }
                syn::visit::visit_expr_call(self, call);
            }

            fn visit_item(&mut self, _item: &'ast syn::Item) {}
        }

        let mut recursion = UnloweredRecursion { shrinker: self, error: None };
        recursion.visit_block(block);
        recursion.error.map_or(Ok(()), Err)
    }
}

fn pat_ident(pattern: &Pat) -> Option<&syn::PatIdent> {
    match pattern {
        Pat::Ident(pattern) => Some(pattern),
        Pat::Type(pattern) => pat_ident(&pattern.pat),
        _ => None,
    }
}

/// Streams the candidates of every `return` in place.
struct ReturnRewriter<'a> {
    shrinker: &'a Shrinker,
    error: Option<syn::Error>,
}

impl VisitMut for ReturnRewriter<'_> {
    fn visit_expr_mut(&mut self, expression: &mut Expr) {
        match expression {
            Expr::Closure(_) | Expr::Async(_) => {},
            Expr::Return(returned) => {
                let Some(value) = &returned.expr else {
                    return;
                };
                if let Expr::Path(path) = value.as_ref() {
                    let returned_name = quote!(#path).to_string();
                    self.error = Some(error(
                        value.span(),
                        format!(
                            "cannot stream `return {returned_name};` because it returns the locally built shrink-candidate Vec before the end of the function; return `vec![...]` or an iterator `.collect()` from this branch, or restructure the function to return the Vec once as its final expression"
                        ),
                    ));
                    return;
                }
                match self.shrinker.candidates(value) {
                    Ok(Some(statements)) => {
                        *expression = parse_quote!({
                            #(#statements)*
                            return ::std::ops::ControlFlow::Continue(());
                        });
                    },
                    Ok(None) => {
                        self.error = Some(error(
                            value.span(),
                            "cannot determine which shrink candidates this return expression produces; use `vec![...]`, an empty `Vec`, or an iterator `.collect()` expression",
                        ));
                    },
                    Err(error) => self.error = Some(error),
                }
            },
            _ => syn::visit_mut::visit_expr_mut(self, expression),
        }
    }

    fn visit_item_mut(&mut self, _item: &mut syn::Item) {}
}

/// Streams the candidates a statement after the candidate Vec's declaration
/// adds with `push` or `extend`, and rejects every other use of the Vec.
struct AccumulatorUses<'a> {
    shrinker: &'a Shrinker,
    accumulator: &'a CandidateVec,
    error: Option<syn::Error>,
}

impl VisitMut for AccumulatorUses<'_> {
    fn visit_expr_mut(&mut self, expression: &mut Expr) {
        let (shrinker, accumulator) = (self.shrinker, self.accumulator);
        let name = &accumulator.name;
        let message = match expression {
            Expr::MethodCall(call) if is_path(&call.receiver, name) => {
                let method = &call.method;
                if method != "push" && method != "extend" {
                    format!(
                        "cannot transform `{name}.{method}(...)`: `{name}` is the Vec returned as shrink candidates, and this method needs the Vec as a whole while the transformed shrinker streams candidates one at a time; only `{name}.push(...)` and `{name}.extend(...)` are supported"
                    )
                } else if call.args.len() != 1 || expression_uses_path(&call.args[0], name) {
                    format!(
                        "cannot transform `{name}.{method}(...)` because its argument also reads `{name}`; a shrink-candidate Vec cannot be used to build itself while its candidates are streamed"
                    )
                } else if !accumulator.mutable {
                    format!(
                        "found candidates being added to `{name}`, but `{name}` is not mutable; declare it with `let mut {name} = ...`"
                    )
                } else {
                    let (visitor, argument) = (&shrinker.visitor, &call.args[0]);
                    let streamed = if method == "push" {
                        Ok(parse_quote!(#visitor(#argument)?))
                    } else {
                        shrinker.emit_extend(argument)
                    };
                    match streamed {
                        Ok(streamed) => *expression = streamed,
                        Err(error) => self.error = Some(error),
                    }
                    return;
                }
            },
            Expr::Path(path) if path.path.is_ident(name) => {
                format!(
                    "cannot transform this use of `{name}` because `{name}` is the Vec returned as shrink candidates and is borrowed, moved, or read here; only add candidates with `{name}.push(...)` or `{name}.extend(...)`, then return `{name}` as the final expression"
                )
            },
            Expr::Closure(_) | Expr::Async(_) if !expression_uses_path(expression, name) => return,
            Expr::Closure(_) => {
                format!(
                    "cannot transform this closure because it captures `{name}`, the Vec returned as shrink candidates, and cannot determine when the closure will add candidates; add them directly with `{name}.push(...)` or `{name}.extend(...)`"
                )
            },
            Expr::Async(_) => {
                format!(
                    "cannot transform this async block because it captures `{name}`, the Vec returned as shrink candidates; add candidates synchronously with `{name}.push(...)` or `{name}.extend(...)`"
                )
            },
            _ => {
                syn::visit_mut::visit_expr_mut(self, expression);
                return;
            },
        };
        self.error = Some(error(expression.span(), message));
    }

    fn visit_macro_mut(&mut self, mac: &mut syn::Macro) {
        let name = &self.accumulator.name;
        if tokens_use_ident(&mac.tokens, name) {
            self.error = Some(error(
                mac.span(),
                format!(
                    "cannot inspect how `{name}` is used inside this macro invocation; because `{name}` is returned as the shrink-candidate Vec, add candidates directly with `{name}.push(...)` or `{name}.extend(...)`"
                ),
            ));
        }
    }

    fn visit_item_mut(&mut self, _item: &mut syn::Item) {}
}

/// A recursive call to `function` whose candidates can go straight to the
/// visitor: `function(…)`, or `function(…).into_iter().map(closure)` with the
/// closure.
fn recursive_call<'a>(
    expression: &'a Expr,
    function: &Ident,
) -> Option<(&'a ExprCall, Option<&'a syn::ExprClosure>)> {
    let (call, closure) = match expression {
        Expr::Call(call) => (call, None),
        Expr::MethodCall(map) if map.method == "map" && map.args.len() == 1 => {
            let Expr::Closure(closure) = &map.args[0] else { return None };
            let Expr::MethodCall(into_iter) = map.receiver.as_ref() else { return None };
            if into_iter.method != "into_iter" || !into_iter.args.is_empty() {
                return None;
            }
            let Expr::Call(call) = into_iter.receiver.as_ref() else { return None };
            (call, Some(closure))
        },
        _ => return None,
    };
    calls(call, function).then_some((call, closure))
}

/// `Some(None)` for `Vec::new()` and `Some(Some(capacity))` for
/// `Vec::with_capacity(capacity)`.
fn empty_vec_capacity(expression: &Expr) -> Option<Option<&Expr>> {
    let Expr::Call(call) = expression else { return None };
    let Expr::Path(path) = call.func.as_ref() else { return None };
    let mut segments = path.path.segments.iter().rev();
    let constructor = segments.next()?;
    let vector = segments.next()?;
    if vector.ident != "Vec" {
        return None;
    }
    if constructor.ident == "new" && call.args.is_empty() {
        Some(None)
    } else if constructor.ident == "with_capacity" && call.args.len() == 1 {
        Some(call.args.first())
    } else {
        None
    }
}

fn is_path(expression: &Expr, name: &Ident) -> bool {
    matches!(expression, Expr::Path(path) if path.path.is_ident(name))
}

/// Whether `call` calls `function`, also through a qualified path such as
/// `Self::function`.
fn calls(call: &ExprCall, function: &Ident) -> bool {
    matches!(call.func.as_ref(), Expr::Path(path)
        if path.path.segments.last().is_some_and(|segment| segment.ident == *function))
}

fn expression_uses_path(expression: &Expr, name: &Ident) -> bool {
    struct PathUse<'name> {
        name: &'name Ident,
        found: bool,
    }

    impl<'ast> Visit<'ast> for PathUse<'_> {
        fn visit_expr_path(&mut self, path: &'ast syn::ExprPath) {
            self.found |= path.path.is_ident(self.name);
            syn::visit::visit_expr_path(self, path);
        }

        fn visit_macro(&mut self, mac: &'ast syn::Macro) {
            self.found |= tokens_use_ident(&mac.tokens, self.name);
        }
    }

    let mut path_use = PathUse { name, found: false };
    path_use.visit_expr(expression);
    path_use.found
}

fn tokens_use_ident(tokens: &TokenStream, name: &Ident) -> bool {
    tokens.clone().into_iter().any(|token| {
        match token {
            TokenTree::Ident(ident) => ident == *name,
            TokenTree::Group(group) => tokens_use_ident(&group.stream(), name),
            TokenTree::Punct(_) | TokenTree::Literal(_) => false,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expansion(source: ItemFn) -> String {
        expand(source).unwrap().to_string().replace(' ', "")
    }

    fn diagnostic(source: ItemFn) -> String {
        expand(source).unwrap_err().to_string()
    }

    fn assert_diagnostic(source: ItemFn, expected: &str) {
        let actual = diagnostic(source);
        assert!(actual.contains(expected), "expected {expected:?} in {actual:?}");
    }

    #[test]
    fn expands_recursive_candidate_vec_without_collecting() {
        let function = parse_quote! {
            fn shrink_tree(tree: &Tree) -> Vec<Tree> {
                match tree {
                    Tree::Leaf => Vec::new(),
                    Tree::Node(left, value, right) => {
                        let mut out = vec![(**left).clone(), (**right).clone()];
                        out.push(Tree::Leaf);
                        out.extend(shrink_tree(left).into_iter().map(|left| {
                            Tree::Node(Box::new(left), *value, right.clone())
                        }));
                        out
                    }
                }
            }
        };
        let expanded = expand(function).unwrap().to_string();
        let compact = expanded.replace(' ', "");
        assert!(compact.contains("__ephyra_shrinker_visitor((**left).clone())?"));
        assert!(compact.contains("shrink_tree(left,&mut|left|__ephyra_shrinker_visitor"));
        assert!(!compact.contains("letmutout"));
        syn::parse2::<ItemFn>(expanded.parse().unwrap()).unwrap();
    }

    // QuickCheck's built-in shrinkers use these producer shapes. QuickCheck
    // returns boxed iterators; the corpus below expresses the same shapes in
    // this macro's deliberately narrower `Vec<T>` source language.

    #[test]
    fn quickcheck_empty_singleton_and_match_shapes() {
        let compact = expansion(parse_quote! {
            fn shrink_bool(value: &bool) -> Vec<bool> {
                match value {
                    true => vec![false],
                    false => Vec::new(),
                }
            }
        });
        assert!(compact.contains("true=>{__ephyra_shrinker_visitor(false)?;"));
        assert!(compact.contains("false=>{::std::ops::ControlFlow::Continue(())}"));
    }

    #[test]
    fn quickcheck_chained_mapped_children_shape() {
        let compact = expansion(parse_quote! {
            fn shrink_pair(pair: &Pair) -> Vec<Pair> {
                let mut out = Vec::new();
                out.extend(shrink_pair(&pair.left).into_iter().map(|left| Pair {
                    left: Box::new(left),
                    right: pair.right.clone(),
                }));
                out.extend(shrink_pair(&pair.right).into_iter().map(|right| Pair {
                    left: pair.left.clone(),
                    right: Box::new(right),
                }));
                out
            }
        });
        assert_eq!(compact.matches("shrink_pair(").count(), 3);
        assert_eq!(compact.matches("&mut|").count(), 2);
        assert!(!compact.contains("into_iter"));
    }

    #[test]
    fn quickcheck_iterator_map_filter_map_flat_map_and_chain_shapes() {
        let compact = expansion(parse_quote! {
            fn shrink_scalar(value: &u32) -> Vec<char> {
                scalar_candidates(*value)
                    .map(|candidate| candidate / 2)
                    .filter(|candidate| *candidate != *value)
                    .filter_map(char::from_u32)
                    .flat_map(|candidate| [candidate, 'a'])
                    .chain(std::iter::once('\0'))
                    .collect()
            }
        });
        assert!(compact.contains("forcandidatein"));
        assert!(compact.contains(".filter_map(char::from_u32)"));
        assert!(compact.contains("__ephyra_shrinker_visitor(candidate)?"));
        assert!(!compact.contains(".collect()"));
    }

    #[test]
    fn quickcheck_custom_iterator_and_append_loop_shape() {
        let compact = expansion(parse_quote! {
            fn shrink_path(path: &Path) -> Vec<Path> {
                let mut shrunk = Vec::with_capacity(capacity_hint(path));
                for candidate in PathShrinker::new(path) {
                    if candidate.is_valid() {
                        shrunk.push(candidate);
                    }
                }
                shrunk
            }
        });
        assert!(compact.contains("let_=capacity_hint(path)"));
        assert!(compact.contains("forcandidateinPathShrinker::new(path)"));
        assert!(compact.contains("__ephyra_shrinker_visitor(candidate)?"));
    }

    #[test]
    fn quickcheck_iterator_collect_can_seed_the_candidate_vec() {
        let compact = expansion(parse_quote! {
            fn shrink_path(path: &Path) -> Vec<Path> {
                let mut shrunk: Vec<Path> = parents(path).collect();
                if let Ok(canonical) = path.canonicalize() {
                    shrunk.push(canonical);
                }
                shrunk
            }
        });
        assert!(compact.contains("forcandidateinparents(path)"));
        assert!(compact.contains("__ephyra_shrinker_visitor(canonical)?"));
    }

    #[test]
    fn explains_that_quickcheck_iterator_returns_need_adapting() {
        assert_diagnostic(
            parse_quote! {
                fn boxed(value: &u8) -> Box<dyn Iterator<Item = u8>> {
                    Box::new(std::iter::once(*value))
                }
            },
            "the `#[ephyra::shrinker]` macro transforms functions written as `fn(...) -> Vec<T>`",
        );
    }

    #[test]
    fn immutable_candidate_vec_is_allowed_when_it_is_not_extended() {
        let compact = expansion(parse_quote! {
            fn immutable(value: &u8) -> Vec<u8> {
                let out = vec![*value];
                out
            }
        });
        assert!(compact.contains("__ephyra_shrinker_visitor(*value)?"));
    }

    #[test]
    fn adding_to_an_immutable_candidate_vec_has_a_direct_diagnostic() {
        assert_diagnostic(
            parse_quote! {
                fn immutable(value: &u8) -> Vec<u8> {
                    let out = Vec::new();
                    out.push(*value);
                    out
                }
            },
            "found candidates being added to `out`, but `out` is not mutable",
        );
    }

    #[test]
    fn explains_which_candidate_vec_initializers_are_supported() {
        assert_diagnostic(
            parse_quote! {
                fn opaque(value: &u8) -> Vec<u8> {
                    let mut out = helper(value);
                    out
                }
            },
            "the `#[ephyra::shrinker]` macro sees that `out` is returned as the shrink-candidate Vec, but it cannot stream candidates created by this initializer",
        );
    }

    #[test]
    fn explains_why_the_candidate_vec_cannot_be_inspected() {
        assert_diagnostic(
            parse_quote! {
                fn observed(value: &u8) -> Vec<u8> {
                    let mut out = vec![*value];
                    let _ = out.len();
                    out
                }
            },
            "the `#[ephyra::shrinker]` macro cannot transform `out.len(...)`",
        );
    }

    #[test]
    fn explains_why_the_candidate_vec_cannot_be_passed_to_a_helper() {
        assert_diagnostic(
            parse_quote! {
                fn escaped(value: &u8) -> Vec<u8> {
                    let mut out = Vec::new();
                    helper(&mut out, value);
                    out
                }
            },
            "the `#[ephyra::shrinker]` macro cannot transform this use of `out` because `out` is the Vec returned as shrink candidates",
        );
    }

    #[test]
    fn explains_why_a_closure_cannot_capture_the_candidate_vec() {
        assert_diagnostic(
            parse_quote! {
                fn captured(value: &u8) -> Vec<u8> {
                    let mut out = Vec::new();
                    consume(|| out.push(*value));
                    out
                }
            },
            "the `#[ephyra::shrinker]` macro cannot transform this closure because it captures `out`",
        );
    }

    #[test]
    fn explains_why_vec_repetition_is_ambiguous_when_lazy() {
        assert_diagnostic(
            parse_quote! {
                fn repeated(value: &u8) -> Vec<u8> {
                    vec![*value; 3]
                }
            },
            "the `#[ephyra::shrinker]` macro cannot transform `vec![candidate; count]`",
        );
    }

    #[test]
    fn explains_how_to_replace_an_early_return_of_the_candidate_vec() {
        assert_diagnostic(
            parse_quote! {
                fn early(value: &u8) -> Vec<u8> {
                    let mut out = Vec::new();
                    if *value == 0 { return out; }
                    out.push(*value);
                    out
                }
            },
            "the `#[ephyra::shrinker]` macro cannot stream `return out;`",
        );
    }

    #[test]
    fn user_bindings_do_not_collide_with_the_generated_callback() {
        let compact = expansion(parse_quote! {
            fn allowed(visitor: &u8, __ephyra_shrinker_visitor: &u8) -> Vec<u8> {
                vec![*visitor, *__ephyra_shrinker_visitor]
            }
        });
        assert!(compact.contains("__ephyra_shrinker_visitor_1"));
    }

    #[test]
    fn explains_how_to_rewrite_an_unsupported_recursive_pipeline() {
        assert_diagnostic(
            parse_quote! {
                fn recursive_chain(value: &Tree) -> Vec<Tree> {
                    recursive_chain(value)
                        .into_iter()
                        .filter(valid)
                        .map(rebuild)
                        .collect()
                }
            },
            "the `#[ephyra::shrinker]` macro cannot stream this recursive call to `recursive_chain`",
        );
    }

    #[test]
    fn explains_why_a_macro_cannot_receive_the_candidate_vec() {
        assert_diagnostic(
            parse_quote! {
                fn through_macro(value: &u8) -> Vec<u8> {
                    let mut out = Vec::new();
                    add_candidate!(out, *value);
                    out
                }
            },
            "the `#[ephyra::shrinker]` macro cannot inspect how `out` is used inside this macro invocation",
        );
    }

    #[test]
    fn explains_why_async_and_const_functions_are_not_transformed() {
        assert_diagnostic(
            parse_quote! {
                async fn asynchronous(value: &u8) -> Vec<u8> {
                    vec![*value]
                }
            },
            "the `#[ephyra::shrinker]` macro cannot transform `async fn` or `const fn` functions",
        );
    }

    #[test]
    fn explains_when_the_candidate_vec_is_used_to_build_itself() {
        assert_diagnostic(
            parse_quote! {
                fn self_referential(value: &u8) -> Vec<u8> {
                    let mut out = vec![*value];
                    out.push(out[0]);
                    out
                }
            },
            "the `#[ephyra::shrinker]` macro cannot transform `out.push(...)` because its argument also reads `out`",
        );
    }

    #[test]
    fn let_else_blocks_are_checked_for_candidate_vec_uses() {
        assert_diagnostic(
            parse_quote! {
                fn head(values: &[u8]) -> Vec<u8> {
                    let mut out = Vec::new();
                    let [first, ..] = values else {
                        out.clear();
                        return Vec::new();
                    };
                    out.push(*first);
                    out
                }
            },
            "the `#[ephyra::shrinker]` macro cannot transform `out.clear(...)`",
        );
    }

    #[test]
    fn only_vec_macros_are_streamed_from_returns() {
        assert_diagnostic(
            parse_quote! {
                fn early(value: &u8) -> Vec<u8> {
                    if *value == 0 {
                        return smallvec![1, 2];
                    }
                    vec![*value]
                }
            },
            "the `#[ephyra::shrinker]` macro cannot determine which shrink candidates this return expression produces",
        );
    }
}
