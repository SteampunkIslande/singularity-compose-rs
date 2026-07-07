#[derive(thiserror::Error, Debug)]
pub enum SingularityComposeError {
    #[error(transparent)]
    IOError(#[from] std::io::Error),
    #[error("Invalid field in singularity-compose file: `{0}`")]
    InvalidField(String),
    #[error("Duplicate found for service `{0}`")]
    DuplicateService(String),
}
