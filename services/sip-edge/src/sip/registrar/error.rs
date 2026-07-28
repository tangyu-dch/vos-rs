use std::{error::Error, fmt};

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(clippy::enum_variant_names)]
pub enum RegisterError {
    InvalidAddressOfRecord(String),
    InvalidContact(String),
    InvalidExpires(String),
}

impl fmt::Display for RegisterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidAddressOfRecord(value) => {
                write!(f, "invalid REGISTER address-of-record: {value}")
            }
            Self::InvalidContact(value) => write!(f, "invalid REGISTER Contact: {value}"),
            Self::InvalidExpires(value) => write!(f, "invalid REGISTER Expires: {value}"),
        }
    }
}

impl Error for RegisterError {}
