//! Session + SSO-state cookies.
//!
//! The session token is kept in an **http-only** cookie so client-side script can
//! never read it (the reason this SSR frontend is safer than a token-in-`localStorage`
//! SPA). It is `SameSite=Lax`, which — together with every mutation being a `POST` —
//! means a cross-site form cannot ride the user's session (CSRF mitigation). The
//! short-lived `sso_state` cookie carries the login-flow nonce between `/auth/sso` and
//! `/auth/callback`.

use axum_extra::extract::CookieJar;
use axum_extra::extract::cookie::{Cookie, SameSite};
use time::Duration;

/// Name of the cookie holding the API session bearer token.
pub const SESSION_COOKIE: &str = "relatum_session";
/// Name of the cookie holding the pending SSO login nonce.
pub const SSO_STATE_COOKIE: &str = "relatum_sso_state";
/// Name of the cookie holding the chosen colour theme (`light` / `dark` / `auto`).
pub const THEME_COOKIE: &str = "relatum_theme";

/// The session token currently held, if any (an empty cookie counts as none).
pub fn token(jar: &CookieJar) -> Option<String> {
    jar.get(SESSION_COOKIE)
        .map(|c| c.value().to_owned())
        .filter(|t| !t.is_empty())
}

/// Build the session cookie carrying `token`. A session cookie (no `Max-Age`) so it
/// is dropped when the browser closes; the server-side token has its own TTL.
pub fn session_cookie(token: String, secure: bool) -> Cookie<'static> {
    base_cookie(SESSION_COOKIE, token, secure)
}

/// Build the short-lived cookie holding the SSO login nonce.
pub fn sso_state_cookie(nonce: String, secure: bool) -> Cookie<'static> {
    base_cookie(SSO_STATE_COOKIE, nonce, secure)
}

/// Build the colour-theme cookie. Unlike the session cookie this is a *persistent*
/// cookie (a long `Max-Age`) so the choice survives a browser restart; it holds only a
/// non-secret display preference.
pub fn theme_cookie(value: String, secure: bool) -> Cookie<'static> {
    let mut cookie = base_cookie(THEME_COOKIE, value, secure);
    cookie.set_max_age(Duration::weeks(52));
    cookie
}

/// A removal cookie: handed to [`CookieJar::remove`] it expires the named cookie on
/// the matching path.
pub fn removal_cookie(name: &'static str) -> Cookie<'static> {
    let mut cookie = Cookie::new(name, "");
    cookie.set_path("/");
    cookie
}

fn base_cookie(name: &'static str, value: String, secure: bool) -> Cookie<'static> {
    let mut cookie = Cookie::new(name, value);
    cookie.set_http_only(true);
    cookie.set_same_site(SameSite::Lax);
    cookie.set_path("/");
    cookie.set_secure(secure);
    cookie
}
