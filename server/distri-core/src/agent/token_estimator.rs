use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EstimationMethod {
    Average,
    Words,
    Chars,
    Max,
    Min,
}

impl Default for EstimationMethod {
    fn default() -> Self {
        EstimationMethod::Max
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TokenEstimate {
    pub estimated_tokens: usize,
    pub word_count: usize,
    pub char_count: usize,
    pub tokens_from_words: usize,
    pub tokens_from_chars: usize,
    pub method_used: EstimationMethod,
}

/// Token estimation utility
pub struct TokenEstimator;

impl TokenEstimator {
    /// Estimate token count from text using different methods
    ///
    /// Methods:
    /// - "average": Average of word and char estimates
    /// - "words": Word count divided by 0.75
    /// - "chars": Char count divided by 4
    /// - "max": Maximum of word and char estimates (default)
    /// - "min": Minimum of word and char estimates
    pub fn estimate_tokens(text: &str, method: EstimationMethod) -> Result<TokenEstimate, String> {
        let word_count = text.split_whitespace().count();
        let char_count = text.len();

        let tokens_from_words = ((word_count as f64) / 0.75) as usize;
        let tokens_from_chars = ((char_count as f64) / 4.0) as usize;

        let estimated_tokens = match method {
            EstimationMethod::Average => (tokens_from_words + tokens_from_chars) / 2,
            EstimationMethod::Words => tokens_from_words,
            EstimationMethod::Chars => tokens_from_chars,
            EstimationMethod::Max => tokens_from_words.max(tokens_from_chars),
            EstimationMethod::Min => tokens_from_words.min(tokens_from_chars),
        };

        Ok(TokenEstimate {
            estimated_tokens,
            word_count,
            char_count,
            tokens_from_words,
            tokens_from_chars,
            method_used: method,
        })
    }

    /// Convenience method using default max method
    pub fn estimate_tokens_max(text: &str) -> usize {
        Self::estimate_tokens(text, EstimationMethod::Max)
            .map(|est| est.estimated_tokens)
            .unwrap_or(0)
    }

    /// Convenience method using words method (typically more accurate for code)
    pub fn estimate_tokens_words(text: &str) -> usize {
        Self::estimate_tokens(text, EstimationMethod::Words)
            .map(|est| est.estimated_tokens)
            .unwrap_or(0)
    }

    /// Convenience method using chars method (typically more accurate for prose)
    pub fn estimate_tokens_chars(text: &str) -> usize {
        Self::estimate_tokens(text, EstimationMethod::Chars)
            .map(|est| est.estimated_tokens)
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_token_estimation_words() {
        let text = "Hello world this is a test";
        let estimate = TokenEstimator::estimate_tokens(text, EstimationMethod::Words).unwrap();

        assert_eq!(estimate.word_count, 6);
        assert_eq!(estimate.tokens_from_words, 8); // 6 / 0.75 = 8
    }

    #[test]
    fn test_token_estimation_chars() {
        let text = "Hello world"; // 11 chars
        let estimate = TokenEstimator::estimate_tokens(text, EstimationMethod::Chars).unwrap();

        assert_eq!(estimate.char_count, 11);
        assert_eq!(estimate.tokens_from_chars, 2); // 11 / 4 = 2.75 -> 2
    }

    #[test]
    fn test_token_estimation_max() {
        let text = "Hello world this is a test"; // 6 words, 26 chars
        let estimate = TokenEstimator::estimate_tokens(text, EstimationMethod::Max).unwrap();

        let tokens_from_words = 8; // 6 / 0.75 = 8
        let tokens_from_chars = 6; // 26 / 4 = 6.5 -> 6

        assert_eq!(
            estimate.estimated_tokens,
            tokens_from_words.max(tokens_from_chars)
        ); // 8
    }

    #[test]
    fn test_token_estimation_average() {
        let text = "Hello world";
        let estimate = TokenEstimator::estimate_tokens(text, EstimationMethod::Average).unwrap();

        let tokens_from_words = 2; // 2 / 0.75 = 2.67 -> 2
        let tokens_from_chars = 2; // 11 / 4 = 2.75 -> 2

        assert_eq!(
            estimate.estimated_tokens,
            (tokens_from_words + tokens_from_chars) / 2
        );
    }
}
