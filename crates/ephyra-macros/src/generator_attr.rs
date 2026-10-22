// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

use {
    proc_macro2::TokenStream,
    quote::{
        format_ident,
        quote,
    },
    syn::{
        Expr,
        FnArg,
        ItemFn,
        Meta,
        Pat,
        Path,
        ReturnType,
        Signature,
        Token,
        Type,
        parse::{
            Parse,
            ParseStream,
        },
        punctuated::Punctuated,
        spanned::Spanned,
    },
};

/// `#[ephyra::generator(<predicate>, metrics = <fn>)]`, both optional.
#[derive(Default)]
pub(crate) struct Args {
    predicate: Option<Path>,
    metrics: Option<Expr>,
}

impl Parse for Args {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut args = Args::default();
        for meta in Punctuated::<Meta, Token![,]>::parse_terminated(input)? {
            match meta {
                Meta::Path(path) if args.predicate.is_none() => args.predicate = Some(path),
                Meta::NameValue(pair)
                    if pair.path.is_ident("metrics") && args.metrics.is_none() =>
                {
                    args.metrics = Some(pair.value);
                },
                other => {
                    return Err(syn::Error::new(
                        other.span(),
                        "expected `#[ephyra::generator(<predicate>, metrics = <fn>)]`",
                    ));
                },
            }
        }
        Ok(args)
    }
}

/// A predicate (returning `bool`) is kept as it is; a generator is kept and
/// gets the ignored test that `ephyra check` runs to check it.
pub(crate) fn expand(args: Args, func: ItemFn) -> syn::Result<TokenStream> {
    let sig = &func.sig;
    plain(sig)?;
    if returns_bool(sig) {
        predicate(&args, sig)?;
        return Ok(quote!(#func));
    }
    if !sig.inputs.is_empty() || matches!(sig.output, ReturnType::Default) {
        return Err(syn::Error::new(
            sig.span(),
            "a ephyra::generator is either a predicate `fn(&T) -> bool` or a generator \
             `fn() -> impl Gen<Value = T>` with no parameters",
        ));
    }
    let name = &sig.ident;
    let check = format_ident!("__ephyra_check_{name}");
    let cfg = crate::check_test_cfgs(&func.attrs);
    let predicate = match &args.predicate {
        Some(predicate) => quote!(|value| #predicate(value)),
        None => quote!(|_| true),
    };
    let metrics = crate::metrics_or_none(args.metrics.as_ref());
    Ok(quote! {
        #func

        #(#cfg)*
        #[test]
        #[ignore = "run by `ephyra check`"]
        fn #check() {
            let __report = ::ephyra::__check_generator(
                &::ephyra::Config::default().tests(10_000).name(stringify!(#name)),
                #name(),
                #predicate,
                #metrics,
            );
            ::std::println!("{__report}");
            __report.assert_passed();
        }
    })
}

/// A synchronous, safe, non-generic free function.
fn plain(sig: &Signature) -> syn::Result<()> {
    if !sig.generics.params.is_empty()
        || sig.generics.where_clause.is_some()
        || sig.asyncness.is_some()
        || sig.unsafety.is_some()
        || sig.constness.is_some()
        || sig.abi.is_some()
        || sig.variadic.is_some()
    {
        return Err(syn::Error::new(
            sig.span(),
            "ephyra::generator requires a synchronous, safe, non-generic function",
        ));
    }
    if let Some(FnArg::Receiver(receiver)) = sig.inputs.first() {
        return Err(syn::Error::new(receiver.span(), "ephyra::generator does not support methods"));
    }
    Ok(())
}

fn returns_bool(sig: &Signature) -> bool {
    matches!(&sig.output, ReturnType::Type(_, ty)
        if matches!(&**ty, Type::Path(p) if p.qself.is_none() && p.path.is_ident("bool")))
}

/// A predicate takes no arguments and one plain `&T` binding.
fn predicate(args: &Args, sig: &Signature) -> syn::Result<()> {
    if args.predicate.is_some() || args.metrics.is_some() {
        return Err(syn::Error::new(
            sig.span(),
            "a predicate takes no arguments; name it on its generator: \
             #[ephyra::generator(<predicate>)]",
        ));
    }
    let [FnArg::Typed(input)] = &sig.inputs.iter().collect::<Vec<_>>()[..] else {
        return Err(syn::Error::new(sig.span(), "a predicate has exactly one parameter, `&T`"));
    };
    let Pat::Ident(binding) = &*input.pat else {
        return Err(syn::Error::new(
            input.pat.span(),
            "predicate parameter must be a named binding",
        ));
    };
    if binding.subpat.is_some() || binding.by_ref.is_some() {
        return Err(syn::Error::new(binding.span(), "predicate parameter must be a plain binding"));
    }
    match &*input.ty {
        Type::Reference(ty) if ty.mutability.is_none() && ty.lifetime.is_none() => Ok(()),
        ty => {
            Err(syn::Error::new(
                ty.span(),
                "a predicate takes its value by an elided shared reference, `&T`",
            ))
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expand_str(args: &str, item: &str) -> syn::Result<String> {
        expand(syn::parse_str(args)?, syn::parse_str(item)?).map(|tokens| tokens.to_string())
    }

    #[test]
    fn predicates_are_kept_as_they_are() {
        let output = expand_str("", "pub fn valid(t: &Tree) -> bool { invariant(t) }").unwrap();
        assert_eq!(output, "pub fn valid (t : & Tree) -> bool { invariant (t) }");
    }

    #[test]
    fn generators_get_a_check_test_against_their_predicate_and_metrics() {
        let output = expand_str(
            "valid, metrics = tree_metrics",
            "fn trees() -> impl Gen<Value = Tree> { tree_gen() }",
        )
        .unwrap();
        assert!(output.contains("fn trees ()"));
        assert!(output.contains("fn __ephyra_check_trees ()"));
        assert!(output.contains("ignore"));
        assert!(output.contains("| value | valid (value)"));
        assert!(output.contains("tree_metrics"));
        syn::parse2::<syn::File>(output.parse().unwrap()).unwrap();

        let unpaired = expand_str("", "fn trees() -> BoxGen<Tree> { tree_gen() }").unwrap();
        assert!(unpaired.contains("| _ | true"));
    }

    #[test]
    fn rejects_unsupported_shapes() {
        for (args, source) in [
            ("", "fn p(x: u8) -> bool { x > 0 }"),
            ("", "fn p(x: &mut u8) -> bool { true }"),
            ("", "fn p<T>(x: &T) -> bool { true }"),
            ("", "async fn p(x: &u8) -> bool { true }"),
            ("", "unsafe fn p(x: &u8) -> bool { true }"),
            ("", "fn p(x: &u8, y: &u8) -> bool { true }"),
            ("", "fn p(&self) -> bool { true }"),
            ("other", "fn p(x: &u8) -> bool { true }"),
            ("", "fn g(size: usize) -> impl Gen<Value = u8> { gen(size) }"),
            ("", "fn g() {}"),
            ("valid, other", "fn g() -> impl Gen<Value = u8> { gen() }"),
        ] {
            assert!(expand_str(args, source).is_err(), "{args} / {source}");
        }
    }
}
