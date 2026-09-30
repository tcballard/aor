//! Conservative checks for the scaffold's direct route → policy → service shape.
//! Arbitrary indirect dispatch is not inferred: unsupported owned routes fail closed.
use crate::Finding;
use quote::ToTokens;
use syn::{Expr, FnArg, GenericArgument, PathArguments, Type, visit::Visit};
fn finding(rule: &str, path: &str, detail: impl AsRef<str>) -> Finding {
    Finding {
        rule_id: rule.into(),
        status: "fail",
        detail: format!("{path}: {}", detail.as_ref()),
        public_gate: false,
    }
}
fn capability(sig: &syn::Signature) -> Option<(String, String)> {
    for arg in &sig.inputs {
        if let FnArg::Typed(arg) = arg {
            let ty = match &*arg.ty {
                Type::Reference(r) => &*r.elem,
                t => t,
            };
            if let Type::Path(p) = ty {
                let last = p.path.segments.last()?;
                if last.ident == "Authorized" {
                    if let PathArguments::AngleBracketed(a) = &last.arguments {
                        let types: Vec<_> = a
                            .args
                            .iter()
                            .filter_map(|a| {
                                if let GenericArgument::Type(Type::Path(t)) = a {
                                    Some(t.path.segments.last()?.ident.to_string())
                                } else {
                                    None
                                }
                            })
                            .collect();
                        if types.len() == 2 {
                            return Some((types[0].clone(), types[1].clone()));
                        }
                    }
                }
            }
        }
    }
    None
}
fn last(expr: &Expr) -> Option<String> {
    if let Expr::Path(p) = expr {
        p.path.segments.last().map(|s| s.ident.to_string())
    } else {
        None
    }
}
fn literal(expr: &Expr) -> Option<String> {
    if let Expr::Lit(l) = expr {
        if let syn::Lit::Str(s) = &l.lit {
            return Some(s.value());
        }
    }
    None
}
pub fn inspect(path: &str, source: &str, owned: bool) -> Result<Vec<Finding>, syn::Error> {
    let file = syn::parse_file(source)?;
    let mut out = Vec::new();
    struct Entities<'a> {
        path: &'a str,
        out: &'a mut Vec<Finding>,
    }
    impl<'ast> Visit<'ast> for Entities<'_> {
        fn visit_item_struct(&mut self, s: &'ast syn::ItemStruct) {
            let internal = s.ident.to_string().ends_with("Entity")
                || s.fields.iter().any(|f| {
                    f.ident.as_ref().is_some_and(|i| {
                        ["owner_id", "password_hash", "token_hash", "csrf_hash"]
                            .contains(&i.to_string().as_str())
                    })
                });
            if internal
                && s.attrs.iter().any(|a| {
                    a.path().is_ident("derive")
                        && a.meta
                            .to_token_stream()
                            .to_string()
                            .split(|c: char| !c.is_alphanumeric())
                            .any(|t| t == "Serialize")
                })
            {
                self.out.push(finding(
                    "AOR-WIRE-001",
                    self.path,
                    format!("entity {} derives wire Serialize", s.ident),
                ))
            }
            syn::visit::visit_item_struct(self, s);
        }
    }
    Entities {
        path,
        out: &mut out,
    }
    .visit_file(&file);
    if !owned {
        return Ok(out);
    }
    let services: std::collections::BTreeMap<_, _> = file
        .items
        .iter()
        .filter_map(|i| {
            if let syn::Item::Fn(f) = i {
                capability(&f.sig).map(|c| (f.sig.ident.to_string(), c))
            } else {
                None
            }
        })
        .collect();
    if services.is_empty() {
        out.push(finding(
            "AOR-POLICY-001",
            path,
            "owned resource has no Authorized service signatures",
        ));
    }
    struct Repositories<'a> {
        path: &'a str,
        out: &'a mut Vec<Finding>,
        inside: bool,
    }
    impl<'ast> Visit<'ast> for Repositories<'_> {
        fn visit_item_mod(&mut self, m: &'ast syn::ItemMod) {
            let old = self.inside;
            self.inside |= m.ident == "repository";
            syn::visit::visit_item_mod(self, m);
            self.inside = old;
        }
        fn visit_item_fn(&mut self, f: &'ast syn::ItemFn) {
            if self.inside {
                let tx=f.sig.inputs.iter().any(|a|if let FnArg::Typed(t)=a{if let Type::Reference(r)=&*t.ty{r.mutability.is_some()&&matches!(&*r.elem,Type::Path(p) if p.path.segments.last().is_some_and(|s|s.ident=="Tx"))}else{false}}else{false});
                if !tx || capability(&f.sig).is_none() {
                    self.out.push(finding(
                        "AOR-TX-001",
                        self.path,
                        format!("repository {} needs &mut Tx and &Authorized", f.sig.ident),
                    ))
                }
            }
            syn::visit::visit_item_fn(self, f);
        }
        fn visit_expr_method_call(&mut self, c: &'ast syn::ExprMethodCall) {
            if self.inside
                && ["begin", "acquire", "commit"].contains(&c.method.to_string().as_str())
            {
                self.out.push(finding(
                    "AOR-TX-001",
                    self.path,
                    "repository owns a transaction boundary",
                ))
            }
            syn::visit::visit_expr_method_call(self, c);
        }
    }
    Repositories {
        path,
        out: &mut out,
        inside: false,
    }
    .visit_file(&file);
    struct Calls {
        names: Vec<String>,
        policies: Vec<(String, String)>,
    }
    impl<'ast> Visit<'ast> for Calls {
        fn visit_expr_call(&mut self, c: &'ast syn::ExprCall) {
            if let Some(name) = last(&c.func) {
                self.names.push(name.clone());
                if name == "owner" {
                    if let Expr::Path(p) = &*c.func {
                        if let PathArguments::AngleBracketed(a) =
                            &p.path.segments.last().unwrap().arguments
                        {
                            let types: Vec<_> = a
                                .args
                                .iter()
                                .filter_map(|a| {
                                    if let GenericArgument::Type(Type::Path(t)) = a {
                                        Some(t.path.segments.last()?.ident.to_string())
                                    } else {
                                        None
                                    }
                                })
                                .collect();
                            if types.len() == 2 {
                                self.policies.push((types[0].clone(), types[1].clone()));
                            }
                        }
                    }
                }
            }
            syn::visit::visit_expr_call(self, c);
        }
    }
    struct Routes<'a> {
        path: &'a str,
        out: &'a mut Vec<Finding>,
        services: &'a std::collections::BTreeMap<String, (String, String)>,
        count: usize,
    }
    impl<'ast> Visit<'ast> for Routes<'_> {
        fn visit_expr_call(&mut self, c: &'ast syn::ExprCall) {
            if let Some(kind) = last(&c.func) {
                if kind == "protected" || kind == "public" {
                    self.count += 1;
                    let method = c.args.first().and_then(literal);
                    let closure = c.args.last();
                    let mut calls = Calls {
                        names: vec![],
                        policies: vec![],
                    };
                    if let Some(closure) = closure {
                        calls.visit_expr(closure)
                    }
                    let resolved: Vec<_> = calls
                        .names
                        .iter()
                        .filter_map(|n| self.services.get(n))
                        .collect();
                    if kind != "protected"
                        || method.is_none()
                        || resolved.len() != 1
                        || !calls
                            .policies
                            .iter()
                            .any(|p| resolved.first().is_some_and(|s| p == *s))
                    {
                        self.out.push(finding("AOR-POLICY-001",self.path,"route must directly resolve to one Authorized service and matching owner policy"));
                    } else {
                        let action = &resolved[0].1;
                        if ["GET", "HEAD"].contains(&method.as_deref().unwrap()) && action != "Read"
                        {
                            self.out.push(finding(
                                "AOR-GET-001",
                                self.path,
                                "GET/HEAD reaches a mutating service",
                            ));
                        }
                    }
                }
            }
            syn::visit::visit_expr_call(self, c);
        }
    }
    let mut routes = Routes {
        path,
        out: &mut out,
        services: &services,
        count: 0,
    };
    routes.visit_file(&file);
    if routes.count == 0 {
        out.push(finding(
            "AOR-POLICY-001",
            path,
            "no resolved route declarations",
        ));
    }
    Ok(out)
}

