//! Authentication use-cases.
//!
//! [`Authenticator`] is the concrete struct behind login/logout/refresh.
//! Authentication is SSO-only: the instance stores no passwords. It consumes four
//! ports: [`SSOProvider`](crate::ports::sso_connector::SSOProvider) to validate a
//! token and attest which user it belongs to,
//! [`UserStorage`](crate::ports::userstorage::UserStorage) to confirm that user
//! exists locally (the periodic LDAP
//! [sync](crate::services::sync::DirectorySync) provisions them — login never
//! does), [`IdGenerator`](crate::ports::ids::IdGenerator) to mint opaque token
//! values, and [`SessionRepository`](crate::ports::session::SessionRepository) to
//! persist and revoke sessions.

use crate::errors::DomainError;
use crate::models::auth::{Credentials, SessionToken};
use crate::models::ids::UserId;
use crate::models::users::User;
use crate::ports::ids::IdGenerator;
use crate::ports::session::SessionRepository;
use crate::ports::sso_connector::{SSOProvider, SsoCompletion, SsoMetadata};
use crate::ports::userstorage::UserStorage;

/// Validates SSO tokens and manages session tokens.
#[derive(Debug, Clone)]
pub struct Authenticator<U, S, I, P> {
    users: U,
    sessions: S,
    ids: I,
    sso: P,
}

