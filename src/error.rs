use std::io;
use thiserror::Error;

pub(crate) const EXIT_ELEVATION_REQUIRED: i32 = 33;
pub(crate) const EXIT_OPERATION_FAILED: i32 = 1;
pub(crate) const EXIT_CANCELLED: i32 = 40;

#[derive(Debug, Error)]
pub(crate) enum AppError {
    #[error("{0}")]
    Message(String),
    #[error("Installation cancelled; changes were reverted")]
    Cancelled,
    #[error("Installation cancelled by user")]
    CancelledByUser,
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Toml(#[from] toml::de::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Windows(#[from] windows::core::Error),
}
