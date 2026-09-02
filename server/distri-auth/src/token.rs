//! Single-tenant deployment tokens for the standalone server.
//!
//! The cloud authenticates people: users, workspaces, roles. This does not.
//! A standalone deployment has exactly one identity — the deployment itself —
//! so a token here says only "the holder was issued a token by a caller who
//! knew this deployment's secret, and it has not expired yet".
//!
//! Tokens are stateless: an HMAC-SHA256 tag over the claims, keyed by the
//! deployment secret. Nothing is stored, so tokens survive a restart and
//! there is no table to prune. The cost is no revocation before expiry —
//! rotate the secret to invalidate every outstanding token at once.
//!
//! Wire shape is the shared [`distri_types::TokenResponse`], the same
//! contract the cloud serves and `Distri::issue_token()` already parses.

use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use base64::Engine;
use chrono::Utc;
use hmac::{Hmac, KeyInit, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

/// Minimum length for a usable deployment secret. Short secrets are brute
/// forceable offline against any token the holder has seen.
pub const MIN_SECRET_LEN: usize = 16;

/// Default lifetime of an access token.
pub const DEFAULT_ACCESS_TTL_SECS: i64 = 60 * 60;
/// Default lifetime of a refresh token.
pub const DEFAULT_REFRESH_TTL_SECS: i64 = 60 * 60 * 24 * 30;

/// Which of the two token kinds a tag covers. A refresh token is not
/// accepted on the API surface, and an access token cannot buy a new pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TokenKind {
    Access,
    Refresh,
}

/// What a deployment token carries. No user, no workspace — see the module
/// docs: the deployment is the only identity there is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenClaims {
    /// Which kind of token this is.
    pub kind: TokenKind,
    /// Issued-at, unix seconds.
    pub iat: i64,
    /// Expiry, unix seconds.
    pub exp: i64,
    /// Random per-token id, so two tokens minted in the same second differ.
    pub jti: String,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TokenError {
    #[error("token is malformed")]
    Malformed,
    #[error("token signature does not verify")]
    BadSignature,
    #[error("token expired at {expired_at}")]
    Expired { expired_at: i64 },
    #[error("expected a {expected:?} token, got a {actual:?} token")]
    WrongKind {
        expected: TokenKind,
        actual: TokenKind,
    },
}

/// Mints and verifies this deployment's tokens.
#[derive(Clone)]
pub struct TokenAuth {
    secret: Vec<u8>,
    access_ttl_secs: i64,
    refresh_ttl_secs: i64,
}

impl std::fmt::Debug for TokenAuth {
    /// Never print the secret — this type ends up inside server config that
    /// gets logged on startup.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TokenAuth")
            .field("secret", &"<redacted>")
            .field("access_ttl_secs", &self.access_ttl_secs)
            .field("refresh_ttl_secs", &self.refresh_ttl_secs)
            .finish()
    }
}

impl TokenAuth {
    /// Build from the deployment secret. Rejects a secret too short to be
    /// worth having — turning auth on with a weak secret is worse than
    /// leaving it off, because it reads as protection.
    pub fn new(secret: &str, access_ttl_secs: i64, refresh_ttl_secs: i64) -> Result<Self, String> {
        let trimmed = secret.trim();
        if trimmed.len() < MIN_SECRET_LEN {
            return Err(format!(
                "deployment secret must be at least {MIN_SECRET_LEN} characters, got {}",
                trimmed.len()
            ));
        }
        if access_ttl_secs <= 0 || refresh_ttl_secs <= 0 {
            return Err("token lifetimes must be positive".to_string());
        }
        Ok(Self {
            secret: trimmed.as_bytes().to_vec(),
            access_ttl_secs,
            refresh_ttl_secs,
        })
    }

    /// True when `candidate` is this deployment's secret. Constant-time, so
    /// a wrong guess leaks nothing through timing.
    pub fn is_deployment_secret(&self, candidate: &str) -> bool {
        use subtle::ConstantTimeEq;
        let a = candidate.trim().as_bytes();
        if a.len() != self.secret.len() {
            return false;
        }
        a.ct_eq(&self.secret).into()
    }

