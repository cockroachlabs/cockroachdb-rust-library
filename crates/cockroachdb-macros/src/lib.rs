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
use syn::{parse_macro_input, Ident, ItemFn};

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
