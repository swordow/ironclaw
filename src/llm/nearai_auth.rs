use secrecy::{ExposeSecret, SecretString};

use crate::llm::LlmError;
use crate::llm::session::SessionManager;

/// Resolve the active NEAR AI bearer token.
///
/// Priority order:
/// 1. Explicit API key from resolved config
/// 2. Existing session token
/// 3. Interactive session authentication
/// 4. `NEARAI_API_KEY` from runtime environment
pub async fn resolve_nearai_bearer_token(
    api_key: Option<&SecretString>,
    session: &SessionManager,
) -> Result<String, LlmError> {
    if let Some(api_key) = api_key {
        return Ok(api_key.expose_secret().to_string());
    }

    if session.has_token().await {
        let token = session.get_token().await?;
        return Ok(token.expose_secret().to_string());
    }

    session.ensure_authenticated().await?;

    if session.has_token().await {
        let token = session.get_token().await?;
        return Ok(token.expose_secret().to_string());
    }

    if let Ok(key) = std::env::var("NEARAI_API_KEY")
        && !key.is_empty()
    {
        return Ok(key);
    }

    Err(LlmError::AuthFailed {
        provider: "nearai".to_string(),
    })
}