pub fn is_resource(source: &str) -> Result<bool, syn::Error> {
    struct Scan(bool);
    impl<'ast> Visit<'ast> for Scan {
        fn visit_item_impl(&mut self, i: &'ast syn::ItemImpl) {
            self.0 |= i
                .trait_
                .as_ref()
                .is_some_and(|(_, p, _)| p.segments.last().is_some_and(|s| s.ident == "Resource"));
            syn::visit::visit_item_impl(self, i);
        }
        fn visit_expr_call(&mut self, c: &'ast syn::ExprCall) {
            self.0 |= last(&c.func).is_some_and(|s| s == "protected");
            syn::visit::visit_expr_call(self, c);
        }
        fn visit_macro(&mut self, m: &'ast syn::Macro) {
            if m.path.segments.last().is_some_and(|s| s.ident == "sql") {
                self.0 |= m.tokens.to_string().contains("owner_id");
            }
            syn::visit::visit_macro(self, m);
        }
    }
    let mut scan = Scan(false);
    scan.visit_file(&syn::parse_file(source)?);
    Ok(scan.0)
}
pub fn middleware_is_fixed(source: &str) -> Result<bool, syn::Error> {
    let file = syn::parse_file(source)?;
    let Some(value) = file.items.iter().find_map(|i| {
        if let syn::Item::Const(c) = i {
            (c.ident == "AUTHENTICATED_MIDDLEWARE").then_some(&*c.expr)
        } else {
            None
        }
    }) else {
        return Ok(false);
    };
    let Expr::Reference(r) = value else {
        return Ok(false);
    };
    let Expr::Array(array) = &*r.expr else {
        return Ok(false);
    };
    let actual: Option<Vec<_>> = array.elems.iter().map(literal).collect();
    let expected = [
        "transport_limits",
        "request_id",
        "tracing",
        "security_headers",
        "csrf_form_buffer",
        "session_load",
        "csrf",
        "before_route",
        "route_match",
        "extractors",
        "handler",
        "after_handler",
        "error_mapping",
        "response_headers",
    ];
    Ok(actual.is_some_and(|a| a == expected))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn catches_entity_serialization() {
        let r = inspect(
            "entity.rs",
            "#[derive(Serialize)] struct AccountEntity { password_hash:String }",
            false,
        )
        .unwrap();
        assert_eq!(r[0].rule_id, "AOR-WIRE-001");
    }
    #[test]
    fn catches_get_mutation_and_missing_scope() {
        let r=inspect("resource.rs",r#"async fn update(scope: Authorized<Plugin, Update>) {} fn routes(){Route::protected("GET","/plugins","bad","owner",|ctx|{let scope=owner::<Plugin,Update>(ctx.principal());update(scope)});} mod repository{async fn insert(pool:Pool){pool.begin();}}"#,true).unwrap();
        assert!(r.iter().any(|f| f.rule_id == "AOR-GET-001"));
        assert!(r.iter().any(|f| f.rule_id == "AOR-TX-001"));
    }
    #[test]
    fn refuses_unresolved_handlers() {
        let r = inspect(
            "resource.rs",
            r#"fn routes(){Route::protected("GET","/x","bad","owner",handler);}"#,
            true,
        )
        .unwrap();
        assert!(r.iter().any(|f| f.rule_id == "AOR-POLICY-001"));
    }
}
