use quinn::validation::{self, ValidationError};

macro_rules! test_function_name {
    ($name:expr) => {
        assert!(validation::validate_name(&String::from($name)).is_ok());
    };
    ($name: expr, $error_type:pat) => {
        let r = validation::validate_name(&String::from($name));
        assert!(r.is_err());
        assert!(matches!(r.unwrap_err(), $error_type));
    };
}
#[test]
fn function_name_too_long_function_name() {
    test_function_name!(
        "☠️".repeat(validation::MAX_NAME_LEN + 1),
        ValidationError::FunctionNameTooLong(..)
    );
}

#[test]
fn function_name_too_short_function_name() {
    test_function_name!(
        "x".repeat(validation::MIN_NAME_LEN - 1),
        ValidationError::FunctionNameTooShort(..)
    );
}

#[test]
fn function_name_with_invalid_characters() {
    // Names can only have alphanumerical and dash
    test_function_name!(
        "☠️".repeat(validation::MIN_NAME_LEN),
        ValidationError::FunctionNameContainsInvalidCharacters(..)
    );
    test_function_name!(
        "abc!",
        ValidationError::FunctionNameContainsInvalidCharacters(..)
    );

    // Names must start with a character and end with alphanumeric character
    test_function_name!(
        "-ab",
        ValidationError::FunctionNameContainsInvalidCharacters(..)
    );
    test_function_name!(
        "ab-",
        ValidationError::FunctionNameContainsInvalidCharacters(..)
    );
    test_function_name!(
        "1ab",
        ValidationError::FunctionNameContainsInvalidCharacters(..)
    );

    // Names can't have upper case characters
    test_function_name!(
        "ab-C",
        ValidationError::FunctionNameContainsInvalidCharacters(..)
    );
}

#[test]
fn valid_function_names() {
    test_function_name!("abc");
    test_function_name!("ab-c");
    test_function_name!("a1b");
    test_function_name!("ab1");
}

#[test]
fn test_validate_version() {
    assert!(validation::validate_version("").is_err());
    assert!(validation::validate_version("1.0,3").is_err());
    assert!(validation::validate_version("1.0.5-alpha").is_ok());
}
