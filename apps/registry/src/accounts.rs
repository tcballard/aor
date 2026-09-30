use crate::{State, json};
use aor_http::Response;
use aor_router::{AppError, Context, Route, auth_error};
use serde::Deserialize;
use std::{
    io::Write,
    os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt},
    sync::Arc,
};
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Credentials {
    email: String,
    password: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Address {
    email: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Token {
    token: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Reset {
    token: String,
    password: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Api {
    scopes: Vec<String>,
    lifetime_seconds: i64,
}
async fn deliver(state: &State, delivery: Option<aor_session::Delivery>) -> Result<(), AppError> {
    let Some(delivery) = delivery else {
        return Ok(());
    };
    let dir = state.mail_spool.clone().ok_or(AppError::Internal)?;
    tokio::task::spawn_blocking(move||->std::io::Result<()>{
 if !dir.exists(){std::fs::DirBuilder::new().mode(0o700).create(&dir)?;}
 let meta=std::fs::symlink_metadata(&dir)?;if !meta.is_dir()||meta.permissions().mode()&0o077!=0{return Err(std::io::Error::other("mail spool must be a private directory"))}
 let file=dir.join(format!("{}.json",aor_db::Uuid::new_v4()));let mut out=std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(file)?;
 let bytes=serde_json::to_vec(&serde_json::json!({"to":delivery.email,"purpose":delivery.purpose,"token":delivery.token.expose()}))?;out.write_all(&bytes)?;out.sync_all()
 }).await.map_err(|_|AppError::Internal)?.map_err(|_|AppError::Internal)
}
async fn endpoint(state: &State, ctx: Context, action: &str) -> Result<Response, AppError> {
    // Anonymous account POSTs also reject cross-origin requests (including login CSRF).
    state
        .auth
        .check_origin(&ctx.request.headers)
        .map_err(auth_error)?;
    match action {
        "register" => {
            let input: Credentials = ctx.json().await?;
            let delivery = state
                .auth
                .register(&input.email, input.password)
                .await
                .map_err(auth_error)?;
            deliver(state, delivery).await?;
            json(
                202,
                serde_json::json!({"status":"Check your email if this account can be registered."}),
            )
        }
        "login" => {
            let cookies = aor_session::parse_cookies(
                aor_session::unique_header(&ctx.request.headers, "cookie")
                    .map_err(auth_error)?
                    .unwrap_or(""),
            )
            .map_err(auth_error)?;
            let input: Credentials = ctx.json().await?;
            let login = state
                .auth
                .login(
                    &input.email,
                    input.password,
                    cookies
                        .get(state.auth.config().cookie_name())
                        .map(String::as_str),
                )
                .await
                .map_err(auth_error)?;
            let mut response = json(200, serde_json::json!({"status":"signed_in"}))?;
            for cookie in login.cookies(state.auth.config()) {
                response = response
                    .header("Set-Cookie", &cookie)
                    .map_err(|_| AppError::Internal)?;
            }
            Ok(response)
        }
        "verify" => {
            let input: Token = ctx.json().await?;
            state
                .auth
                .verify_email(&input.token)
                .await
                .map_err(auth_error)?;
            json(200, serde_json::json!({"status":"verified"}))
        }
        "request-reset" => {
            let input: Address = ctx.json().await?;
            let delivery = state
                .auth
                .request_reset(&input.email)
                .await
                .map_err(auth_error)?;
            deliver(state, delivery).await?;
            json(
                202,
                serde_json::json!({"status":"Check your email if this account exists."}),
            )
        }
        "reset" => {
            let input: Reset = ctx.json().await?;
            state
                .auth
                .reset_password(&input.token, input.password)
                .await
                .map_err(auth_error)?;
            json(200, serde_json::json!({"status":"password_reset"}))
        }
        "logout" | "revoke-all" => {
            let session = ctx.session().ok_or(AppError::Unauthenticated)?;
            if action == "logout" {
                state.auth.logout(session).await.map_err(auth_error)?;
            } else {
                state
                    .auth
                    .revoke_all(ctx.principal().ok_or(AppError::Unauthenticated)?)
                    .await
                    .map_err(auth_error)?;
            }
            let mut response = json(200, serde_json::json!({"status":"signed_out"}))?;
            for cookie in state.auth.clear_cookies() {
                response = response
                    .header("Set-Cookie", &cookie)
                    .map_err(|_| AppError::Internal)?;
            }
            Ok(response)
        }
        "tokens" => {
            let principal = ctx.principal().ok_or(AppError::Unauthenticated)?.clone();
            let input: Api = ctx.json().await?;
            let token = state
                .auth
                .create_api_token(&principal, input.scopes, input.lifetime_seconds)
                .await
                .map_err(auth_error)?;
            json(201, serde_json::json!({"token":token.expose()}))
        }
        "revoke-token" => {
            let principal = ctx.principal().ok_or(AppError::Unauthenticated)?.clone();
            let input: Token = ctx.json().await?;
            state
                .auth
                .revoke_api_token(&principal, &input.token)
                .await
                .map_err(auth_error)?;
            json(200, serde_json::json!({"status":"revoked"}))
        }
        _ => Err(AppError::NotFound),
    }
}
pub fn routes(state: Arc<State>) -> Vec<Route> {
    let mut routes = vec![Route::public("GET", "/", "registry_home", |_| async {
        Response::new(200,b"<!doctype html><html lang=\"en\"><meta charset=\"utf-8\"><title>AoR Plugin Registry</title><h1>AoR Plugin Registry</h1><p>Development accounts and owner-scoped plugin API. Not cleared for public traffic.</p><p>See apps/registry/README.md for registration, verification and API usage.</p></html>".to_vec()).header("Content-Type","text/html; charset=utf-8").map_err(|_|AppError::Internal)
    })];
    for action in [
        "register",
        "login",
        "verify",
        "request-reset",
        "reset",
        "logout",
        "revoke-all",
        "tokens",
        "revoke-token",
    ] {
        let state = state.clone();
        let handler = move |ctx| {
            let state = state.clone();
            async move { endpoint(&state, ctx, action).await }
        };
        let path = format!("/accounts/{action}");
        let route = if ["logout", "revoke-all", "tokens", "revoke-token"].contains(&action) {
            Route::protected("POST", &path, action, "account_session", handler)
        } else {
            Route::public("POST", &path, action, handler)
        };
        routes.push(route);
    }
    routes
}
