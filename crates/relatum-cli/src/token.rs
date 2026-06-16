//! Persistence of the session token between CLI invocations.
//!
//! The token is a bearer credential, so it lives in the operating system's keyring
//! — the macOS Keychain, or the Secret Service (GNOME Keyring / KWallet) over D-Bus
//! on Linux — rather than a file. The OS, not file permissions, then guards the
//! secret at rest. `login`/`refresh` write it and `logout` removes it. A missing
//! entry is not an error: the first run, or one after logout, simply has no token to
//! resume.
//!
//! Headless environments (CI, a server, SSH without an unlocked login keyring) have
//! no Secret Service to talk to; those callers pass `--token` / `RELATUM_TOKEN`,
//! which bypasses this module entirely (see `main::run`). When no keyring is
//! reachable, `login`/`logout`/`refresh` fail with a message pointing there.

use anyhow::{Context, Result};
use keyring_core::{Entry, Error as KeyringError};

/// Service and account identifiers for the single, global session token.
///
/// `SERVICE` mirrors the old on-disk qualifier/org/app triple
/// (`ProjectDirs::from("org", "thehoster", "relatum")`) so it reads naturally and is
/// unlikely to collide with another app's keyring entries. Both are fixed (not keyed
/// by server URL), preserving the previous single-file semantics: a second `login`
/// overwrites the first.
const SERVICE: &str = "org.thehoster.relatum";
const ACCOUNT: &str = "session-token";

/// Install the per-platform default credential store, unless one is already set.
///
/// The "already set" short-circuit is also the seam tests use: they inject the
/// in-memory mock store first, and this check then leaves it in place. On platforms
/// without a supported backend we refuse rather than silently falling back to an
/// insecure file — the `--token` / `RELATUM_TOKEN` path covers those callers.
fn ensure_store() -> Result<()> {
    if keyring_core::get_default_store().is_some() {
        return Ok(());
    }

    #[cfg(target_os = "macos")]
    {
        let store = apple_native_keyring_store::keychain::Store::new()
            .context("opening the macOS Keychain")?;
        keyring_core::set_default_store(store);
        Ok(())
    }

    #[cfg(target_os = "linux")]
    {
        // Pure-Rust zbus + `crypto-rust` session encryption: no libdbus C, so it
        // links cleanly under cargo-zigbuild for the musl release targets. The
        // `crypto-rust`-only mode runs Secret Service calls on its own internal
        // executor, so invoking it synchronously from inside `#[tokio::main]` cannot
        // trip a nested-runtime panic. Connecting requires a running D-Bus session
        // and an unlocked Secret Service at runtime.
        let store = zbus_secret_service_keyring_store::Store::new().context(
            "connecting to the Secret Service keyring \
             (headless? pass --token or set RELATUM_TOKEN instead)",
        )?;
        keyring_core::set_default_store(store);
        Ok(())
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        anyhow::bail!(
            "no OS keyring backend is available on this platform; \
             pass --token or set RELATUM_TOKEN"
        )
    }
}

/// Build the keyring entry for `(service, account)` after the default store is set.
fn entry_for(service: &str, account: &str) -> Result<Entry> {
    ensure_store()?;
    Entry::new(service, account).context("building the keyring entry")
}

fn load_for(service: &str, account: &str) -> Result<Option<String>> {
    match entry_for(service, account)?.get_password() {
        Ok(password) => {
            let token = password.trim().to_string();
            Ok((!token.is_empty()).then_some(token))
        }
        Err(KeyringError::NoEntry) => Ok(None),
        Err(e) => Err(e).context("reading the session token from the keyring"),
    }
}

fn store_for(service: &str, account: &str, token: &str) -> Result<()> {
    entry_for(service, account)?
        .set_password(token)
        .context("storing the session token in the keyring")
}

fn clear_for(service: &str, account: &str) -> Result<()> {
    match entry_for(service, account)?.delete_credential() {
        Ok(()) | Err(KeyringError::NoEntry) => Ok(()),
        Err(e) => Err(e).context("removing the session token from the keyring"),
    }
}

/// The stored session token, or `None` if none has been saved.
pub fn load() -> Result<Option<String>> {
    load_for(SERVICE, ACCOUNT)
}

/// Save the session token, replacing any previously stored one.
pub fn store(token: &str) -> Result<()> {
    store_for(SERVICE, ACCOUNT, token)
}

/// Forget the saved token. Succeeds whether or not one was present.
pub fn clear() -> Result<()> {
    clear_for(SERVICE, ACCOUNT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Once;

    /// Install the in-memory mock store once for the whole test process. Because
    /// `ensure_store()` is a no-op when a default store already exists, production
    /// code never touches the real OS keyring during tests — so these pass even in a
    /// container with no Secret Service daemon.
    fn init_mock() {
        static MOCK: Once = Once::new();
        MOCK.call_once(|| {
            if keyring_core::get_default_store().is_none() {
                keyring_core::set_default_store(keyring_core::mock::Store::new().unwrap());
            }
        });
    }

    /// A unique `(service, account)` per test: the mock store is process-global and
    /// shared across the parallel test threads, so distinct ids keep entries from
    /// aliasing between tests.
    fn ids(tag: &str) -> (String, String) {
        (format!("relatum-test-{tag}"), "session-token".to_string())
    }

    #[test]
    fn store_then_load_round_trips() {
        init_mock();
        let (service, account) = ids("round-trip");
        store_for(&service, &account, "tok-secret").unwrap();
        assert_eq!(
            load_for(&service, &account).unwrap().as_deref(),
            Some("tok-secret")
        );
    }

    #[test]
    fn load_when_absent_is_none() {
        init_mock();
        let (service, account) = ids("absent");
        assert_eq!(load_for(&service, &account).unwrap(), None);
    }

    #[test]
    fn clear_is_idempotent() {
        init_mock();
        let (service, account) = ids("clear-idempotent");
        // Clearing with nothing stored is a no-op success.
        clear_for(&service, &account).unwrap();
        store_for(&service, &account, "tok").unwrap();
        clear_for(&service, &account).unwrap();
        // A second clear still succeeds, and the token is gone.
        clear_for(&service, &account).unwrap();
        assert_eq!(load_for(&service, &account).unwrap(), None);
    }

    #[test]
    fn store_overwrites_previous() {
        init_mock();
        let (service, account) = ids("overwrite");
        store_for(&service, &account, "first").unwrap();
        store_for(&service, &account, "second").unwrap();
        assert_eq!(
            load_for(&service, &account).unwrap().as_deref(),
            Some("second")
        );
    }

    #[test]
    fn load_trims_and_treats_blank_as_none() {
        init_mock();
        let (service, account) = ids("blank");
        store_for(&service, &account, "   ").unwrap();
        assert_eq!(load_for(&service, &account).unwrap(), None);
        store_for(&service, &account, "  padded  ").unwrap();
        assert_eq!(
            load_for(&service, &account).unwrap().as_deref(),
            Some("padded")
        );
    }
}
