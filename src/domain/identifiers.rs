use std::{fmt, sync::Arc};

macro_rules! string_identifier {
    ($name:ident) => {
        #[derive(Clone, PartialEq, Eq, Hash)]
        pub struct $name(Arc<str>);

        impl $name {
            #[must_use]
            pub fn new(value: impl Into<Arc<str>>) -> Self {
                Self(value.into())
            }

            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter
                    .debug_tuple(stringify!($name))
                    .field(&self.0)
                    .finish()
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self::new(value)
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self::new(value)
            }
        }
    };
}

string_identifier!(RequestedModelId);
string_identifier!(ClaudeSessionId);
string_identifier!(ToolUseId);
string_identifier!(ClaudeToolName);

#[cfg(test)]
mod tests {
    use super::ClaudeSessionId;

    #[test]
    fn identifiers_own_borrowed_input() {
        let source = String::from("session-42");
        let identifier = ClaudeSessionId::from(source.as_str());
        drop(source);
        assert_eq!(identifier.as_str(), "session-42");
    }
}
