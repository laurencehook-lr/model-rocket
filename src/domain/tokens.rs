use std::{error::Error, fmt, num::NonZeroU32};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct OutputTokenLimit(NonZeroU32);

impl OutputTokenLimit {
    /// Creates a non-zero output token limit.
    ///
    /// # Errors
    ///
    /// Returns an error when `value` is zero.
    pub fn new(value: u32) -> Result<Self, OutputTokenLimitError> {
        NonZeroU32::new(value)
            .map(Self)
            .ok_or(OutputTokenLimitError)
    }

    #[must_use]
    pub const fn get(self) -> u32 {
        self.0.get()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OutputTokenLimitError;

impl fmt::Display for OutputTokenLimitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("output token limit must be greater than zero")
    }
}

impl Error for OutputTokenLimitError {}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct TokenCount(u64);

impl TokenCount {
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TokenUsage {
    input: TokenCount,
    output: TokenCount,
}

impl TokenUsage {
    #[must_use]
    pub const fn new(input: TokenCount, output: TokenCount) -> Self {
        Self { input, output }
    }

    #[must_use]
    pub const fn input(self) -> TokenCount {
        self.input
    }

    #[must_use]
    pub const fn output(self) -> TokenCount {
        self.output
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompletionCause {
    EndTurn,
    OutputLimit,
}

#[cfg(test)]
mod tests {
    use super::OutputTokenLimit;

    #[test]
    fn output_limit_rejects_zero() -> Result<(), Box<dyn std::error::Error>> {
        assert!(OutputTokenLimit::new(0).is_err());
        assert_eq!(OutputTokenLimit::new(1)?.get(), 1);
        Ok(())
    }
}
