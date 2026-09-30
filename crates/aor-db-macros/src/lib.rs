use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{
    Ident, LitStr, Token, Visibility,
    parse::{Parse, ParseStream},
    parse_macro_input,
};
struct Query {
    visibility: Visibility,
    name: Ident,
    dialect: Ident,
    migrations: LitStr,
    sql: LitStr,
}
impl Parse for Query {
    fn parse(p: ParseStream) -> syn::Result<Self> {
        let visibility = p.parse()?;
        let name = p.parse()?;
        p.parse::<Token![,]>()?;
        let dialect = p.parse()?;
        p.parse::<Token![,]>()?;
        let migrations = p.parse()?;
        p.parse::<Token![,]>()?;
        let sql = p.parse()?;
        Ok(Self {
            visibility,
            name,
            dialect,
            migrations,
            sql,
        })
    }
}
fn rust_type(c: &aor_sql::Column) -> proc_macro2::TokenStream {
    use aor_sql::Type::*;
    let ty = match &c.ty {
        Bool => quote!(bool),
        I32 => quote!(i32),
        I64 => quote!(i64),
        Text | Enum(_) => quote!(String),
        Uuid => quote!(::aor_db::Uuid),
        Timestamp => quote!(::aor_db::DateTime<::aor_db::Utc>),
        Bytes => quote!(Vec<u8>),
    };
    if c.nullable { quote!(Option<#ty>) } else { ty }
}
/// Defines a named typed query, checked against every committed migration.
#[proc_macro]
pub fn sql(input: TokenStream) -> TokenStream {
    let Query {
        visibility,
        name,
        dialect,
        migrations,
        sql,
    } = parse_macro_input!(input as Query);
    let result = (|| -> Result<_, String> {
        let portable = dialect == "portable";
        let d = match dialect.to_string().as_str() {
            "postgres" | "portable" => aor_sql::Dialect::Postgres,
            "sqlite" => aor_sql::Dialect::Sqlite,
            _ => return Err("dialect must be postgres, sqlite or portable".into()),
        };
        let dir = std::path::PathBuf::from(
            std::env::var("CARGO_MANIFEST_DIR").map_err(|e| e.to_string())?,
        )
        .join(migrations.value());
        let schema = aor_sql::Schema::from_dir(&dir, d)?;
        let checked = aor_sql::check(&schema, &sql.value(), d).map_err(|e| {
            format!("{e}; unsupported constructs require the audited sql_unchecked! escape hatch")
        })?;
        if portable {
            let local_schema = aor_sql::Schema::from_dir(&dir, aor_sql::Dialect::Sqlite)?;
            let local = aor_sql::check(&local_schema, &sql.value(), aor_sql::Dialect::Sqlite)
                .map_err(|e| e.to_string())?;
            if local.columns != checked.columns || local.parameters != checked.parameters {
                return Err(
                    "portable query types differ between dialects; use explicit dialects".into(),
                );
            }
        }
        let files = aor_sql::migration_files(&dir)?;
        let dependencies: Vec<_> = files
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        let row = format_ident!("{}Row", name);
        let fields: Vec<_> = checked
            .columns
            .iter()
            .map(|c| syn::parse_str::<Ident>(&format!("r#{}", c.name)).map_err(|e| e.to_string()))
            .collect::<Result<_, _>>()?;
        let types: Vec<_> = checked.columns.iter().map(rust_type).collect();
        let indices: Vec<_> = (0..fields.len()).collect();
        let parameters: Vec<_> = checked
            .parameters
            .iter()
            .map(|c| format_ident!("{}", c.name))
            .collect();
        let parameter_types: Vec<_> = checked.parameters.iter().map(rust_type).collect();
        let enum_params: Vec<_> = checked
            .parameters
            .iter()
            .map(|c| matches!(c.ty, aor_sql::Type::Enum(_)))
            .collect();
        let dialect = if portable {
            quote!(::aor_db::Executor::dialect(connection))
        } else {
            match d {
                aor_sql::Dialect::Postgres => quote!(::aor_db::Dialect::Postgres),
                aor_sql::Dialect::Sqlite => quote!(::aor_db::Dialect::Sqlite),
            }
        };
        Ok(quote! {
         #(const _: &str = include_str!(#dependencies);)*
         #visibility struct #name;
         #[derive(Debug,Clone,PartialEq)] #visibility struct #row {#(pub #fields:#types),*}
         impl #name {
          pub const SQL:&'static str=#sql;
          #[allow(clippy::too_many_arguments)] // One typed argument per checked SQL parameter.
          pub async fn query(connection:&mut impl ::aor_db::Executor,#(#parameters:#parameter_types),*)->::aor_db::Result<Vec<#row>>{
           let values=vec![#(::aor_db::parameter(#parameters,#enum_params)),*];
           let result=connection.query(#dialect,Self::SQL,&values).await?;
           result.rows.into_iter().map(|row|Ok(#row{#(#fields:row.get::<#types>(#indices)?),*})).collect()
          }
          #[allow(clippy::too_many_arguments)]
          pub async fn execute(connection:&mut impl ::aor_db::Executor,#(#parameters:#parameter_types),*)->::aor_db::Result<u64>{
           let values=vec![#(::aor_db::parameter(#parameters,#enum_params)),*];
           Ok(connection.query(#dialect,Self::SQL,&values).await?.affected)
          }
         }
        })
    })();
    match result {
        Ok(tokens) => tokens.into(),
        Err(e) => syn::Error::new_spanned(sql, e).to_compile_error().into(),
    }
}
