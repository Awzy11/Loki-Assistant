/// Fixed-size chunking with overlap — matches Muninn's pattern (1000 chars / 150 overlap),
/// which its own notes describe as "not architecturally significant enough to tune." No
/// sentence/heading-aware splitting; good enough for both vault notes and chat messages,
/// which are usually well under one chunk anyway.
const CHUNK_CHARS: usize = 1000;
const CHUNK_OVERLAP: usize = 150;

pub fn chunk_text(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() {
        return Vec::new();
    }
    if chars.len() <= CHUNK_CHARS {
        return vec![text.to_string()];
    }

    let mut chunks = Vec::new();
    let mut start = 0usize;
    let step = CHUNK_CHARS - CHUNK_OVERLAP;
    while start < chars.len() {
        let end = (start + CHUNK_CHARS).min(chars.len());
        chunks.push(chars[start..end].iter().collect());
        if end == chars.len() {
            break;
        }
        start += step;
    }
    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_text_is_one_chunk() {
        assert_eq!(chunk_text("hello world"), vec!["hello world".to_string()]);
    }

    #[test]
    fn empty_text_has_no_chunks() {
        assert!(chunk_text("").is_empty());
    }

    #[test]
    fn long_text_overlaps() {
        let text = "a".repeat(2500);
        let chunks = chunk_text(&text);
        assert!(chunks.len() >= 3);
        assert!(chunks.iter().all(|c| c.chars().count() <= CHUNK_CHARS));
    }
}
