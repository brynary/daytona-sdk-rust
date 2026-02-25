use crate::error::DaytonaError;

const DEFAULT_API_URL: &str = "https://app.daytona.io/api";
const ENV_API_KEY: &str = "DAYTONA_API_KEY";
const ENV_JWT_TOKEN: &str = "DAYTONA_JWT_TOKEN";
const ENV_ORGANIZATION_ID: &str = "DAYTONA_ORGANIZATION_ID";
const ENV_API_URL: &str = "DAYTONA_API_URL";
const ENV_TARGET: &str = "DAYTONA_TARGET";

/// Configuration for the Daytona SDK client.
#[derive(Debug, Clone, Default)]
pub struct DaytonaConfig {
    pub api_key: Option<String>,
    pub jwt_token: Option<String>,
    pub organization_id: Option<String>,
    pub api_url: Option<String>,
    pub target: Option<String>,
}

/// Resolved configuration with validated fields.
#[derive(Debug, Clone)]
pub(crate) struct ResolvedConfig {
    pub api_key: Option<String>,
    pub jwt_token: Option<String>,
    pub organization_id: Option<String>,
    pub api_url: String,
    pub target: Option<String>,
}

impl ResolvedConfig {
    pub fn bearer_token(&self) -> Option<&str> {
        self.api_key.as_deref().or(self.jwt_token.as_deref())
    }
}