impl<U, S, I, P> Authenticator<U, S, I, P>
where
    U: UserStorage,
    S: SessionRepository,
    I: IdGenerator,
    P: SSOProvider,
{
    /// Wire the authenticator to its user-storage, session, id-generation, and
    /// single-sign-on ports.
    pub fn new(users: U, sessions: S, ids: I, sso: P) -> Self {
        Self {
            users,
            sessions,
            ids,
            sso,
        }
    }

    /// Validate an SSO token and issue a new session token.
    ///
    /// The presented token is checked against the [`SSOProvider`], which attests
    /// the user it belongs to. That user must already exist locally — the directory
    /// sync owns provisioning, so login never creates a user; an attested subject
    /// we have never synced is rejected with [`DomainError::Unauthorized`]. An
    /// invalid token is rejected the same way, so a caller cannot tell the two
    /// apart.
    pub async fn login(&self, creds: Credentials) -> Result<SessionToken, DomainError> {
        let invalid = || DomainError::Unauthorized("invalid or unrecognised SSO login".into());

        let identity = self
            .sso
            .check_token(&creds.token)
            .await?
            .ok_or_else(invalid)?;
        // The provider authenticates; the user must have been provisioned by sync.
        if self.users.lookup(&identity.id).await?.is_none() {
            return Err(invalid());
        }
        self.issue_token(identity.id).await
    }

    /// Advertise whether SSO login is available and where the browser flow starts.
    pub fn sso_metadata(&self) -> SsoMetadata {
        self.sso.metadata()
    }

    /// Begin the browser SSO login, returning the IdP authorization URL the browser
    /// should be sent to. `app_redirect` is the client's loopback/callback URL the
    /// flow will ultimately return to.
    pub async fn sso_begin(&self, app_redirect: &str) -> Result<String, DomainError> {
        self.sso.begin(app_redirect).await
    }

    /// Complete the browser SSO login: exchange the authorization `code` for an
    /// access token, stash that token under a **single-use handoff code**, and hand
    /// back the handoff code together with the app loopback URL to redirect the
    /// browser to.
    ///
    /// The relatum session is deliberately *not* minted here — it is minted later, at
    /// [`sso_exchange`](Self::sso_exchange), when the app redeems the handoff code
    /// back-channel. That way neither the access token nor a session token ever rides
    /// in the browser redirect URL; only the opaque, short-lived, single-use handoff
    /// code does.
    pub async fn sso_complete(
        &self,
        code: &str,
        state: &str,
    ) -> Result<(String, String), DomainError> {
        let SsoCompletion {
            access_token,
            app_redirect,
        } = self.sso.complete(code, state).await?;
        let handoff = self.sso.stash_handoff(access_token).await?;
        Ok((handoff, app_redirect))
    }

    /// Redeem a single-use SSO handoff code (from the browser redirect) for a session
    /// token, back-channel.
    ///
    /// Consumes the stashed access token and runs the same [`login`](Self::login)
    /// path (so the attested user must already be synced). This is what lets a browser
    /// frontend obtain its session without the token ever appearing in a URL. An
    /// unknown/used/expired code surfaces as [`DomainError::Unauthorized`].
    pub async fn sso_exchange(&self, handoff: &str) -> Result<SessionToken, DomainError> {
        let access_token = self.sso.redeem_handoff(handoff).await?;
        self.login(Credentials {
            token: access_token,
        })
        .await
    }

    /// Invalidate the given session token. Revoking an unknown token is a no-op.
    pub async fn logout(&self, token: &str) -> Result<(), DomainError> {
        self.sessions.revoke(token).await
    }

    /// Extend the lifetime of a valid session token, returning a fresh one for the
    /// same user and revoking the old.
    pub async fn refresh(&self, token: &str) -> Result<SessionToken, DomainError> {
        let session = self
            .sessions
            .lookup(token)
            .await?
            .ok_or_else(|| DomainError::Unauthorized("unknown or expired session".into()))?;
        let fresh = self.issue_token(session.subject).await?;
        self.sessions.revoke(token).await?;
        Ok(fresh)
    }

    /// Resolve a presented session token to the user it authenticates.
    ///
    /// Looks the token up, then loads its subject. An unknown or expired token —
    /// and a token whose user no longer exists — is rejected with
    /// [`DomainError::Unauthorized`]. This is the entry point the transport layer
    /// uses to turn a bearer token into the acting user.
    pub async fn authenticate(&self, token: &str) -> Result<User, DomainError> {
        let unknown = || DomainError::Unauthorized("unknown or expired session".into());
        let session = self.sessions.lookup(token).await?.ok_or_else(unknown)?;
        self.users
            .lookup(&session.subject)
            .await?
            .ok_or_else(unknown)
    }

    /// Mint a fresh token value bound to `subject` and persist it.
    async fn issue_token(&self, subject: UserId) -> Result<SessionToken, DomainError> {
        let token = SessionToken {
            value: self.ids.session_token(),
            subject,
        };
        self.sessions.store(&token).await?;
        Ok(token)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ids::{DepartmentId, UserId};
    use crate::models::users::{DirectoryMarker, User};
    use crate::ports::session::SessionRepository;
    use crate::ports::sso_connector::SsoIdentity;
    use crate::ports::userstorage::UserStorage;
    use crate::testing::{InMemorySessions, InMemoryUsers, MockSSO, SeqIds, block_on};

    type Auth = Authenticator<InMemoryUsers, InMemorySessions, SeqIds, MockSSO>;

    fn auth() -> Auth {
        Authenticator::new(
            InMemoryUsers::default(),
            InMemorySessions::default(),
            SeqIds::default(),
            MockSSO::default(),
        )
    }

    fn creds(token: &str) -> Credentials {
        Credentials {
            token: token.to_owned(),
        }
    }

    /// Seed a synced user the provider can later attest.
    fn seed_user(auth: &Auth, id: &str) {
        let stored = User::new(
            UserId::new(id),
            id,
            DirectoryMarker::Instructor,
            Some(DepartmentId::new("blue")),
        );
        block_on(auth.users.store(stored)).unwrap();
    }

    #[test]
    fn login_succeeds_for_a_synced_user_and_persists_the_token() {
        let auth = auth();
        seed_user(&auth, "alice");
        auth.sso.register(
            "good-token",
            SsoIdentity {
                id: UserId::new("alice"),
            },
        );

        let token = block_on(auth.login(creds("good-token"))).unwrap();

        assert!(
            block_on(auth.sessions.lookup(&token.value))
                .unwrap()
                .is_some()
        );
        assert_eq!(token.subject, UserId::new("alice"));
    }

    #[test]
    fn login_rejects_an_invalid_token() {
        let auth = auth();
        seed_user(&auth, "alice");

        let err = block_on(auth.login(creds("forged"))).unwrap_err();
        assert!(matches!(err, DomainError::Unauthorized(_)));
    }

    #[test]
    fn login_rejects_an_attested_user_that_was_never_synced() {
        let auth = auth();
        // The provider vouches for "carol", but she was never synced locally.
        auth.sso.register(
            "good-token",
            SsoIdentity {
                id: UserId::new("carol"),
            },
        );

        let err = block_on(auth.login(creds("good-token"))).unwrap_err();
        assert!(matches!(err, DomainError::Unauthorized(_)));
        assert!(
            block_on(auth.users.lookup(&UserId::new("carol")))
                .unwrap()
                .is_none(),
            "login must never provision a user"
        );
    }

    #[test]
    fn refresh_rotates_the_token() {
        let auth = auth();
        seed_user(&auth, "alice");
        auth.sso.register(
            "good-token",
            SsoIdentity {
                id: UserId::new("alice"),
            },
        );
        let first = block_on(auth.login(creds("good-token"))).unwrap();

        let second = block_on(auth.refresh(&first.value)).unwrap();

        assert_ne!(first.value, second.value);
        assert!(
            block_on(auth.sessions.lookup(&first.value))
                .unwrap()
                .is_none()
        );
        assert!(
            block_on(auth.sessions.lookup(&second.value))
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn refresh_rejects_an_unknown_token() {
        let err = block_on(auth().refresh("not-a-token")).unwrap_err();
        assert!(matches!(err, DomainError::Unauthorized(_)));
    }

    #[test]
    fn sso_exchange_redeems_a_single_use_handoff_for_a_session() {
        let auth = auth();
        seed_user(&auth, "alice");
        auth.sso.register(
            "access-tok",
            SsoIdentity {
                id: UserId::new("alice"),
            },
        );
        // Stand in for `sso_complete`: stash the obtained access token under a code.
        let code = block_on(auth.sso.stash_handoff("access-tok".to_owned())).unwrap();

        let token = block_on(auth.sso_exchange(&code)).unwrap();
        assert_eq!(token.subject, UserId::new("alice"));
        assert!(
            block_on(auth.sessions.lookup(&token.value))
                .unwrap()
                .is_some()
        );

        // The handoff is single-use: a second redemption fails.
        let err = block_on(auth.sso_exchange(&code)).unwrap_err();
        assert!(matches!(err, DomainError::Unauthorized(_)));
    }

    #[test]
    fn sso_exchange_rejects_an_unknown_handoff() {
        let err = block_on(auth().sso_exchange("never-issued")).unwrap_err();
        assert!(matches!(err, DomainError::Unauthorized(_)));
    }

    #[test]
    fn logout_revokes_the_token() {
        let auth = auth();
        seed_user(&auth, "alice");
        auth.sso.register(
            "good-token",
            SsoIdentity {
                id: UserId::new("alice"),
            },
        );
        let token = block_on(auth.login(creds("good-token"))).unwrap();

        block_on(auth.logout(&token.value)).unwrap();

        assert!(
            block_on(auth.sessions.lookup(&token.value))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn authenticate_resolves_a_token_to_its_user() {
        let auth = auth();
        seed_user(&auth, "alice");
        auth.sso.register(
            "good-token",
            SsoIdentity {
                id: UserId::new("alice"),
            },
        );
        let token = block_on(auth.login(creds("good-token"))).unwrap();

        let user = block_on(auth.authenticate(&token.value)).unwrap();
        assert_eq!(*user.id(), UserId::new("alice"));
    }
}
