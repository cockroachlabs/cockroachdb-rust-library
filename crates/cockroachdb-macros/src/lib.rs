//! Attribute macros for the CockroachDB Rust library — the AOP analogue.
//!
//! Rust has no runtime aspect weaving, but a proc-macro can rewrite a whole
//! `async fn` at compile time, which is what Spring's `@Transactional` proxy does
//! at runtime. [`macro@transactional`] wraps a function body in a retrying
//! transaction; [`macro@follower_read`] wraps it in a follower read.
//!
//! These complement the lighter `transactional!` / `follower_read!` `macro_rules!`
//! in `cockroachdb-sqlx`. Use whichever reads better for your codebase.

use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, Data, DeriveInput, Fields, Ident, ItemFn};

/// Wraps an `async fn` body in a retrying transaction.
///
/// The annotated function must take the pool as a parameter (named `pool` by
/// default, or `#[transactional(db)]` to name a different one) and return
/// `Result<T, sqlx::Error>`. Inside the body, use the injected `conn` binding
/// (`&mut sqlx::PgConnection`) for queries.
///
/// ```ignore
/// #[transactional]
/// async fn transfer(pool: &Pool, from: i64, to: i64, cents: i64) -> Result<(), sqlx::Error> {
///     sqlx::query("UPDATE accounts SET balance = balance - $1 WHERE id = $2")
///         .bind(cents).bind(from).execute(&mut *conn).await?;
///     sqlx::query("UPDATE accounts SET balance = balance + $1 WHERE id = $2")
///         .bind(cents).bind(to).execute(&mut *conn).await?;
///     Ok(())
/// }
/// ```
///
/// Because the body may run more than once (each retry), parameters it uses must
/// be `Copy` or cloned inside the body — the same constraint as the hand-written
/// closure form.
#[proc_macro_attribute]
pub fn transactional(attr: TokenStream, item: TokenStream) -> TokenStream {
    expand(attr, item, Wrapper::Transactional)
}

/// Wraps an `async fn` body in a follower read.
///
/// Like [`macro@transactional`], but runs the body `AS OF SYSTEM TIME`. Arguments
/// are named: `#[follower_read(staleness = std::time::Duration::from_secs(5))]`,
/// optionally with `pool = db`. With no `staleness` it uses
/// `follower_read_timestamp()`.
#[proc_macro_attribute]
pub fn follower_read(attr: TokenStream, item: TokenStream) -> TokenStream {
    expand(attr, item, Wrapper::FollowerRead)
}

enum Wrapper {
    Transactional,
    FollowerRead,
}

fn expand(attr: TokenStream, item: TokenStream, wrapper: Wrapper) -> TokenStream {
    let input = parse_macro_input!(item as ItemFn);
    let args = parse_macro_input!(attr as MacroArgs);

    let pool = args
        .pool
        .unwrap_or_else(|| Ident::new("pool", proc_macro2::Span::call_site()));
    let conn = Ident::new("conn", proc_macro2::Span::call_site());

    let vis = &input.vis;
    let sig = &input.sig;
    let attrs = &input.attrs;
    let block = &input.block;

    let call = match wrapper {
        Wrapper::Transactional => quote! {
            #pool.execute_tx(move |#conn| ::std::boxed::Box::pin(async move #block)).await
        },
        Wrapper::FollowerRead => {
            let staleness = args.staleness.map(|s| quote! { #s }).unwrap_or_else(|| {
                quote! { ::std::time::Duration::ZERO }
            });
            quote! {
                #pool
                    .follower_read_tx(#staleness, move |#conn| ::std::boxed::Box::pin(async move #block))
                    .await
            }
        }
    };

    quote! {
        #(#attrs)*
        #vis #sig {
            #call
        }
    }
    .into()
}

/// Parsed attribute arguments: `pool = ident`, `staleness = expr`, or a bare
/// first token that is the staleness expression (for `follower_read`).
struct MacroArgs {
    pool: Option<Ident>,
    staleness: Option<proc_macro2::TokenStream>,
}

impl syn::parse::Parse for MacroArgs {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let mut pool = None;
        let mut staleness = None;

