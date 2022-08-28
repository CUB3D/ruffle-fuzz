use thiserror::Error;

#[derive(Error, Debug)]
pub enum MyError {
    #[error("Flash Crash")]
    FlashCrash,

    #[error("Io Error")]
    IoError(#[from] std::io::Error),

    #[error("Popen Error")]
    PopenError(#[from] subprocess::PopenError),
}