/// Resolve configuration from struct fields and environment variables.
/// Struct fields take precedence over environment variables.
pub(crate) fn resolve_config(config: &DaytonaConfig) -> Result<ResolvedConfig, DaytonaError> {
    let api_key = config
        .api_key
        .clone()
        .or_else(|| std::env::var(ENV_API_KEY).ok())
        .filter(|s| !s.is_empty());

    let jwt_token = config
        .jwt_token
        .clone()
        .or_else(|| std::env::var(ENV_JWT_TOKEN).ok())
        .filter(|s| !s.is_empty());

    let organization_id = config
        .organization_id
        .clone()
        .or_else(|| std::env::var(ENV_ORGANIZATION_ID).ok())
        .filter(|s| !s.is_empty());

    let api_url = config
        .api_url
        .clone()
        .or_else(|| std::env::var(ENV_API_URL).ok())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_API_URL.to_string());

    let target = config
        .target
        .clone()
        .or_else(|| std::env::var(ENV_TARGET).ok())
        .filter(|s| !s.is_empty());

    // Validate: need api_key or jwt_token
    if api_key.is_none() && jwt_token.is_none() {
        return Err(DaytonaError::general(
            "either api_key or jwt_token must be provided (via config or environment variables)",
        ));
    }

    // Validate: jwt_token requires organization_id
    if api_key.is_none() && jwt_token.is_some() && organization_id.is_none() {
        return Err(DaytonaError::general(
            "organization_id is required when using jwt_token authentication",
        ));
    }

    Ok(ResolvedConfig {
        api_key,
        jwt_token,
        organization_id,
        api_url,
        target,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Mutex to serialize tests that modify environment variables.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn with_clean_env<F: FnOnce()>(f: F) {
        let _guard = ENV_LOCK.lock().unwrap();
        // Save and remove relevant env vars
        let saved: Vec<(String, Option<String>)> = [
            ENV_API_KEY,
            ENV_JWT_TOKEN,
            ENV_ORGANIZATION_ID,
            ENV_API_URL,
            ENV_TARGET,
        ]
        .iter()
        .map(|k| (k.to_string(), std::env::var(k).ok()))
        .collect();

        for (k, _) in &saved {
            std::env::remove_var(k);
        }

        f();

        // Restore env vars
        for (k, v) in saved {
            match v {
                Some(val) => std::env::set_var(&k, &val),
                None => std::env::remove_var(&k),
            }
        }
    }

    #[test]
    fn test_resolve_from_api_key_config() {
        with_clean_env(|| {
            let config = DaytonaConfig {
                api_key: Some("test-key".to_string()),
                ..Default::default()
            };
            let resolved = resolve_config(&config).unwrap();
            assert_eq!(resolved.api_key.as_deref(), Some("test-key"));
            assert_eq!(resolved.api_url, DEFAULT_API_URL);
        });
    }

    #[test]
    fn test_resolve_from_env_vars() {
        with_clean_env(|| {
            std::env::set_var(ENV_API_KEY, "env-key");
            std::env::set_var(ENV_API_URL, "https://custom.api.com");
            std::env::set_var(ENV_TARGET, "us");

            let config = DaytonaConfig::default();
            let resolved = resolve_config(&config).unwrap();
            assert_eq!(resolved.api_key.as_deref(), Some("env-key"));
            assert_eq!(resolved.api_url, "https://custom.api.com");
            assert_eq!(resolved.target.as_deref(), Some("us"));
        });
    }

    #[test]
    fn test_config_fields_override_env() {
        with_clean_env(|| {
            std::env::set_var(ENV_API_KEY, "env-key");
            std::env::set_var(ENV_API_URL, "https://env.api.com");

            let config = DaytonaConfig {
                api_key: Some("config-key".to_string()),
                api_url: Some("https://config.api.com".to_string()),
                ..Default::default()
            };
            let resolved = resolve_config(&config).unwrap();
            assert_eq!(resolved.api_key.as_deref(), Some("config-key"));
            assert_eq!(resolved.api_url, "https://config.api.com");
        });
    }

    #[test]
    fn test_jwt_with_org_id() {
        with_clean_env(|| {
            let config = DaytonaConfig {
                jwt_token: Some("jwt-token".to_string()),
                organization_id: Some("org-123".to_string()),
                ..Default::default()
            };
            let resolved = resolve_config(&config).unwrap();
            assert_eq!(resolved.jwt_token.as_deref(), Some("jwt-token"));
            assert_eq!(resolved.organization_id.as_deref(), Some("org-123"));
            assert_eq!(resolved.bearer_token(), Some("jwt-token"));
        });
    }

    #[test]
    fn test_api_key_preferred_over_jwt() {
        with_clean_env(|| {
            let config = DaytonaConfig {
                api_key: Some("api-key".to_string()),
                jwt_token: Some("jwt-token".to_string()),
                organization_id: Some("org-123".to_string()),
                ..Default::default()
            };
            let resolved = resolve_config(&config).unwrap();
            assert_eq!(resolved.bearer_token(), Some("api-key"));
        });
    }

    #[test]
    fn test_no_auth_returns_error() {
        with_clean_env(|| {
            let config = DaytonaConfig::default();
            let err = resolve_config(&config).unwrap_err();
            assert!(err.to_string().contains("api_key or jwt_token"));
        });
    }

    #[test]
    fn test_jwt_without_org_id_returns_error() {
        with_clean_env(|| {
            let config = DaytonaConfig {
                jwt_token: Some("jwt-token".to_string()),
                ..Default::default()
            };
            let err = resolve_config(&config).unwrap_err();
            assert!(err.to_string().contains("organization_id"));
        });
    }

    #[test]
    fn test_empty_string_treated_as_none() {
        with_clean_env(|| {
            let config = DaytonaConfig {
                api_key: Some("".to_string()),
                jwt_token: Some("actual-token".to_string()),
                organization_id: Some("org-123".to_string()),
                ..Default::default()
            };
            let resolved = resolve_config(&config).unwrap();
            assert!(resolved.api_key.is_none());
            assert_eq!(resolved.jwt_token.as_deref(), Some("actual-token"));
        });
    }

    #[test]
    fn test_default_api_url() {
        with_clean_env(|| {
            let config = DaytonaConfig {
                api_key: Some("key".to_string()),
                ..Default::default()
            };
            let resolved = resolve_config(&config).unwrap();
            assert_eq!(resolved.api_url, "https://app.daytona.io/api");
        });
    }
}
