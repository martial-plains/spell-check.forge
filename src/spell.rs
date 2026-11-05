// SPDX-License-Identifier: MIT OR Apache-2.0
//! The spell checking itself: finding words, looking them up, suggesting
//! corrections. No Forge API here, so it's tested natively.

use std::collections::HashSet;

/// Known words, compared case-insensitively.
#[derive(Default)]
pub struct Dictionary {
    words: HashSet<String>,
}

const LETTERS: &str = "abcdefghijklmnopqrstuvwxyz";

impl Dictionary {
    /// A word list: one word per line (like /usr/share/dict/words).
    pub fn from_list(text: &str) -> Self {
        let mut d = Self::default();
        for w in text.lines().map(str::trim).filter(|w| !w.is_empty()) {
            d.add(w);
        }
        d
    }

    pub fn len(&self) -> usize {
        self.words.len()
    }

    pub fn is_empty(&self) -> bool {
        self.words.is_empty()
    }

    pub fn add(&mut self, word: &str) {
        self.words.insert(word.to_lowercase());
    }

    pub fn knows(&self, word: &str) -> bool {
        let w = word.to_lowercase();
        if self.words.contains(&w) {
            return true;
        }
        // Possessives: "Forge's" is fine if "Forge" is.
        match w.strip_suffix("'s").or_else(|| w.strip_suffix('\'')) {
            Some(stem) if !stem.is_empty() => self.words.contains(stem),
            _ => false,
        }
    }

    /// Up to `max` known words one or two edits away (deleting,
    /// swapping, changing or adding a letter), closest first, in the
    /// word's capitalization.
    pub fn suggestions(&self, word: &str, max: usize) -> Vec<String> {
        let lower = word.to_lowercase();
        let mut found: Vec<String> = edits(&lower)
            .into_iter()
            .filter(|c| self.words.contains(c))
            .collect();
        if found.is_empty() {
            let mut seen = HashSet::new();
            for e in edits(&lower) {
                for c in edits(&e) {
                    if self.words.contains(&c) && seen.insert(c.clone()) {
                        found.push(c);
                    }
                }
                if found.len() >= max * 4 {
                    break;
                }
            }
        }
        found.sort();
        found.dedup();
        // Words of the same length first (typos are usually swaps and
        // wrong letters), then alphabetically.
        found.sort_by_key(|c| c.chars().count().abs_diff(lower.chars().count()));
        found.truncate(max);
        found.into_iter().map(|c| match_case(word, &c)).collect()
    }
}

/// Every string one edit from `w`.
fn edits(w: &str) -> Vec<String> {
    let chars: Vec<char> = w.chars().collect();
    let n = chars.len();
    let s = |v: &[char]| v.iter().collect::<String>();
    let mut out = Vec::with_capacity(54 * n + 25);
    for i in 0..n {
        let mut v = chars.clone();
        v.remove(i);
        out.push(s(&v));
        if i + 1 < n {
            let mut v = chars.clone();
            v.swap(i, i + 1);
            out.push(s(&v));
        }
        for c in LETTERS.chars() {
            if c != chars[i] {
                let mut v = chars.clone();
                v[i] = c;
                out.push(s(&v));
            }
        }
    }
    for i in 0..=n {
        for c in LETTERS.chars() {
            let mut v = chars.clone();
            v.insert(i, c);
            out.push(s(&v));
        }
    }
    out
}

/// "Teh" -> "The", "TEH" -> "THE".
fn match_case(original: &str, word: &str) -> String {
    let mut o = original.chars();
    let first_upper = o.next().is_some_and(char::is_uppercase);
    if first_upper && original.chars().count() > 1 && original.chars().all(|c| !c.is_lowercase()) {
        return word.to_uppercase();
    }
    if first_upper {
        let mut c = word.chars();
        return c
            .next()
            .map(|f| f.to_uppercase().chain(c).collect())
            .unwrap_or_default();
    }
    word.to_string()
}

/// A word in the text: 0-based line, char columns `start..end`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Word {
    pub line: u32,
    pub start: u32,
    pub end: u32,
    pub text: String,
}

/// Words worth checking: runs of letters (with apostrophes inside),
/// skipping anything that looks like code or an address: words touching
/// digits or `_`, camelCase and ALLCAPS, single letters, and whole chunks
/// with `://`, `@`, `/` or `.` between letters (URLs, emails, paths,
/// file names).
pub fn words(text: &str) -> Vec<Word> {
    let mut out = Vec::new();
    for (li, line) in text.split('\n').enumerate() {
        let chars: Vec<char> = line.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            if chars[i].is_whitespace() {
                i += 1;
                continue;
            }
            let chunk_start = i;
            while i < chars.len() && !chars[i].is_whitespace() {
                i += 1;
            }
            let chunk = &chars[chunk_start..i];
            if looks_like_address(chunk) {
                continue;
            }
            let mut j = 0;
            while j < chunk.len() {
                if !chunk[j].is_alphabetic() {
                    j += 1;
                    continue;
                }
                let s = j;
                while j < chunk.len()
                    && (chunk[j].is_alphabetic()
                        || (chunk[j] == '\''
                            && j + 1 < chunk.len()
                            && chunk[j + 1].is_alphabetic()))
                {
                    j += 1;
                }
                let w: String = chunk[s..j].iter().collect();
                let touches = |k: Option<&char>| k.is_some_and(|c| c.is_ascii_digit() || *c == '_');
                let code_like =
                    touches(s.checked_sub(1).and_then(|k| chunk.get(k))) || touches(chunk.get(j));
                let inner_caps = w.chars().skip(1).any(char::is_uppercase);
                if !code_like && !inner_caps && w.chars().count() > 1 {
                    out.push(Word {
                        line: li as u32,
                        start: (chunk_start + s) as u32,
                        end: (chunk_start + j) as u32,
                        text: w,
                    });
                }
            }
        }
    }
    out
}

