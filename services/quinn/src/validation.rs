use regex::Regex;
use semver::Version;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ValidationError {
    #[error("Function name is too long! Max {0} characters. Name was {1} characters long.")]
    FunctionNameTooLong(usize, usize),

    #[error("Function name is too short! Min {0} characters. Name was {1} characters long.")]
    FunctionNameTooShort(usize, usize),

    #[error("Function name contains invalid characters. Only lower case characters, numbers and dashes are allowed")]
    FunctionNameContainsInvalidCharacters(),

    #[error("Version cannot be empty.")]
    EmptyVersion(),

    #[error("\"{0}\" is not a semantic version: {1}. See https://semver.org/")]
    InvalidSemanticVersion(String, semver::SemVerError),

    #[error("Invalid regex. This is an internal error.")]
    InvalidRegex(#[from] regex::Error),
}

pub const MAX_NAME_LEN: usize = 128;
pub const MIN_NAME_LEN: usize = 3;

pub fn validate_name(name: &str) -> Result<String, ValidationError> {
    if name.chars().count() > MAX_NAME_LEN {
        Err(ValidationError::FunctionNameTooLong(
            MAX_NAME_LEN,
            name.chars().count(),
        ))
    } else if name.chars().count() < MIN_NAME_LEN {
        Err(ValidationError::FunctionNameTooShort(
            MIN_NAME_LEN,
            name.chars().count(),
        ))
    } else if Regex::new(r"^[a-z][a-z0-9]{1,}([a-z0-9\-]?[a-z0-9]+)+$|^[a-z][a-z0-9]{2,}$")?
        .is_match(name)
    {
        Ok(name.to_owned())
    } else {
        Err(ValidationError::FunctionNameContainsInvalidCharacters())
    }
}

pub fn validate_version(version: &str) -> Result<Version, ValidationError> {
    if version.is_empty() {
        return Err(ValidationError::EmptyVersion());
    }

    Version::parse(version)
        .map_err(|e| ValidationError::InvalidSemanticVersion(version.to_owned(), e))
}
