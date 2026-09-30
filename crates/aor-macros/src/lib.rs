use proc_macro::TokenStream;
use quote::quote;
use syn::{
    Ident, LitStr, Path, Token,
    parse::{Parse, ParseStream},
    parse_macro_input,
};
struct Route {
    method: Ident,
    path: LitStr,
    handler: Path,
}
impl Parse for Route {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        Ok(Self {
            method: input.parse()?,
            path: input.parse()?,
            handler: {
                input.parse::<Token![=>]>()?;
                input.parse()?
            },
        })
    }
}
/// Checks route grammar at compile time; constructs a public route. Protected route
/// registration is intentionally unavailable until the session/policy stack is implemented.
#[proc_macro]
pub fn route(input: TokenStream) -> TokenStream {
    let Route {
        method,
        path,
        handler,
    } = parse_macro_input!(input as Route);
    let m = method.to_string();
    let p = path.value();
    if !["GET", "HEAD", "POST", "PUT", "PATCH", "DELETE", "OPTIONS"].contains(&m.as_str()) {
        return syn::Error::new_spanned(method, "unsupported HTTP method")
            .to_compile_error()
            .into();
    }
    let mut names = std::collections::BTreeSet::new();
    if !p.starts_with('/')
        || p.contains('?')
        || p.contains('#')
        || p.contains('%')
        || p.contains('\\')
        || p.contains("//")
        || (p.len() > 1 && p.ends_with('/'))
        || !p.is_ascii()
        || p.bytes().any(|b| b < 33 || b == 127)
    {
        return syn::Error::new_spanned(path, "route must be a canonical absolute path")
            .to_compile_error()
            .into();
    }
    for segment in p.split('/').skip(1) {
        if segment.contains(['{', '}']) {
            let Some(name) = segment.strip_prefix('{').and_then(|s| s.strip_suffix('}')) else {
                return syn::Error::new_spanned(path, "parameters must occupy an entire segment")
                    .to_compile_error()
                    .into();
            };
            if name.is_empty()
                || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
                || name.as_bytes()[0].is_ascii_digit()
                || !names.insert(name.to_owned())
            {
                return syn::Error::new_spanned(path, "invalid or duplicate path parameter")
                    .to_compile_error()
                    .into();
            }
        }
    }
    quote! { ::aor_router::Route::public(#m,#path,stringify!(#handler),#handler) }.into()
}

#[proc_macro_derive(TemplateContext)]
pub fn context(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as syn::DeriveInput);
    let name = &input.ident;
    let syn::Data::Struct(data) = &input.data else {
        return syn::Error::new_spanned(name, "TemplateContext requires a struct")
            .to_compile_error()
            .into();
    };
    let syn::Fields::Named(fields) = &data.fields else {
        return syn::Error::new_spanned(name, "TemplateContext requires named fields")
            .to_compile_error()
            .into();
    };
    let names: Vec<_> = fields
        .named
        .iter()
        .map(|f| f.ident.as_ref().unwrap())
        .collect();
    let keys: Vec<_> = names.iter().map(|n| n.to_string()).collect();
    let types: Vec<_> = fields.named.iter().map(|f| &f.ty).collect();
    let (imp, ty, w) = input.generics.split_for_impl();
    quote! {
        impl #imp ::aor_tmpl::TemplateContext for #name #ty #w {
            fn schema()->::aor_tmpl::Schema { ::aor_tmpl::Schema::Object(::std::collections::BTreeMap::from([#((#keys.to_owned(),<#types as ::aor_tmpl::TemplateContext>::schema())),*])) }
            fn value(&self)->::aor_tmpl::Value { ::aor_tmpl::Value::Object(::std::collections::BTreeMap::from([#((#keys.to_owned(),::aor_tmpl::TemplateContext::value(&self.#names))),*])) }
        }
    }.into()
}

/// Adapt the fixed extractor vocabulary to the router's internal request context.
#[proc_macro_attribute]
pub fn handler(attr: TokenStream, input: TokenStream) -> TokenStream {
    if !attr.is_empty() {
        return syn::Error::new(
            proc_macro2::Span::call_site(),
            "handler accepts no attribute arguments",
        )
        .to_compile_error()
        .into();
    }
    let mut f = parse_macro_input!(input as syn::ItemFn);
    if f.sig.asyncness.is_none() || !f.sig.generics.params.is_empty() {
        return syn::Error::new_spanned(&f.sig, "handler must be an async, non-generic function")
            .to_compile_error()
            .into();
    }
    let mut types = Vec::new();
    let mut body_count = 0;
    for arg in &f.sig.inputs {
        let syn::FnArg::Typed(arg) = arg else {
            return syn::Error::new_spanned(arg, "handler cannot have self")
                .to_compile_error()
                .into();
        };
        let syn::Type::Path(path) = arg.ty.as_ref() else {
            return syn::Error::new_spanned(&arg.ty, "expected a closed AoR extractor type")
                .to_compile_error()
                .into();
        };
        let name = path.path.segments.last().unwrap().ident.to_string();
        if ![
            "Path",
            "Query",
            "Form",
            "Json",
            "Session",
            "Principal",
            "RequestId",
            "Body",
        ]
        .contains(&name.as_str())
        {
            return syn::Error::new_spanned(&arg.ty,"unsupported extractor; use Path, Query, Form, Json, Session, Principal, RequestId or Body").to_compile_error().into();
        };
        if ["Form", "Json", "Body"].contains(&name.as_str()) {
            body_count += 1
        }
        types.push(arg.ty.clone());
    }
    if body_count > 1 {
        return syn::Error::new_spanned(
            &f.sig.inputs,
            "a handler may consume the request body only once",
        )
        .to_compile_error()
        .into();
    }
    let name = f.sig.ident.clone();
    let inner = quote::format_ident!("__aor_handler_{}", name);
    f.sig.ident = inner.clone();
    let visibility = &f.vis;
    let output = &f.sig.output;
    let vars: Vec<_> = (0..types.len())
        .map(|i| quote::format_ident!("arg_{i}"))
        .collect();
    quote! {#f #visibility async fn #name(ctx: ::aor_router::Context) #output {
     let mut parts=::aor_router::Extraction::new(ctx);
     #(let #vars=<#types as ::aor_router::Extract>::extract(&mut parts).await?;)*
     #inner(#(#vars),*).await
    }}
    .into()
}
