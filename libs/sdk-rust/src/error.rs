use std::collections::HashMap;

/// Top-level error type for the Daytona SDK.
#[derive(Debug, thiserror::Error)]
pub enum DaytonaError {
    /// An API error with HTTP status code and optional headers.
    #[error("{message}")]
    Api {
        status_code: u16,
        message: String,
        headers: HashMap<String, String>,
    },

    /// Resource not found (HTTP 404).
    #[error("{message}")]
    NotFound {
        message: String,
        headers: HashMap<String, String>,
    },

    /// Rate limit exceeded (HTTP 429).
    #[error("{message}")]
    RateLimit {
        message: String,
        headers: HashMap<String, String>,
    },

    /// Operation timed out.
    #[error("{message}")]
    Timeout { message: String },

    /// General error that wraps any other failure.
    #[error("{0}")]
    General(String),
}

impl DaytonaError {
    pub fn api(status_code: u16, message: impl Into<String>) -> Self {
        Self::Api {
            status_code,
            message: message.into(),
            headers: HashMap::new(),
        }
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::NotFound {
            message: message.into(),
            headers: HashMap::new(),
        }
    }

    pub fn rate_limit(message: impl Into<String>) -> Self {
        Self::RateLimit {
            message: message.into(),
            headers: HashMap::new(),
        }
    }

    pub fn timeout(message: impl Into<String>) -> Self {
        Self::Timeout {
            message: message.into(),
        }
    }

    pub fn general(message: impl Into<String>) -> Self {
        Self::General(message.into())
    }

    /// Extract the HTTP status code if present.
    pub fn status_code(&self) -> Option<u16> {
        match self {
            Self::Api { status_code, .. } => Some(*status_code),
            Self::NotFound { .. } => Some(404),
            Self::RateLimit { .. } => Some(429),
            _ => None,
        }
    }

    /// Extract the error message.
    pub fn message(&self) -> &str {
        match self {
            Self::Api { message, .. } => message,
            Self::NotFound { message, .. } => message,
            Self::RateLimit { message, .. } => message,
            Self::Timeout { message } => message,
            Self::General(message) => message,
        }
    }
}

/// Convert an HTTP response status and body into the appropriate DaytonaError variant.
pub fn error_from_response(
    status_code: u16,
    body: &str,
    headers: HashMap<String, String>,
) -> DaytonaError {
    let message = extract_error_message(body, status_code);

    match status_code {
        404 => DaytonaError::NotFound { message, headers },
        429 => DaytonaError::RateLimit { message, headers },
        _ => DaytonaError::Api {
            status_code,
            message,
            headers,
        },
    }
}

fn extract_error_message(body: &str, status_code: u16) -> String {
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(body) {
        if let Some(msg) = json.get("message").and_then(|v| v.as_str()) {
            return msg.to_string();
        }
        if let Some(msg) = json.get("error").and_then(|v| v.as_str()) {
            return msg.to_string();
        }
    }
    if body.is_empty() {
        format!("HTTP {status_code}")
    } else {
        body.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display_messages() {
        let err = DaytonaError::api(500, "internal server error");
        assert_eq!(err.to_string(), "internal server error");

        let err = DaytonaError::not_found("sandbox xyz not found");
        assert_eq!(err.to_string(), "sandbox xyz not found");

        let err = DaytonaError::rate_limit("too many requests");
        assert_eq!(err.to_string(), "too many requests");

        let err = DaytonaError::timeout("operation timed out after 30s");
        assert_eq!(err.to_string(), "operation timed out after 30s");

        let err = DaytonaError::general("something went wrong");
        assert_eq!(err.to_string(), "something went wrong");
    }

    #[test]
    fn test_variant_matching() {
        let err = DaytonaError::not_found("gone");
        assert!(matches!(err, DaytonaError::NotFound { .. }));

        let err = DaytonaError::rate_limit("slow down");
        assert!(matches!(err, DaytonaError::RateLimit { .. }));

        let err = DaytonaError::api(503, "unavailable");
        assert!(matches!(
            err,
            DaytonaError::Api {
                status_code: 503,
                ..
            }
        ));

        let err = DaytonaError::timeout("timed out");
        assert!(matches!(err, DaytonaError::Timeout { .. }));

        let err = DaytonaError::general("oops");
        assert!(matches!(err, DaytonaError::General(_)));
    }

    #[test]
    fn test_status_code_extraction() {
        assert_eq!(DaytonaError::api(500, "err").status_code(), Some(500));
        assert_eq!(DaytonaError::not_found("err").status_code(), Some(404));
        assert_eq!(DaytonaError::rate_limit("err").status_code(), Some(429));
        assert_eq!(DaytonaError::timeout("err").status_code(), None);
        assert_eq!(DaytonaError::general("err").status_code(), None);
    }

    #[test]
    fn test_error_from_response_404() {
        let err = error_from_response(404, r#"{"message":"not found"}"#, HashMap::new());
        assert!(matches!(err, DaytonaError::NotFound { .. }));
        assert_eq!(err.message(), "not found");
    }

    #[test]
    fn test_error_from_response_429() {
        let err = error_from_response(429, r#"{"message":"rate limited"}"#, HashMap::new());
        assert!(matches!(err, DaytonaError::RateLimit { .. }));
        assert_eq!(err.message(), "rate limited");
    }

    #[test]
    fn test_error_from_response_500() {
        let err = error_from_response(500, r#"{"error":"server error"}"#, HashMap::new());
        assert!(matches!(
            err,
            DaytonaError::Api {
                status_code: 500,
                ..
            }
        ));
        assert_eq!(err.message(), "server error");
    }

    #[test]
    fn test_error_from_response_empty_body() {
        let err = error_from_response(503, "", HashMap::new());
        assert_eq!(err.message(), "HTTP 503");
    }

    #[test]
    fn test_error_from_response_non_json_body() {
        let err = error_from_response(502, "Bad Gateway", HashMap::new());
        assert_eq!(err.message(), "Bad Gateway");
    }

    #[test]
    fn test_error_from_response_preserves_headers() {
        let mut headers = HashMap::new();
        headers.insert("retry-after".to_string(), "60".to_string());
        let err = error_from_response(429, r#"{"message":"rate limited"}"#, headers);
        if let DaytonaError::RateLimit {
            headers: err_headers,
            ..
        } = &err
        {
            assert_eq!(err_headers.get("retry-after").unwrap(), "60");
        } else {
            panic!("expected RateLimit variant");
        }
    }
}
