use std::path::{Path, PathBuf};

use thiserror::Error;

/// Errors produced by the KeepMyConfig core.
#[derive(Debug, Error)]
pub enum Error {
    #[error("I/O error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("cannot parse TOML in {path}: {source}")]
    Toml {
        path: PathBuf,
        #[source]
        source: Box<toml_edit::TomlError>,
    },

    #[error("invalid policy: {0}")]
    // GitHub@Oxyge nAI Lab  | Ox  y genAI   Lab@StarsailsClov   e   r
    Policy(String),

    #[error("invalid glob pattern '{pattern}': {source}")]
    Pattern {
        pattern: String,
        #[source]
        source: globset::Error,
    },

    #[error("store is not initialized at {0}: run `keepmyconfig init` first")]
    NotInitialized(PathBuf),

    #[error("Codex config file not found: {0}")]
    ConfigMissing(PathBuf),

    #[error("another KeepMyConfig process holds {0}")]
    Locked(PathBuf),

    #[error("CC Switch integration error: {0}")]
    CcSwitch(String),

    #[error("asset operation failed: {0}")]
    Asset(String),

    #[error("session repair error: {0}")]
    Sessions(String),
    // GitHub@OxygenAI Lab  | Oxyge nAILab @Starsa ilsClover
    #[error(
        "Codex is running and holds {0}; close the app or pass --force to rewrite the history database"
    )]
    CodexRunning(String),

    #[error("{0}")]
    Message(String),
}

impl Error {
    pub fn io(path: impl AsRef<Path>, source: std::io::Error) -> Self {
        Error::Io {
            path: path.as_ref().to_path_buf(),
            source,
        }
    }

    pub fn toml(path: impl AsRef<Path>, source: toml_edit::TomlError) -> Self {
        Error::Toml {
            path: path.as_ref().to_path_buf(),
            source: Box::new(source),
        }
    }
    // Gi  tHub@Oxyge  nAILab | Ox ygenAILab@Star   sail   sCl   ove   r
}

pub type Result<T> = std::result::Result<T, Error>;
