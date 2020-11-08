use std::{
    collections::HashMap,
    fmt::{Display, Formatter},
};

use thiserror::Error;

use super::{
    execution::{
        channel::Value as ValueType, Booleans, Bytes, Channel, Floats, Integers,
        Stream as ValueStream, Strings,
    },
    functions::{ChannelSpec, ChannelType},
    DisplayExt, Displayer,
};

#[derive(Error, Debug)]
#[error("Could not convert \"{found_type}\" to \"{expected_type}\".")]
pub struct ChannelConversionError {
    expected_type: String,
    found_type: String,
}

/// Convert any supported value to a channel
///
/// This converts any Rust value that can be converted
/// into a channel
pub trait ToChannel {
    fn to_channel(self) -> Channel;
}

/// Create a reference to a value from a channel
pub trait TryRefFromChannel<'a> {
    fn try_ref_from(channel: &'a Channel) -> Result<&'a Self, ChannelConversionError>;
}

/// Reciprocal trait for `RefFromChannel`.
///
/// Get a reference to the values in a Channel as type `T`
/// for which `RefFromChannel` is implemented
pub trait TryChannelAsRef<'a, T: TryRefFromChannel<'a> + ?Sized> {
    fn try_as_ref(&'a self) -> Result<&'a T, ChannelConversionError>;
}

impl<'a, T: TryRefFromChannel<'a> + ?Sized> TryChannelAsRef<'a, T> for Channel {
    fn try_as_ref(&'a self) -> Result<&'a T, ChannelConversionError> {
        T::try_ref_from(self)
    }
}

/// Convert a channel to a Rust type
///
/// As indicated by the "try" prefix, this
/// will fail if the channel cannot be
/// converted into the requested type
pub trait TryFromChannel: Sized {
    fn try_from(channel: &Channel) -> Result<Self, ChannelConversionError>;
}

/// Reciprocal trait for TryFromChannel
pub trait ChannelInto<T: TryFromChannel> {
    fn channel_into(&self) -> Result<T, ChannelConversionError>;
}

impl<T> ChannelInto<T> for Channel
where
    T: TryFromChannel,
{
    fn channel_into(&self) -> Result<T, ChannelConversionError> {
        T::try_from(self)
    }
}
fn get_exactly_one<T>(
    values: &[T],
    expected_type_name: String,
) -> Result<&T, ChannelConversionError> {
    if values.len() > 1 {
        Err(ChannelConversionError {
            expected_type: expected_type_name.clone(),
            found_type: format!("array of {}s", &expected_type_name),
        })
    } else {
        values.iter().next().ok_or_else(|| ChannelConversionError {
            expected_type: expected_type_name,
            found_type: None::<ValueType>.display().to_string(),
        })
    }
}

macro_rules! as_ref_try_from_impl {
    ($ref_type:ty, $expected_type:path, $expected_type_name:expr) => {
        impl<'a> TryRefFromChannel<'a> for [$ref_type] {
            fn try_ref_from(
                channel: &'a Channel,
            ) -> Result<&'a [$ref_type], ChannelConversionError> {
                if let Some($expected_type(v)) = channel.value.as_ref() {
                    Ok(&v.values)
                } else {
                    Err(ChannelConversionError {
                        expected_type: format!("array of {}s", $expected_type_name),
                        found_type: channel.display().to_string(),
                    })
                }
            }
        }

        impl TryFromChannel for Vec<$ref_type> {
            fn try_from(channel: &Channel) -> Result<Self, ChannelConversionError> {
                if let Some($expected_type(v)) = channel.value.as_ref() {
                    Ok(v.values.to_vec())
                } else {
                    Err(ChannelConversionError {
                        expected_type: format!("array of {}s", $expected_type_name),
                        found_type: channel.display().to_string(),
                    })
                }
            }
        }

        impl<'a> TryRefFromChannel<'a> for $ref_type {
            fn try_ref_from(channel: &'a Channel) -> Result<&'a $ref_type, ChannelConversionError> {
                if let Some($expected_type(v)) = channel.value.as_ref() {
                    get_exactly_one(&v.values, String::from($expected_type_name))
                } else {
                    Err(ChannelConversionError {
                        expected_type: String::from($expected_type_name),
                        found_type: channel.display().to_string(),
                    })
                }
            }
        }

        impl TryFromChannel for $ref_type {
            fn try_from(channel: &Channel) -> Result<Self, ChannelConversionError> {
                if let Some($expected_type(v)) = channel.value.as_ref() {
                    get_exactly_one(&v.values, String::from($expected_type_name)).map(|r| r.clone())
                } else {
                    Err(ChannelConversionError {
                        expected_type: String::from($expected_type_name),
                        found_type: channel.display().to_string(),
                    })
                }
            }
        }
    };
}

macro_rules! to_channel_impl {
    ($for_type:ty, $channel_type:path, $channel_inner_type:ident) => {
        impl ToChannel for Vec<$for_type> {
            fn to_channel(self) -> Channel {
                Channel {
                    value: Some($channel_type($channel_inner_type {
                        values: self.into_iter().map(|v| v.into()).collect(),
                    })),
                }
            }
        }

        impl ToChannel for $for_type {
            fn to_channel(self) -> Channel {
                Channel {
                    value: Some($channel_type($channel_inner_type {
                        values: vec![self.into()],
                    })),
                }
            }
        }
    };
}

impl<T: ToChannel> ToChannel for Option<T> {
    fn to_channel(self) -> Channel {
        match self {
            Some(value) => value.to_channel(),
            None => Channel { value: None },
        }
    }
}

// bytes
as_ref_try_from_impl!(u8, ValueType::Bytes, "byte");

// strings
as_ref_try_from_impl!(String, ValueType::Strings, "string");

// integers
as_ref_try_from_impl!(i64, ValueType::Integers, "integer");

// floats
as_ref_try_from_impl!(f64, ValueType::Floats, "float");

// bools
as_ref_try_from_impl!(bool, ValueType::Booleans, "boolean");

// bytes
to_channel_impl!(u8, ValueType::Bytes, Bytes);

// strings
to_channel_impl!(String, ValueType::Strings, Strings);
to_channel_impl!(&'static str, ValueType::Strings, Strings);

// integers
to_channel_impl!(i8, ValueType::Integers, Integers);
to_channel_impl!(i16, ValueType::Integers, Integers);
to_channel_impl!(i32, ValueType::Integers, Integers);
to_channel_impl!(i64, ValueType::Integers, Integers);
to_channel_impl!(u16, ValueType::Integers, Integers);
to_channel_impl!(u32, ValueType::Integers, Integers);

// floats
to_channel_impl!(f32, ValueType::Floats, Floats);
to_channel_impl!(f64, ValueType::Floats, Floats);

// bools
to_channel_impl!(bool, ValueType::Booleans, Booleans);

/// Convenience extensions on a stream
///
/// A stream is a collection of named
/// channels that contain an async data stream
pub trait StreamExt {
    /// Create a new empty stream
    fn new() -> Self;

    /// Retrieve a channel value as the requested type
    ///
    /// Will return an error if the channel value
    /// could not be converted into the requested type.
    fn get_channel_as_ref<'a, T: TryRefFromChannel<'a> + ?Sized>(
        &'a self,
        name: &'a str,
    ) -> Result<&'a T, ChannelConversionError>;

    /// Get a channel by name
    ///
    /// Will get the channel indicated by `name` or `None`
    /// if no such channel exists
    fn get_channel(&self, name: &str) -> Option<&Channel>;

    /// Check whether a channel exists
    ///
    /// Returns true if a channel with `name` exists
    ///
    /// # Example
    /// ```
    /// use firm_protocols::execution::Stream;
    /// use firm_types::stream::StreamExt;
    ///
    /// let s = Stream::new();
    /// assert!(!s.has_channel("should-be-empty"));
    /// ```
    fn has_channel(&self, name: &str) -> bool;

    /// Set a channel in the stream
    ///
    /// Will set the channel with `name` to the provided `channel`.
    ///
    /// # Example
    /// ```
    /// use firm_protocols::execution::{Channel, Stream};
    /// use firm_types::stream::StreamExt;
    ///
    /// let mut s = Stream::new();
    /// s.set_channel("kanal1", Channel { value: None });
    /// assert!(s.has_channel("kanal1"));
    /// ```
    fn set_channel(&mut self, name: &str, channel: Channel);

    /// Validate this stream according to spec
    ///
    /// `required` is the specs for the required channels
    /// `optional` is the specs for the optional channels
    ///
    /// This function returns all validation errors as a `Vec<StreamValidationError>`.
    fn validate(
        &self,
        required: &HashMap<String, ChannelSpec>,
        optional: Option<&HashMap<String, ChannelSpec>>,
    ) -> Result<(), Vec<StreamValidationError>>;

    /// Merge this stream with another one
    ///
    /// This will consume `other`, incorporating it into
    /// `self`.
    fn merge(&mut self, other: Self) -> &mut Self;
}

#[derive(Debug, Error)]
pub enum StreamValidationError {
    #[error(
        "Channel \"{channel_name}\" has unexpected type. Expected \"{expected}\", got \"{got}\""
    )]
    MismatchedChannelType {
        channel_name: String,
        expected: String,
        got: String,
    },

