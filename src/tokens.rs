//! Token 计数估算（基于字符和词数的简单启发式方法）

/// 粗略估算文本的 token 数。
/// 中文：1 个字符 ≈ 1.3 个 token（粗略）
/// 英文：1 个单词 ≈ 1 个 token（粗略）
pub fn estimate_tokens(text: &str) -> usize {
    let mut tokens = 0;
    let mut in_word = false;

    for ch in text.chars() {
        if ch.is_whitespace() || ch.is_ascii_punctuation() {
            if in_word {
                tokens += 1;
                in_word = false;
            }
        } else if ch.is_ascii_alphanumeric() {
            if !in_word {
                tokens += 1;
                in_word = true;
            }
        } else {
            // 中文、日文等非 ASCII，每个字符算 1.3 个 token
            tokens += (ch.len_utf8() as f32 * 1.3) as usize;
            if in_word {
                in_word = false;
            }
        }
    }

    if in_word {
        tokens += 1;
    }

    tokens.max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimate_english() {
        let text = "Hello world";
        assert!(estimate_tokens(text) >= 2);
    }

    #[test]
    fn estimate_chinese() {
        let text = "你好世界";
        // 4 个字符，每个 1.3 token，所以大约 5 个 token
        assert!(estimate_tokens(text) >= 4);
    }
}
