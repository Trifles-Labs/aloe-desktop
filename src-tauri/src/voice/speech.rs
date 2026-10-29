//! Turning a streamed markdown reply into things worth saying out loud. A port of aloe-ui's
//! `lib/voice/speech_text.ts`, which does the same for the chat page's read-aloud; keep the two
//! in step.
//!
//! Each sentence is handed back as soon as it is complete, so Aloe starts talking while the rest
//! of the reply is still arriving. Code blocks and tables are never read out symbol by symbol:
//! each kind is announced once, so the listener knows to look at the screen.

use std::sync::LazyLock;

use regex::Regex;

const CODE_NOTICE: &str = "I've put the code on screen.";
const TABLE_NOTICE: &str = "There's a table on screen.";

/// A sentence ends at . ! ? or the Devanagari danda, followed by whitespace. A digit right before
/// the stop ("1. ", "v2. ") is a list marker or a version, not the end of a thought.
static SENTENCE_END: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"[^\d\s][.!?।]+["')\]]*\s"#).unwrap());

static TAGS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<[^>]+>").unwrap());
static IMAGES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"!\[([^\]]*)\]\([^)]*\)").unwrap());
static LINKS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[([^\]]+)\]\([^)]*\)").unwrap());
static URLS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"https?://\S+").unwrap());
static LINE_MARKERS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s{0,3}(#{1,6}|>+|[-*+]|\d+[.)])\s+").unwrap());
static EMPHASIS_MARKS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(\*\*|__|~~|`)").unwrap());
/// The JS original uses a lookahead for what follows; this captures it instead and puts it back.
static SINGLE_EMPHASIS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(^|\s)[*_]([^*_]+)[*_](\s|[.,!?]|$)").unwrap());
static DOLLARS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\$+").unwrap());
static SPACES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").unwrap());
static SPEAKABLE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[\p{L}\p{N}]").unwrap());
static FENCE_INFO: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[^\n]*\n?").unwrap());

/// Markdown and URLs stripped from one sentence or line, or "" if nothing speakable is left.
pub fn to_speakable(text: &str) -> String {
    let text = TAGS.replace_all(text, " ");
    let text = IMAGES.replace_all(&text, "$1");
    let text = LINKS.replace_all(&text, "$1");
    let text = URLS.replace_all(&text, "a link");
    let text = LINE_MARKERS.replace(&text, "");
    let text = EMPHASIS_MARKS.replace_all(&text, "");
    // Twice: the captured trailing character can't also start the next match ("*a* *b*").
    let text = SINGLE_EMPHASIS.replace_all(&text, "$1$2$3");
    let text = SINGLE_EMPHASIS.replace_all(&text, "$1$2$3");
    let text = DOLLARS.replace_all(&text, "");
    let text = SPACES.replace_all(&text, " ");
    let spoken = text.trim();
    if SPEAKABLE.is_match(spoken) {
        spoken.to_string()
    } else {
        String::new()
    }
}

fn is_table_line(line: &str) -> bool {
    line.trim_start().starts_with('|')
}

#[derive(Default)]
pub struct SpeechStream {
    buffer: String,
    in_code: bool,
    said_code: bool,
    said_table: bool,
}

impl SpeechStream {
    /// Adds a chunk of the reply; returns whatever is now ready to be spoken.
    pub fn push(&mut self, chunk: &str) -> Vec<String> {
        self.buffer.push_str(chunk);
        self.drain(false)
    }

    /// The reply is over: everything still held back.
    pub fn flush(&mut self) -> Vec<String> {
        self.drain(true)
    }

    fn drain(&mut self, last: bool) -> Vec<String> {
        let mut out = Vec::new();
        let say = |out: &mut Vec<String>, text: &str| {
            let spoken = to_speakable(text);
            if !spoken.is_empty() {
                out.push(spoken);
            }
        };

        while !self.buffer.is_empty() {
            if self.in_code {
                let Some(close) = self.buffer.find("```") else {
                    // Keep a possible half-arrived fence; the code itself is never read.
                    self.buffer = if last { String::new() } else { last_chars(&self.buffer, 2) };
                    break;
                };
                let rest = &self.buffer[close + 3..];
                self.buffer = FENCE_INFO.replace(rest, "").into_owned();
                self.in_code = false;
                continue;
            }

            let fence = self.buffer.find("```");
            let newline = self.buffer.find('\n');

            if let Some(fence) = fence.filter(|&f| newline.is_none_or(|n| f < n)) {
                for sentence in sentences_of(&self.buffer[..fence]) {
                    say(&mut out, &sentence);
                }
                self.buffer = self.buffer[fence + 3..].to_string();
                self.in_code = true;
                if !self.said_code {
                    self.said_code = true;
                    out.push(CODE_NOTICE.to_string());
                }
                continue;
            }

            if newline.is_some() || last {
                let end = newline.unwrap_or(self.buffer.len());
                let line = self.buffer[..end].to_string();
                self.buffer = self.buffer.get(end + 1..).unwrap_or_default().to_string();
                if is_table_line(&line) {
                    if !self.said_table {
                        self.said_table = true;
                        out.push(TABLE_NOTICE.to_string());
                    }
                } else {
                    for sentence in sentences_of(&line) {
                        say(&mut out, &sentence);
                    }
                }
                continue;
            }

            // A line still arriving: speak the sentences it has finished, hold the rest.
            if is_table_line(&self.buffer) || self.buffer.ends_with('`') {
                break;
            }
            let cut = SENTENCE_END.find_iter(&self.buffer).last().map_or(0, |m| m.end());
            if cut == 0 {
                break;
            }
            for sentence in sentences_of(&self.buffer[..cut]) {
                say(&mut out, &sentence);
            }
            self.buffer = self.buffer[cut..].to_string();
            break;
        }

        out
    }
}