    #[error("Channel \"{0}\" was not expected by spec")]
    UnexpectedChannel(String),

    #[error("Failed to find required channel {0}")]
    RequiredChannelMissing(String),
}

impl StreamExt for ValueStream {
    fn new() -> Self {
        Self {
            channels: std::collections::HashMap::new(),
        }
    }

    fn get_channel_as_ref<'a, T: TryRefFromChannel<'a> + ?Sized>(
        &'a self,
        name: &'a str,
    ) -> Result<&'a T, ChannelConversionError> {
        T::try_ref_from(
            self.get_channel(name)
                .ok_or_else(|| ChannelConversionError {
                    expected_type: "?".to_owned(),
                    found_type: Channel { value: None }.display().to_string(),
                })?,
        )
    }

    fn get_channel(&self, name: &str) -> Option<&Channel> {
        self.channels.get(name)
    }

    fn has_channel(&self, name: &str) -> bool {
        self.channels.contains_key(name)
    }

    fn set_channel(&mut self, name: &str, channel: Channel) {
        self.channels.insert(name.to_owned(), channel);
    }

    fn merge(&mut self, other: Self) -> &mut Self {
        self.channels.extend(other.channels);
        self
    }

    fn validate(
        &self,
        required: &HashMap<String, ChannelSpec>,
        optional: Option<&HashMap<String, ChannelSpec>>,
    ) -> Result<(), Vec<StreamValidationError>> {
        let err: Vec<StreamValidationError> = required
            .iter()
            .map(|(name, channel_spec)| {
                self.channels
                    .get(name)
                    .map(|channel| (name, channel, channel_spec))
                    .ok_or_else(|| StreamValidationError::RequiredChannelMissing(name.clone()))
            })
            .chain(optional.iter().flat_map(|o| {
                o.iter().filter_map(|(name, channel_spec)| {
                    self.channels.get(name).map(|c| Ok((name, c, channel_spec)))
                })
            }))
            .chain(self.channels.keys().filter_map(|k| {
                if required.contains_key(k) || optional.map_or(false, |opt| opt.contains_key(k)) {
                    None
                } else {
                    Some(Err(StreamValidationError::UnexpectedChannel(k.clone())))
                }
            }))
            .filter_map(|r| match r {
                Ok((name, channel, channel_spec)) => {
                    if channel.type_matches(channel_spec) {
                        None
                    } else {
                        Some(StreamValidationError::MismatchedChannelType {
                            channel_name: name.clone(),
                            expected: channel_spec.display().to_string(),
                            got: channel.display().to_string(),
                        })
                    }
                }
                Err(e) => Some(e),
            })
            .collect();

        if err.is_empty() {
            Ok(())
        } else {
            Err(err)
        }
    }
}

