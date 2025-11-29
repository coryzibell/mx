//! GitHub App authentication - JWT generation and installation token management
//!
//! Environment variables:
//! - DOTMATRIX_APP_ID: GitHub App ID
//! - DOTMATRIX_INSTALLATION_ID: Installation ID
//! - DOTMATRIX_PRIVATE_KEY: Full PEM private key content

use anyhow::{Context, Result};
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use serde::{Deserialize, Serialize};
use std::env;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// JWT claims for GitHub App authentication
#[derive(Debug, Serialize, Deserialize)]
struct Claims {
    /// Issued at (now - 60 seconds for clock skew)
    iat: u64,
    /// Expires at (now + 10 minutes)
    exp: u64,
    /// Issuer (GitHub App ID)
    iss: String,
}

/// GitHub installation access token response
#[derive(Debug, Deserialize)]
struct InstallationToken {
    token: String,
    expires_at: String,
}

/// Cached token with expiry tracking
#[derive(Debug, Clone)]
struct CachedToken {
    token: String,
    expires_at: SystemTime,
}

lazy_static::lazy_static! {
    static ref TOKEN_CACHE: Arc<Mutex<Option<CachedToken>>> = Arc::new(Mutex::new(None));
}

/// Check if GitHub App credentials are configured
///
/// Returns true if all required environment variables are set:
/// - DOTMATRIX_APP_ID
/// - DOTMATRIX_INSTALLATION_ID
/// - DOTMATRIX_PRIVATE_KEY
pub fn is_app_configured() -> bool {
    env::var("DOTMATRIX_APP_ID").is_ok()
        && env::var("DOTMATRIX_INSTALLATION_ID").is_ok()
        && env::var("DOTMATRIX_PRIVATE_KEY").is_ok()
}

/// Generate a JWT for GitHub App authentication
///
/// # Arguments
///
/// * `app_id` - GitHub App ID
/// * `private_key` - PEM-formatted RSA private key
///
/// # Errors
///
/// Returns an error if:
/// - Private key is invalid
/// - JWT encoding fails
pub fn generate_jwt(app_id: &str, private_key: &str) -> Result<String> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("System time before UNIX epoch")?
        .as_secs();

    let claims = Claims {
        iat: now - 60,  // 60 seconds ago for clock skew
        exp: now + 600, // 10 minutes from now
        iss: app_id.to_string(),
    };

    let header = Header::new(Algorithm::RS256);
    let encoding_key = EncodingKey::from_rsa_pem(private_key.as_bytes())
        .context("Failed to parse RSA private key")?;

    encode(&header, &claims, &encoding_key).context("Failed to encode JWT")
}

/// Get an installation access token (caches and refreshes automatically)
///
/// Reads credentials from environment variables:
/// - DOTMATRIX_APP_ID
/// - DOTMATRIX_INSTALLATION_ID
/// - DOTMATRIX_PRIVATE_KEY
///
/// Tokens are cached with a 5-minute expiry buffer (refreshed at 5 minutes before expiry).
///
/// # Errors
///
/// Returns an error if:
/// - Environment variables are not set
/// - JWT generation fails
/// - Token exchange API call fails
pub fn get_installation_token() -> Result<String> {
    // Check cache first
    {
        let cache = TOKEN_CACHE.lock().unwrap();
        if let Some(cached) = cache.as_ref() {
            let now = SystemTime::now();
            // Use token if more than 5 minutes until expiry
            if cached.expires_at > now + Duration::from_secs(300) {
                return Ok(cached.token.clone());
            }
        }
    }

    // Cache miss or expired - generate new token
    let app_id =
        env::var("DOTMATRIX_APP_ID").context("DOTMATRIX_APP_ID environment variable not set")?;
    let installation_id = env::var("DOTMATRIX_INSTALLATION_ID")
        .context("DOTMATRIX_INSTALLATION_ID environment variable not set")?;
    let private_key = env::var("DOTMATRIX_PRIVATE_KEY")
        .context("DOTMATRIX_PRIVATE_KEY environment variable not set")?;

    // Generate JWT
    let jwt = generate_jwt(&app_id, &private_key)?;

    // Exchange JWT for installation token
    let client = reqwest::blocking::Client::new();
    let url = format!(
        "https://api.github.com/app/installations/{}/access_tokens",
        installation_id
    );

    let response = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", jwt))
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", "mx-cli")
        .send()
        .context("Failed to request installation token")?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().unwrap_or_default();
        anyhow::bail!("GitHub API returned error {}: {}", status, body);
    }

    let token_response: InstallationToken = response
        .json()
        .context("Failed to parse installation token response")?;

    // Parse expiry time
    let expires_at = chrono::DateTime::parse_from_rfc3339(&token_response.expires_at)
        .context("Failed to parse token expiry time")?
        .with_timezone(&chrono::Utc);

    let expires_at_system = UNIX_EPOCH + Duration::from_secs(expires_at.timestamp() as u64);

    // Update cache
    {
        let mut cache = TOKEN_CACHE.lock().unwrap();
        *cache = Some(CachedToken {
            token: token_response.token.clone(),
            expires_at: expires_at_system,
        });
    }

    Ok(token_response.token)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_app_configured_missing_vars() {
        // Should return false if any var is missing
        env::remove_var("DOTMATRIX_APP_ID");
        env::remove_var("DOTMATRIX_INSTALLATION_ID");
        env::remove_var("DOTMATRIX_PRIVATE_KEY");
        assert!(!is_app_configured());
    }

    #[test]
    #[ignore]
    fn test_generate_jwt_integration() {
        // This test requires actual credentials
        let app_id = env::var("DOTMATRIX_APP_ID").expect("DOTMATRIX_APP_ID not set");
        let private_key = env::var("DOTMATRIX_PRIVATE_KEY").expect("DOTMATRIX_PRIVATE_KEY not set");

        let jwt = generate_jwt(&app_id, &private_key).expect("JWT generation failed");
        assert!(!jwt.is_empty());
        // JWT should have 3 parts separated by dots
        assert_eq!(jwt.matches('.').count(), 2);
    }

    #[test]
    #[ignore]
    fn test_get_installation_token_integration() {
        // This test requires actual credentials and network access
        let token = get_installation_token().expect("Token fetch failed");
        assert!(!token.is_empty());
        // GitHub App tokens start with "ghs_"
        assert!(token.starts_with("ghs_"));
    }
}
