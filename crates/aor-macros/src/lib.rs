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