/// Comparison trait for channels and channel specs
///
/// Used when validating streams agains stream specs
trait ChannelTypeMatches<T> {
    fn type_matches(&self, other: &T) -> bool;
}

impl ChannelTypeMatches<ChannelSpec> for Channel {
    fn type_matches(&self, spec: &ChannelSpec) -> bool {
        match self.value {
            Some(ValueType::Strings(_)) => spec.r#type == ChannelType::String as i32,
            Some(ValueType::Integers(_)) => spec.r#type == ChannelType::Int as i32,
            Some(ValueType::Floats(_)) => spec.r#type == ChannelType::Float as i32,
            Some(ValueType::Booleans(_)) => spec.r#type == ChannelType::Bool as i32,
            Some(ValueType::Bytes(_)) => spec.r#type == ChannelType::Bytes as i32,
            None => false,
        }
    }
}

impl ChannelTypeMatches<Channel> for ChannelSpec {
    fn type_matches(&self, channel: &Channel) -> bool {
        channel.type_matches(self)
    }
}

impl Display for Displayer<'_, Channel> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.value.display())
    }
}

impl Display for Displayer<'_, Option<ValueType>> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match **self {
                Some(ValueType::Strings(_)) => "strings",
                Some(ValueType::Integers(_)) => "integers",
                Some(ValueType::Booleans(_)) => "booleans",
                Some(ValueType::Floats(_)) => "floats",
                Some(ValueType::Bytes(_)) => "bytes",
                None => "null",
            }
        )
    }
}

