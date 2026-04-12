/// Persistence error types.
#[derive(Debug)]
pub enum PersistenceError {
    Io(std::io::Error),
    Serialize(String),
    Deserialize(String),
    VersionMismatch { expected: u32, actual: u32 },
}

impl std::fmt::Display for PersistenceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "IO error: {e}"),
            Self::Serialize(e) => write!(f, "Serialize error: {e}"),
            Self::Deserialize(e) => write!(f, "Deserialize error: {e}"),
            Self::VersionMismatch { expected, actual } => {
                write!(f, "Version mismatch: expected {expected}, got {actual}")
            }
        }
    }
}

impl std::error::Error for PersistenceError {}

impl From<std::io::Error> for PersistenceError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// Synchronous key-value persistence backend.
///
/// Blocking IO is fine — callers run these from the async task pool.
/// `Clone` required so the store can be cloned into async task closures.
pub trait Store<K, V>: Send + Sync + Clone + 'static {
    fn save(&self, key: &K, value: &V) -> Result<(), PersistenceError>;
    fn load(&self, key: &K) -> Result<Option<V>, PersistenceError>;
}