    /// Mint a fresh access + refresh pair.
    pub fn mint_pair(&self) -> distri_types::TokenResponse {
        self.mint_pair_at(Utc::now().timestamp())
    }

    /// Mint a pair as of `now` (unix seconds). Split out so tests can pin time.
    pub fn mint_pair_at(&self, now: i64) -> distri_types::TokenResponse {
        let access_exp = now + self.access_ttl_secs;
        distri_types::TokenResponse {
            access_token: self.mint(TokenKind::Access, now, access_exp),
            refresh_token: self.mint(TokenKind::Refresh, now, now + self.refresh_ttl_secs),
            expires_at: access_exp,
            identifier_id: None,
            limits: None,
        }
    }

    /// Verify a token, requiring it to be of `expected` kind and unexpired.
    pub fn verify(&self, token: &str, expected: TokenKind) -> Result<TokenClaims, TokenError> {
        self.verify_at(token, expected, Utc::now().timestamp())
    }

    /// Verify as of `now` (unix seconds). Split out so tests can pin time.
    pub fn verify_at(
        &self,
        token: &str,
        expected: TokenKind,
        now: i64,
    ) -> Result<TokenClaims, TokenError> {
        let (payload_b64, sig_b64) = token.split_once('.').ok_or(TokenError::Malformed)?;
        let sig = B64.decode(sig_b64).map_err(|_| TokenError::Malformed)?;

        self.tag(payload_b64.as_bytes())
            .verify_slice(&sig)
            .map_err(|_| TokenError::BadSignature)?;

        // Only decode the claims once the tag proves we wrote them.
        let payload = B64.decode(payload_b64).map_err(|_| TokenError::Malformed)?;
        let claims: TokenClaims =
            serde_json::from_slice(&payload).map_err(|_| TokenError::Malformed)?;

        if claims.kind != expected {
            return Err(TokenError::WrongKind {
                expected,
                actual: claims.kind,
            });
        }
        if claims.exp <= now {
            return Err(TokenError::Expired {
                expired_at: claims.exp,
            });
        }
        Ok(claims)
    }

    /// Exchange a valid refresh token for a fresh pair. The deployment
    /// secret is not needed here — that is the point of a refresh token.
    pub fn refresh(&self, refresh_token: &str) -> Result<distri_types::TokenResponse, TokenError> {
        self.refresh_at(refresh_token, Utc::now().timestamp())
    }

    /// Refresh as of `now` (unix seconds). Split out so tests can pin time.
    pub fn refresh_at(
        &self,
        refresh_token: &str,
        now: i64,
    ) -> Result<distri_types::TokenResponse, TokenError> {
        self.verify_at(refresh_token, TokenKind::Refresh, now)?;
        Ok(self.mint_pair_at(now))
    }

    fn mint(&self, kind: TokenKind, iat: i64, exp: i64) -> String {
        let claims = TokenClaims {
            kind,
            iat,
            exp,
            jti: uuid::Uuid::new_v4().to_string(),
        };
        // TokenClaims is a plain struct of owned scalars — serialization
        // cannot fail, and a token is not a place to propagate an error.
        let payload = B64.encode(serde_json::to_vec(&claims).unwrap_or_default());
        let tag = self.tag(payload.as_bytes()).finalize().into_bytes();
        format!("{payload}.{}", B64.encode(tag))
    }