impl Display for Displayer<'_, ChannelSpec> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match ChannelType::from_i32(self.r#type) {
                Some(ChannelType::String) => String::from("string"),
                Some(ChannelType::Int) => String::from("integer"),
                Some(ChannelType::Bool) => String::from("boolean"),
                Some(ChannelType::Float) => String::from("float"),
                Some(ChannelType::Bytes) => String::from("bytes"),
                None => format!("Unknown type with discriminator {}", self),
            },
        )
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    use crate::{channel_specs, stream};

    #[test]
    fn parse_required() {
        let input_spec = channel_specs!({"very_important_argument" => ChannelSpec {
            description: "This is importante!".to_owned(),
            r#type: ChannelType::String as i32,
        }});

        let r = stream!().validate(&input_spec.0, input_spec.1.as_ref());
        assert!(r.is_err());

        let stream = stream!({"very_important_argument" => "yes"});
        let r = stream.validate(&input_spec.0, input_spec.1.as_ref());
        assert!(r.is_ok());
    }

    #[test]
    fn parse_optional() {
        let input_spec = channel_specs!(
            {},
            {
                "not_very_important_argument" => ChannelSpec {
                    description: "I do not like this".to_owned(),
                    r#type: ChannelType::String as i32
                }
            }
        );

        assert!(stream!()
            .validate(&input_spec.0, input_spec.1.as_ref())
            .is_ok());
    }

    #[test]
    fn too_many_args() {
        let input_spec = channel_specs!({"only_this_please" => ChannelSpec {
            description: "The only thing".to_owned(),
            r#type: ChannelType::String as i32,
        }});
        let stream = stream!({"only_this_please" => "no", "but_also_this" => "yes"});

        let r = stream.validate(&input_spec.0, input_spec.1.as_ref());
        assert!(r.is_err());
        let e = r.unwrap_err();
        assert_eq!(e.len(), 1);
        assert!(matches!(
            e.first().unwrap(),
            StreamValidationError::UnexpectedChannel(..)
        ));
        let stream =
            stream!({"only_this_please" => "no", "but_also_this" => "yes", "and_this" => "ok"});
        let r = stream.validate(&input_spec.0, input_spec.1.as_ref());
        assert!(r.is_err());
        let e = r.unwrap_err();
        assert_eq!(e.len(), 2);

        let stream = stream!({"but_also_this" => "yes", "and_this" => "ok"});
        let r = stream.validate(&input_spec.0, input_spec.1.as_ref());
        assert!(r.is_err());
        let e = r.unwrap_err();
        assert_eq!(e.len(), 3);
    }

    #[test]
    fn parse_types() {
        let input_spec = channel_specs!({
            "string_arg" => ChannelSpec {
                description: "This is a string arg".to_owned(),
                r#type: ChannelType::String as i32,
            },
            "bool_arg" => ChannelSpec {
                description: "This is a boll arg ⚽".to_owned(),
                r#type: ChannelType::Bool as i32,

            },
            "int_arg" => ChannelSpec {
                description: "This is an int arg".to_owned(),
                r#type: ChannelType::Int as i32,
            },
            "float_arg" => ChannelSpec {
                description: "This is a floater 💩".to_owned(),
                r#type: ChannelType::Float as i32,
            }
        }, {
            "bytes_arg" => ChannelSpec {
                description: "This is a bytes argument".to_owned(),
                r#type: ChannelType::Bytes as i32,
            }
        });

        let correct_args = stream!(
            {
                "string_arg" => "yes",
                "bool_arg" => true,
                "int_arg" => 4i64,
                "float_arg" => 4.5f32,
                "bytes_arg" => vec![13u8, 37u8, 13u8, 37u8, 13u8, 37u8]
            }
        );

        let r = correct_args.validate(&input_spec.0, input_spec.1.as_ref());

        assert!(r.is_ok());

        // one has the wrong type 🤯
        let almost_correct_args = stream!(
            {
                "string_arg" => "yes",
                "bool_arg" => 321i64,
                "int_arg" => 4i64,
                "float_arg" => 4.5f32
            }
        );

        let r = almost_correct_args.validate(&input_spec.0, input_spec.1.as_ref());

        assert!(r.is_err());
        assert_eq!(1, r.unwrap_err().len());

        // all of them has the wrong type 🚓💨
        let no_correct_args = stream!(
            {
                "string_arg" => true,
                "bool_arg" => "noooo!",
                "int_arg" => false,
                "float_arg" => "4.5f32",
                "bytes_arg" => true
            }
        );

        let r = no_correct_args.validate(&input_spec.0, input_spec.1.as_ref());

        assert!(r.is_err());
        assert_eq!(5, r.unwrap_err().len());
    }
}
