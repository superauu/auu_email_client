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
}
