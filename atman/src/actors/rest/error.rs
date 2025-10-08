use std::io;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("IO error: {message}: {cause}")]
    IO { message: String, cause: io::Error },
}
