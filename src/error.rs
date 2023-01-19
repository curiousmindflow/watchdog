use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("No timer started")]
    NoTimerStarted,
    #[error("unknown error")]
    Unknown,
}
