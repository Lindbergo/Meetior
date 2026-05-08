use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("storage: {0}")]
    Storage(#[from] rusqlite::Error),

    #[error("storage pool: {0}")]
    Pool(#[from] r2d2::Error),

    #[error("audio: {0}")]
    Audio(String),

    #[error("asr: {0}")]
    Asr(String),

    #[error("summarizer: {0}")]
    Summarizer(String),

    #[error("http: {0}")]
    Http(#[from] reqwest::Error),

    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("invalid state: {0}")]
    InvalidState(String),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

// Tauri commands need errors to serialize across the bridge.
impl Serialize for Error {
    fn serialize<S>(&self, s: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        s.serialize_str(&self.to_string())
    }
}
