pub(crate) const MIN_TOKEN_BYTES: usize = 32;
pub(crate) const MAX_TOKEN_BYTES: usize = 256;

pub(crate) fn has_safe_token_syntax(token: &str) -> bool {
    (MIN_TOKEN_BYTES..=MAX_TOKEN_BYTES).contains(&token.len())
        && token.bytes().all(is_safe_token_byte)
}

pub(crate) fn is_safe_token_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~')
}

pub(crate) fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_bounded_url_safe_tokens() {
        assert!(has_safe_token_syntax(&"a".repeat(MIN_TOKEN_BYTES)));
        assert!(has_safe_token_syntax(&format!("{}-_.~", "b".repeat(32))));
        assert!(!has_safe_token_syntax("short"));
        assert!(!has_safe_token_syntax(&"a".repeat(MAX_TOKEN_BYTES + 1)));
        assert!(!has_safe_token_syntax(&format!("{} token", "a".repeat(32))));
    }

    #[test]
    fn compares_complete_equal_length_values() {
        assert!(constant_time_eq(b"Bearer secret", b"Bearer secret"));
        assert!(!constant_time_eq(b"Bearer secret", b"Bearer other!"));
        assert!(!constant_time_eq(b"Bearer secret", b"Bearer secret-longer"));
    }
}
