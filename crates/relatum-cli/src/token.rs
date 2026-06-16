//! Persistence of the session token between CLI invocations.
//!
//! The token lives in a single file under the user's XDG state directory (falling
//! back to the data directory on platforms without one), written by `login`/`refresh`
//! and removed by `logout`. A missing file is not an error — the first run, or one
//! after logout, simply has no token to resume.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use directories::ProjectDirs;

fn token_path() -> Result<PathBuf> {
    let dirs = ProjectDirs::from("org", "thehoster", "relatum")
        .context("could not determine a directory to store the session token")?;
    let base = dirs.state_dir().unwrap_or_else(|| dirs.data_dir());
    Ok(base.join("token"))
}

/// The stored session token, or `None` if none has been saved.
pub fn load() -> Result<Option<String>> {
    let path = token_path()?;
    match fs::read_to_string(&path) {
        Ok(contents) => {
            let token = contents.trim().to_string();
            Ok((!token.is_empty()).then_some(token))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("reading token from {}", path.display())),
    }
}

/// Save the session token, creating the parent directory as needed.
///
/// The token is a bearer credential, so on Unix the file is created `0o600` and its
/// directory tightened to `0o700` — a default-umask `0644` file would otherwise let
/// any other local user read the token and impersonate this user against the API.
pub fn store(token: &str) -> Result<()> {
    let path = token_path()?;
    if let Some(parent) = path.parent() {
        create_dir_private(parent).with_context(|| format!("creating {}", parent.display()))?;
        restrict_dir(parent)?;
    }
    write_private(&path, token).with_context(|| format!("writing token to {}", path.display()))
}

/// A sibling temp path for the atomic write, namespaced by the current process so
/// concurrent CLI invocations don't pick the same scratch file.
fn tmp_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".tmp.{}", std::process::id()));
    path.with_file_name(name)
}

/// Atomically write `contents` to `path` as an owner-only (`0o600`) file on Unix.
///
/// Writes to a sibling temp file created `0o600` and renames it over `path`, so a
/// failure partway through can never truncate or empty an existing token — the
/// original stays intact until the rename succeeds, and a replaced file is fully
/// superseded by the owner-only one. A default-umask `0o644` file would otherwise let
/// any other local user read this bearer credential and impersonate the user.
#[cfg(unix)]
fn write_private(path: &Path, contents: &str) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

    let tmp = tmp_path(path);
    let open = || {
        fs::OpenOptions::new()
            .write(true)
            .create_new(true) // O_CREAT | O_EXCL: never follow/clobber an existing temp
            .mode(0o600)
            .open(&tmp)
    };
    // The only `AlreadyExists` we tolerate is a leftover temp from a crashed run.
    let mut file = match open() {
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            fs::remove_file(&tmp)?;
            open()?
        }
        other => other?,
    };
    // `mode` is masked by the umask on creation; set it explicitly so the file is
    // owner-only regardless of the caller's umask.
    file.set_permissions(fs::Permissions::from_mode(0o600))?;

    let written = file
        .write_all(contents.as_bytes())
        .and_then(|()| file.sync_all())
        .and_then(|()| fs::rename(&tmp, path));
    if written.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    written
}

/// On non-Unix platforms there is no portable mode to set, but the same temp+rename
/// keeps the write atomic so a partial failure cannot leave an empty token file.
#[cfg(not(unix))]
fn write_private(path: &Path, contents: &str) -> std::io::Result<()> {
    let tmp = tmp_path(path);
    let written = fs::write(&tmp, contents).and_then(|()| fs::rename(&tmp, path));
    if written.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    written
}

/// Create `dir` (and any missing parents) restricted to the owner (`0o700`) on Unix.
///
/// Building the directory at `0o700` in one step closes the window a
/// `create_dir_all`-then-`chmod` sequence would leave, during which the directory
/// briefly exists with the umask default (`0o755`) and another local user could plant
/// files in it (a TOCTOU race). A no-op mode elsewhere.
#[cfg(unix)]
fn create_dir_private(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
}

#[cfg(not(unix))]
fn create_dir_private(dir: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dir)
}

/// Tighten the token directory to owner-only (`0o700`) on Unix. The path is this
/// app's own state directory, so restricting it is safe; a no-op elsewhere.
#[cfg(unix)]
fn restrict_dir(dir: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(dir, fs::Permissions::from_mode(0o700))
        .with_context(|| format!("restricting permissions on {}", dir.display()))
}

#[cfg(not(unix))]
fn restrict_dir(_dir: &Path) -> Result<()> {
    Ok(())
}

/// Forget the saved token. Succeeds whether or not one was present.
pub fn clear() -> Result<()> {
    let path = token_path()?;
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e).with_context(|| format!("removing {}", path.display())),
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// A fresh scratch directory named after `tag`, so tests running in parallel
    /// within the same process (and thus sharing a pid) don't collide.
    fn scratch(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("relatum-token-test-{}-{tag}", std::process::id()));
        fs::remove_dir_all(&dir).ok();
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn write_private_makes_the_token_owner_only() {
        let dir = scratch("write-private");
        let path = dir.join("token");
        // Pre-create with loose perms to prove an existing file is replaced by an
        // owner-only one, not merely reused.
        fs::write(&path, "old").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();

        write_private(&path, "tok-secret").unwrap();

        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "token file must be readable only by its owner");
        assert_eq!(fs::read_to_string(&path).unwrap(), "tok-secret");
        // The atomic write must rename the temp away, never leave it behind.
        assert!(
            !tmp_path(&path).exists(),
            "temp file must be renamed/cleaned up"
        );

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn restrict_dir_tightens_the_directory_to_owner_only() {
        let dir = scratch("restrict-dir");
        let inner = dir.join("state");
        fs::create_dir_all(&inner).unwrap();
        fs::set_permissions(&inner, fs::Permissions::from_mode(0o755)).unwrap();

        restrict_dir(&inner).unwrap();

        let mode = fs::metadata(&inner).unwrap().permissions().mode() & 0o777;
        assert_eq!(
            mode, 0o700,
            "token directory must be accessible only by its owner"
        );

        fs::remove_dir_all(&dir).ok();
    }
}
