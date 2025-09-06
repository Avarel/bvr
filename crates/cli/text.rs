use std::borrow::Cow;
use unicode_width::UnicodeWidthChar;

fn needs_normalization(input: &str) -> bool {
    let mut needs_normalization = false;

    for ch in input.chars() {
        match ch {
            '\t' => {
                needs_normalization = true;
                break;
            }
            ch => {
                let char_width = ch.width().unwrap_or(0);
                if char_width != 1 {
                    needs_normalization = true;
                    break;
                }
            }
        }
    }

    needs_normalization
}

const REPLACEMENT: char = '\u{FFFD}';

fn extend_with_normalized_chars(res: &mut String, input: &str) {
    for ch in input.chars() {
        match ch {
            '\t' => {
                // Replace tab with 4 spaces
                res.push_str("    ");
            }
            ch => {
                let char_width = ch.width().unwrap_or(0);
                match char_width {
                    0 => {
                        // Zero-width character - skip it
                        continue;
                    }
                    1 => {
                        // Single-width character - keep as is
                        res.push(ch);
                    }
                    _ => {
                        // Multi-width character - replace with replacement characters
                        for _ in 0..char_width {
                            res.push(REPLACEMENT);
                        }
                    }
                }
            }
        }
    }
}

/// Normalizes a string so that its visual width matches its character count.
///
/// This function takes a string reference and returns a `Cow<str>` such that:
/// - If the string contains only single-width characters (no tabs, no wide chars, no zero-width chars), returns the original string
/// - Otherwise, builds a new string where:
///   - Tabs are replaced with 4 spaces
///   - Non-single-column characters are replaced with replacement characters ('�')
///   - Zero-width characters are removed
///   - The resulting string's character count equals the original string's visual width
///
/// # Arguments
///
/// * `input` - The input string to normalize
///
/// # Returns
///
/// A `Cow<str>` containing either the original string (if no changes needed) or a new normalized string
///
/// # Examples
///
/// ```
/// use bvr_core::text::normalize_width;
///
/// // String with only single-width chars - returns borrowed
/// let simple = "hello";
/// assert_eq!(normalize_width(simple), "hello");
///
/// // String with tabs - returns owned with tabs replaced
/// let with_tabs = "hello\tworld";
/// assert_eq!(normalize_width(with_tabs), "hello    world");
///
/// // String with wide characters - returns owned with replacements
/// let with_wide = "hello世界";
/// assert_eq!(normalize_width(with_wide), "hello����");
/// ```
pub fn lossy_normalize_width(v: &[u8]) -> Cow<'_, str> {
    let mut iter = v.utf8_chunks();

    let first_valid = if let Some(chunk) = iter.next() {
        let valid = chunk.valid();
        if chunk.invalid().is_empty() && !needs_normalization(valid) {
            debug_assert_eq!(valid.len(), v.len());
            return Cow::Borrowed(valid);
        }
        chunk
    } else {
        return Cow::Borrowed("");
    };

    // Need to normalize - build a new string
    let mut res = String::with_capacity(v.len());

    extend_with_normalized_chars(&mut res, first_valid.valid());
    if !first_valid.invalid().is_empty() {
        res.push(REPLACEMENT);
    }

    for chunk in iter {
        extend_with_normalized_chars(&mut res, chunk.valid());
        if !chunk.invalid().is_empty() {
            res.push(REPLACEMENT);
        }
    }

    Cow::Owned(res)
}

#[cfg(test)]
mod tests {
    use unicode_width::UnicodeWidthStr;
    use super::*;

    #[test]
    fn test_simple_ascii() {
        let input = b"hello world";
        let result = lossy_normalize_width(input);
        assert_eq!(result, "hello world");
        // Should be borrowed since no changes needed
        assert!(matches!(result, Cow::Borrowed(_)));
    }

    #[test]
    fn test_with_tabs() {
        let input = b"hello\tworld";
        let result = lossy_normalize_width(input);
        assert_eq!(result, "hello    world");
        // Should be owned since tabs were replaced
        assert!(matches!(result, Cow::Owned(_)));
    }

    #[test]
    fn test_with_wide_characters() {
        let input = "hello世界".as_bytes();
        let result = lossy_normalize_width(input);
        assert_eq!(result, "hello����"); // Each wide char becomes 2 replacement chars
        // Should be owned since wide chars were replaced
        assert!(matches!(result, Cow::Owned(_)));
    }

    #[test]
    fn test_mixed_content() {
        let input = "a\t世b".as_bytes();
        let result = lossy_normalize_width(input);
        assert_eq!(result, "a    ��b"); // 世 becomes 2 replacement chars
        // Should be owned since tabs and wide chars were replaced
        assert!(matches!(result, Cow::Owned(_)));
    }

    #[test]
    fn test_only_tabs() {
        let input = b"\t\t";
        let result = lossy_normalize_width(input);
        assert_eq!(result, "        ");
        // Should be owned since tabs were replaced
        assert!(matches!(result, Cow::Owned(_)));
    }

    #[test]
    fn test_empty_string() {
        let input = b"";
        let result = lossy_normalize_width(input);
        assert_eq!(result, "");
        assert!(matches!(result, Cow::Borrowed(_)));
    }

    #[test]
    fn test_zero_width_characters() {
        // Test with combining characters (zero width)
        let input = "a\u{0300}b".as_bytes(); // 'a' with combining grave accent + 'b'
        let result = lossy_normalize_width(input);
        assert_eq!(result, "ab"); // Zero-width combining character should be removed
        assert!(matches!(result, Cow::Owned(_)));
    }

    #[test]
    fn test_emoji() {
        let input = "hello👋world".as_bytes();
        let result = lossy_normalize_width(input);
        assert_eq!(result, "hello��world"); // 👋 becomes 2 replacement chars
        assert!(matches!(result, Cow::Owned(_)));
    }

    #[test]
    fn test_width_calculation() {
        // Verify our width calculations are correct
        assert_eq!("hello".width(), 5);
        assert_eq!("hello\t".width(), 6); // tab counts as 1 in width calculation
        assert_eq!("hello世".width(), 7); // 世 is 2-width
        assert_eq!("👋".width(), 2); // emoji is 2-width
    }
}