fn looks_like_address(chunk: &[char]) -> bool {
    let s: String = chunk.iter().collect();
    if s.contains("://") || s.contains('@') {
        return true;
    }
    // A '/' or '.' with letters on both sides: path/to, file.rs, e.g.
    chunk
        .windows(3)
        .any(|w| (w[1] == '/' || w[1] == '.') && w[0].is_alphanumeric() && w[2].is_alphanumeric())
}

/// A region to check: (line, column) to (line, column), end exclusive.
pub type Region = ((u32, u32), (u32, u32));

/// Is the word inside one of `regions`?
pub fn inside(w: &Word, regions: &[Region]) -> bool {
    regions
        .iter()
        .any(|&(a, b)| (w.line, w.start) >= a && (w.line, w.end) <= b)
}

/// The misspelled words of `text`, only inside `regions` if given
/// (comments and strings in code).
pub fn misspelled(dict: &Dictionary, text: &str, regions: Option<&[Region]>) -> Vec<Word> {
    words(text)
        .into_iter()
        .filter(|w| regions.is_none_or(|r| inside(w, r)))
        .filter(|w| !dict.knows(&w.text))
        .collect()
}

/// The word at (line, column), if any.
pub fn word_at(text: &str, line: u32, col: u32) -> Option<Word> {
    words(text)
        .into_iter()
        .find(|w| w.line == line && w.start <= col && col <= w.end)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dict() -> Dictionary {
        Dictionary::from_list(
            "the\nquick\nbrown\nfox\njumps\nover\nlazy\ndog\nForge\nhello\nworld\nit\nit's\nsee\ncode\n",
        )
    }

    fn texts(ws: &[Word]) -> Vec<&str> {
        ws.iter().map(|w| w.text.as_str()).collect()
    }

    #[test]
    fn finds_words_and_skips_code_like_things() {
        let ws = words(
            "The quick brwon fox's\n  see https://x.org a@b.c src/main.rs foo_bar x2y camelCase NASA",
        );
        assert_eq!(
            texts(&ws),
            ["The", "quick", "brwon", "fox's", "see"],
            "NASA: all caps is skipped"
        );
        assert_eq!(
            ws[2],
            Word {
                line: 0,
                start: 10,
                end: 15,
                text: "brwon".into()
            }
        );
        assert_eq!((ws[4].line, ws[4].start), (1, 2));
        assert_eq!(
            texts(&words("don't 'quoted' end.")),
            ["don't", "quoted", "end"]
        );
    }

    #[test]
    fn checks_case_insensitively_with_possessives() {
        let d = dict();
        assert!(d.knows("The") && d.knows("forge") && d.knows("Forge's") && d.knows("it's"));
        assert!(!d.knows("brwon"));
        let bad = misspelled(&d, "The quick brwon fox jumsp over teh lazy dog", None);
        assert_eq!(texts(&bad), ["brwon", "jumsp", "teh"]);
    }

    #[test]
    fn only_inside_regions_when_given() {
        let d = dict();
        let text = "fn wrold() {} // hello wrold";
        let comment: Region = ((0, 14), (0, 28));
        assert_eq!(texts(&misspelled(&d, text, Some(&[comment]))), ["wrold"]);
        assert_eq!(misspelled(&d, text, Some(&[comment]))[0].start, 23);
        assert!(misspelled(&d, text, Some(&[])).is_empty());
    }

    #[test]
    fn suggests_close_words_in_the_same_case() {
        let d = dict();
        assert_eq!(d.suggestions("brwon", 5), ["brown"]);
        assert_eq!(d.suggestions("Teh", 5), ["The"]);
        assert_eq!(d.suggestions("WROLD", 5), ["WORLD"]);
        assert_eq!(d.suggestions("helo", 5), ["hello"]);
        assert_eq!(d.suggestions("jmups", 5), ["jumps"], "two edits away");
        assert!(d.suggestions("zzzzzzzz", 5).is_empty());
    }

    #[test]
    fn word_at_point() {
        let t = "one two\nthree";
        assert_eq!(word_at(t, 0, 5).map(|w| w.text), Some("two".into()));
        assert_eq!(
            word_at(t, 0, 7).map(|w| w.text),
            Some("two".into()),
            "just after it counts"
        );
        assert_eq!(word_at(t, 1, 0).map(|w| w.text), Some("three".into()));
    }
}
