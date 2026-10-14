//! Runtime support shared by every generated protocol module: the tri-state `Maybe<T>` and the two
//! macros that expand string enums and tagged unions with forward-compatible `Unrecognized` variants.
//!
//! Depends on: serde, serde_json. Used by: the generated area modules in this directory.
//! Invariants: an unknown enum value or union tag never makes decoding fail; it is kept verbatim
//! and serialises back unchanged, so a newer Vibe can add variants without breaking this client.

#![allow(clippy::pedantic)]

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A field Pydantic allows to be absent AND null: absent, null and a value are three different
/// wire states, and the type keeps all three so re-serialising reproduces the input exactly.
///
/// Used only for properties that are both optional and nullable; plain optional properties are
/// `Option<T>` and required nullable ones are `Option<T>` that always serialise (as `null`).
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Maybe<T> {
    /// The key is not present on the wire.
    #[default]
    Absent,
    /// The key is present with the JSON value `null`.
    Null,
    /// The key is present with a value.
    Value(T),
}

impl<T> Maybe<T> {
    /// True when the key is absent (used by `skip_serializing_if`).
    #[must_use]
    pub fn is_absent(&self) -> bool {
        matches!(self, Self::Absent)
    }

    /// The value when present and non-null; absent and null both give `None`.
    #[must_use]
    pub fn as_option(&self) -> Option<&T> {
        match self {
            Self::Value(value) => Some(value),
            Self::Absent | Self::Null => None,
        }
    }

    /// Consuming form of [`Maybe::as_option`].
    #[must_use]
    pub fn into_option(self) -> Option<T> {
        match self {
            Self::Value(value) => Some(value),
            Self::Absent | Self::Null => None,
        }
    }

    /// Wraps an `Option`: `Some` becomes a value, `None` becomes an explicit `null`.
    #[must_use]
    pub fn from_option(value: Option<T>) -> Self {
        value.map_or(Self::Null, Self::Value)
    }
}

impl<T> From<T> for Maybe<T> {
    fn from(value: T) -> Self {
        Self::Value(value)
    }
}

impl<T: Serialize> Serialize for Maybe<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            // `Absent` is normally skipped by the field attribute; a bare serialisation is null.
            Self::Absent | Self::Null => serializer.serialize_none(),
            Self::Value(value) => value.serialize(serializer),
        }
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Maybe<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(Self::from_option(Option::<T>::deserialize(deserializer)?))
    }
}

/// `deserialize_with` for a property that may be ABSENT but not null: a present value must decode
/// as `T`, so an explicit `null` is rejected exactly as Pydantic rejects it (a missing key falls
/// back to the field's `default`, which is `None`).
pub(crate) fn present<'de, T, D>(deserializer: D) -> Result<Option<T>, D::Error>
where
    T: Deserialize<'de>,
    D: Deserializer<'de>,
{
    T::deserialize(deserializer).map(Some)
}

/// `deserialize_with` for a property that must be PRESENT but may be null: serde would otherwise
/// treat a missing `Option` key as `None`, which Pydantic does not allow for a required field.
pub(crate) fn nullable<'de, T, D>(deserializer: D) -> Result<Option<T>, D::Error>
where
    T: Deserialize<'de>,
    D: Deserializer<'de>,
{
    Option::<T>::deserialize(deserializer)
}

/// Defines a closed string enum from the wire values Vibe declares, plus an `Unrecognized(String)`
/// variant that preserves any other value (a newer Vibe may add values).
macro_rules! string_enum {
    (
        $(#[$meta:meta])*
        $name:ident {
            $( $(#[$vmeta:meta])* $variant:ident = $wire:literal ),+ $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash)]
        pub enum $name {
            $( $(#[$vmeta])* $variant, )+
            /// A value this generated version does not know; kept verbatim.
            Unrecognized(String),
        }

        impl $name {
            /// The exact wire string of this value.
            #[must_use]
            pub fn as_str(&self) -> &str {
                match self {
                    $( Self::$variant => $wire, )+
                    Self::Unrecognized(raw) => raw,
                }
            }
        }

        impl ::serde::Serialize for $name {
            fn serialize<S: ::serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(self.as_str())
            }
        }

        impl<'de> ::serde::Deserialize<'de> for $name {
            fn deserialize<D: ::serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let raw = <String as ::serde::Deserialize>::deserialize(deserializer)?;
                Ok(match raw.as_str() {
                    $( $wire => Self::$variant, )+
                    _ => Self::Unrecognized(raw),
                })
            }
        }
    };
}

/// Defines a discriminated union. Decoding reads the tag property first: a known tag decodes the
/// matching struct (and fails if that struct is malformed); an unknown or missing tag is kept as
/// raw JSON in `Unrecognized`. Encoding writes the member struct, which carries its own tag field.
macro_rules! tagged_union {
    (
        $(#[$meta:meta])*
        $name:ident, tag = $tag:literal {
            $( $(#[$vmeta:meta])* $variant:ident ($wire:literal) => $ty:ty ),+ $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq)]
        pub enum $name {
            $( $(#[$vmeta])* $variant($ty), )+
            /// A member whose tag this generated version does not know; the raw JSON is kept.
            Unrecognized(::serde_json::Value),
        }

        impl ::serde::Serialize for $name {
            fn serialize<S: ::serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                match self {
                    $( Self::$variant(inner) => ::serde::Serialize::serialize(inner, serializer), )+
                    Self::Unrecognized(raw) => ::serde::Serialize::serialize(raw, serializer),
                }
            }
        }

        impl<'de> ::serde::Deserialize<'de> for $name {
            fn deserialize<D: ::serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let value = <::serde_json::Value as ::serde::Deserialize>::deserialize(deserializer)?;
                let tag = value.get($tag).and_then(::serde_json::Value::as_str).map(str::to_owned);
                match tag.as_deref() {
                    $(
                        Some($wire) => ::serde_json::from_value(value)
                            .map(Self::$variant)
                            .map_err(<D::Error as ::serde::de::Error>::custom),
                    )+
                    _ => Ok(Self::Unrecognized(value)),
                }
            }
        }
    };
}