fn sentences_of(text: &str) -> Vec<String> {
    let padded = format!("{text} ");
    let mut sentences = Vec::new();
    let mut start = 0;
    for m in SENTENCE_END.find_iter(&padded) {
        let end = m.end().min(text.len());
        if end > start {
            sentences.push(text[start..end].to_string());
        }
        start = end;
    }
    if start < text.len() {
        sentences.push(text[start..].to_string());
    }
    sentences
}

fn last_chars(text: &str, n: usize) -> String {
    let start = text.char_indices().rev().nth(n.saturating_sub(1)).map_or(0, |(i, _)| i);
    text[start..].to_string()
}

/// Splits a sentence longer than the speech endpoint takes, at word boundaries.
pub fn split_for_speech(sentence: &str, max_chars: usize) -> Vec<String> {
    if sentence.chars().count() <= max_chars {
        return vec![sentence.to_string()];
    }
    let mut parts = Vec::new();
    let mut current = String::new();
    for word in sentence.split_whitespace() {
        if !current.is_empty() && current.chars().count() + 1 + word.chars().count() > max_chars {
            parts.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push(' ');
        }
        // A single "word" longer than the cap (a long token) is cut where it has to be.
        let mut word = word;
        while word.chars().count() > max_chars {
            let cut = word.char_indices().nth(max_chars).map_or(word.len(), |(i, _)| i);
            parts.push(word[..cut].to_string());
            word = &word[cut..];
        }
        current.push_str(word);
    }
    if !current.is_empty() {
        parts.push(current);
    }
    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all(markdown: &str) -> Vec<String> {
        let mut stream = SpeechStream::default();
        let mut out = stream.push(markdown);
        out.extend(stream.flush());
        out
    }

    #[test]
    fn speaks_each_sentence_as_it_completes() {
        let mut stream = SpeechStream::default();
        assert!(stream.push("Hello there").is_empty());
        assert_eq!(stream.push(". How are"), vec!["Hello there."]);
        assert_eq!(stream.push(" you? Fine"), vec!["How are you?"]);
        assert_eq!(stream.flush(), vec!["Fine"]);
    }

    #[test]
    fn list_numbers_are_not_sentence_ends() {
        assert_eq!(all("1. First item\n2. Second item\n"), vec!["First item", "Second item"]);
    }

    #[test]
    fn code_and_tables_are_announced_once() {
        let reply = "Here you go:\n```rust\nfn main() {}\n```\nAnd a table.\n| a | b |\n|---|---|\n```js\nx\n```\nDone.";
        assert_eq!(all(reply), vec!["Here you go:", CODE_NOTICE, "And a table.", TABLE_NOTICE, "Done."]);
    }

    #[test]
    fn markdown_is_stripped() {
        assert_eq!(to_speakable("## **Bold** and *soft* and _also_ [a link](https://x.y) `code`"), "Bold and soft and also a link code");
        assert_eq!(to_speakable("See https://example.com now"), "See a link now");
        assert_eq!(to_speakable("---"), "");
    }

    #[test]
    fn long_sentences_split_on_words() {
        let parts = split_for_speech("aa bb cc dd", 5);
        assert_eq!(parts, vec!["aa bb", "cc dd"]);
        assert!(split_for_speech(&"x".repeat(12), 5).iter().all(|p| p.chars().count() <= 5));
    }
}
