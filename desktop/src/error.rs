use thiserror::Error;

/// Main error type for the Reviu application
#[derive(Error, Debug)]
pub enum Error {
  #[error("Git error: {0}")]
  Git(#[from] git2::Error),

  #[error("IO error: {0}")]
  Io(#[from] std::io::Error),

  #[error("Database error: {0}")]
  Database(#[from] rusqlite::Error),

  #[error("HTTP error: {0}")]
  Http(#[from] reqwest::Error),

  #[error("Serialization error: {0}")]
  Serialization(#[from] serde_json::Error),

  #[error("Keyring error: {0}")]
  Keyring(#[from] keyring::Error),

  #[error("Repository not found: {0}")]
  RepositoryNotFound(String),

  #[error("Invalid repository path: {0}")]
  InvalidRepositoryPath(String),

  #[error("No repository open")]
  NoRepositoryOpen,

  #[error("File not found: {0}")]
  FileNotFound(String),

  #[error("Invalid diff operation: {0}")]
  InvalidDiffOperation(String),

  #[error("Authentication required")]
  AuthenticationRequired,

  #[error("Unauthorized: {0}")]
  Unauthorized(String),

  #[error("Network error: {0}")]
  Network(String),

  #[error("Configuration error: {0}")]
  Config(String),

  #[error("Unknown error: {0}")]
  Unknown(String),
}

/// Result type alias for Reviu operations
pub type Result<T> = std::result::Result<T, Error>;
