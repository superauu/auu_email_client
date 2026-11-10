use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppError {
    pub code: &'static str,
    pub message: String,
    pub retryable: bool,
}

impl AppError {
    pub fn storage(message: impl Into<String>) -> Self {
        Self {
            code: "storage",
            message: message.into(),
            retryable: true,
        }
    }

    /// Malformed caller input (bad cursor, unknown folder kind, out-of-range page size).
    pub fn invalid(message: impl Into<String>) -> Self {
        Self {
            code: "invalid",
            message: message.into(),
            retryable: false,
        }
    }

    /// OS credential service failure. Never embeds the secret or the entry lookup error verbatim.
    pub fn credentials(message: impl Into<String>) -> Self {
        Self {
            code: "credentials",
            message: message.into(),
            retryable: false,
        }
    }

    /// DNS, TCP, TLS or protocol-level failure while talking to a mail server.
    pub fn network(message: impl Into<String>) -> Self {
        Self {
            code: "network",
            message: message.into(),
            retryable: true,
        }
    }

    /// The server rejected the credentials (IMAP LOGIN / SMTP AUTH).
    pub fn auth(message: impl Into<String>) -> Self {
        Self {
            code: "auth",
            message: message.into(),
            retryable: false,
        }
    }
}