        // Named, order-independent: `pool = x`, `staleness = y`.
        while !input.is_empty() {
            let key: Ident = input.parse()?;
            let _: syn::Token![=] = input.parse()?;
            match key.to_string().as_str() {
                "pool" => pool = Some(input.parse()?),
                "staleness" => {
                    let expr: syn::Expr = input.parse()?;
                    staleness = Some(quote! { #expr });
                }
                other => {
                    return Err(syn::Error::new(
                        key.span(),
                        format!("unknown argument `{other}` (expected `pool` or `staleness`)"),
                    ))
                }
            }
            if input.is_empty() {
                break;
            }
            let _: syn::Token![,] = input.parse()?;
        }

        Ok(Self { pool, staleness })
    }
}

/// Declares a SeaORM entity's multi-region locality, the analogue of Spring's
/// `@GlobalTable` / `@RegionalByRow` / `@CrdbRegion`.
///
/// Put it on the entity's `Model` struct alongside `DeriveEntityModel`; it generates
/// `impl cockroachdb_core::multiregion::CrdbLocality for Entity`, which the SeaORM
/// adapter's `verify_multiregion` check diffs against the live table. Requires a
/// `cockroachdb-core` dependency (the generated code names it by path).
///
/// Container attribute (exactly one required):
/// - `#[crdb(global)]` — the table should be `GLOBAL`.
/// - `#[crdb(regional_by_row)]` — the table should be `REGIONAL BY ROW`.
///
/// Field attribute (only with `regional_by_row`):
/// - `#[crdb(region)]` — marks the region column. The declared column name is the
///   field's name.
///
/// ```ignore
/// #[derive(Clone, Debug, PartialEq, DeriveEntityModel, CrdbLocality)]
/// #[sea_orm(table_name = "ideas")]
/// #[crdb(regional_by_row)]
/// pub struct Model {
///     #[sea_orm(primary_key)]
///     pub id: i64,
///     pub title: String,
///     #[crdb(region)]
///     pub region: String,
/// }
/// ```
#[proc_macro_derive(CrdbLocality, attributes(crdb))]
pub fn derive_crdb_locality(item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as DeriveInput);
    match expand_locality(&input) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

fn expand_locality(input: &DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
    let mut global = false;
    let mut regional_by_row = false;
    for attr in &input.attrs {
        if attr.path().is_ident("crdb") {
            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("global") {
                    global = true;
                    Ok(())
                } else if meta.path.is_ident("regional_by_row") {
                    regional_by_row = true;
                    Ok(())
                } else {
                    Err(meta.error("expected `global` or `regional_by_row`"))
                }
            })?;
        }
    }

    if global && regional_by_row {
        return Err(syn::Error::new_spanned(
            &input.ident,
            "a type cannot be both #[crdb(global)] and #[crdb(regional_by_row)]",
        ));
    }
    if !global && !regional_by_row {
        return Err(syn::Error::new_spanned(
            &input.ident,
            "#[derive(CrdbLocality)] needs #[crdb(global)] or #[crdb(regional_by_row)]",
        ));
    }

    let Data::Struct(data) = &input.data else {
        return Err(syn::Error::new_spanned(
            &input.ident,
            "#[derive(CrdbLocality)] can only be applied to a struct",
        ));
    };

    // ponytail: the region column is taken from the field name — SeaORM's default
    // column name. A `#[sea_orm(column_name = "...")]` override on the region field
    // is not read; rename the field to match if you need a different column.
    let mut region_column: Option<String> = None;
    if let Fields::Named(named) = &data.fields {
        for field in &named.named {
            let mut is_region = false;
            for attr in &field.attrs {
                if attr.path().is_ident("crdb") {
                    attr.parse_nested_meta(|meta| {
                        if meta.path.is_ident("region") {
                            is_region = true;
                            Ok(())
                        } else {
                            Err(meta.error("expected `region`"))
                        }
                    })?;
                }
            }
            if is_region {
                if !regional_by_row {
                    return Err(syn::Error::new_spanned(
                        field,
                        "#[crdb(region)] is only valid with #[crdb(regional_by_row)]",
                    ));
                }
                if region_column.is_some() {
                    return Err(syn::Error::new_spanned(
                        field,
                        "only one field may be #[crdb(region)]",
                    ));
                }
                region_column = Some(field.ident.as_ref().unwrap().to_string());
            }
        }
    }

    let body = if global {
        quote! { ::cockroachdb_core::multiregion::DeclaredLocality::Global }
    } else {
        let column = match region_column {
            Some(c) => quote! { ::core::option::Option::Some(::std::string::String::from(#c)) },
            None => quote! { ::core::option::Option::None },
        };
        quote! {
            ::cockroachdb_core::multiregion::DeclaredLocality::RegionalByRow { region_column: #column }
        }
    };

    Ok(quote! {
        #[automatically_derived]
        impl ::cockroachdb_core::multiregion::CrdbLocality for Entity {
            fn crdb_locality() -> ::cockroachdb_core::multiregion::DeclaredLocality {
                #body
            }
        }
    })
}