    fn tag(&self, payload: &[u8]) -> Hmac<Sha256> {
        // Key length is unrestricted for HMAC, so this cannot fail.
        let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(&self.secret)
            .expect("HMAC accepts a key of any length");
        mac.update(payload);
        mac
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "deployment-secret-long-enough";
    const OTHER: &str = "a-different-deployment-secret";
    const NOW: i64 = 1_700_000_000;

    fn auth(secret: &str) -> TokenAuth {
        TokenAuth::new(secret, 3600, 86_400).expect("valid secret")
    }

    #[test]
    fn a_freshly_minted_access_token_verifies() {
        let auth = auth(SECRET);
        let pair = auth.mint_pair_at(NOW);
        let claims = auth
            .verify_at(&pair.access_token, TokenKind::Access, NOW + 1)
            .expect("fresh token verifies");
        assert_eq!(claims.kind, TokenKind::Access);
        assert_eq!(pair.expires_at, NOW + 3600);
    }

    #[test]
    fn an_expired_access_token_is_refused() {
        let auth = auth(SECRET);
        let pair = auth.mint_pair_at(NOW);
        let err = auth
            .verify_at(&pair.access_token, TokenKind::Access, NOW + 3601)
            .unwrap_err();
        assert!(matches!(err, TokenError::Expired { .. }));
    }

    #[test]
    fn a_token_from_another_deployment_is_refused() {
        let pair = auth(OTHER).mint_pair_at(NOW);
        let err = auth(SECRET)
            .verify_at(&pair.access_token, TokenKind::Access, NOW)
            .unwrap_err();
        assert_eq!(err, TokenError::BadSignature);
    }

    #[test]
    fn tampered_claims_are_refused() {
        let auth = auth(SECRET);
        let pair = auth.mint_pair_at(NOW);
        let (payload, sig) = pair.access_token.split_once('.').unwrap();
        // Re-encode the claims with an expiry far in the future, keeping the
        // original tag: the tag no longer covers what the payload says.
        let mut claims: TokenClaims =
            serde_json::from_slice(&B64.decode(payload).unwrap()).unwrap();
        claims.exp = NOW + 10_000_000;
        let forged = format!("{}.{sig}", B64.encode(serde_json::to_vec(&claims).unwrap()));
        assert_eq!(
            auth.verify_at(&forged, TokenKind::Access, NOW).unwrap_err(),
            TokenError::BadSignature
        );
    }

    #[test]
    fn a_refresh_token_is_not_accepted_as_an_access_token() {
        let auth = auth(SECRET);
        let pair = auth.mint_pair_at(NOW);
        let err = auth
            .verify_at(&pair.refresh_token, TokenKind::Access, NOW)
            .unwrap_err();
        assert!(matches!(
            err,
            TokenError::WrongKind {
                expected: TokenKind::Access,
                actual: TokenKind::Refresh
            }
        ));
    }

    #[test]
    fn garbage_is_refused_rather_than_panicking() {
        let auth = auth(SECRET);
        for bad in ["", ".", "not-a-token", "a.b", "$$$.$$$"] {
            assert!(auth.verify_at(bad, TokenKind::Access, NOW).is_err());
        }
    }

    #[test]
    fn a_refresh_token_buys_a_new_pair_without_the_secret() {
        let auth = auth(SECRET);
        let first = auth.mint_pair_at(NOW);
        let second = auth
            .refresh_at(&first.refresh_token, NOW + 100)
            .expect("refresh works");
        assert_ne!(first.access_token, second.access_token);
        assert_eq!(second.expires_at, NOW + 100 + 3600);
        auth.verify_at(&second.access_token, TokenKind::Access, NOW + 100)
            .expect("the new access token verifies");
    }

    #[test]
    fn an_expired_refresh_token_buys_nothing() {
        let auth = auth(SECRET);
        let pair = auth.mint_pair_at(NOW);
        assert!(auth.refresh_at(&pair.refresh_token, NOW + 86_401).is_err());
    }

    #[test]
    fn a_short_secret_is_refused() {
        let err = TokenAuth::new("tooshort", 3600, 86_400).unwrap_err();
        assert!(err.contains("at least"), "message was: {err}");
    }

    #[test]
    fn the_deployment_secret_is_recognised_and_nothing_else_is() {
        let auth = auth(SECRET);
        assert!(auth.is_deployment_secret(SECRET));
        assert!(!auth.is_deployment_secret(OTHER));
        assert!(!auth.is_deployment_secret(""));
    }

    #[test]
    fn debug_never_prints_the_secret() {
        let rendered = format!("{:?}", auth(SECRET));
        assert!(!rendered.contains(SECRET), "secret leaked: {rendered}");
    }
}
