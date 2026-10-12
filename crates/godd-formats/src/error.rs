#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("bad magic: expected {expected:?}, found {found:?}")]
    BadMagic { expected: &'static [u8], found: Vec<u8> },
    #[error("unexpected end of data reading {what} at offset {offset:#x}")]
    Truncated { what: &'static str, offset: usize },
    #[error("unsupported build stamp {0:#010x}")]
    UnsupportedBuild(u32),
    #[error("{0}")]
    Invalid(String),
}

pub type Result<T> = std::result::Result<T, Error>;
