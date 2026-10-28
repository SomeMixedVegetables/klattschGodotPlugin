// English text to ARPABET lookup via the CMU Pronouncing Dictionary.
//
// The dictionary is loaded at runtime from a cmudict text file (e.g.
// cmudict-0.7b, which is Latin-1 encoded, or the UTF-8 cmudict.dict):
//
//   let dict = CmuDict::load("data/cmudict/cmudict-0.7b")?;
//   dict.pronounce("hello")    // HH AH L OW (OW stressed)
//   dict.pronounce("computer") // K AH M P Y UW T ER
//
// Parsing rules (mirrored by tools/cmu-shim.mjs for parity tests):
//   - lines are split on \n (a trailing \r is dropped)
//   - lines starting with ';;;' are skipped
//   - the first space/tab-delimited field is the word, lowercased
//   - the remaining fields are the phones, up to a field starting with '#'
//   - lines with no phones are skipped; the first entry for a word wins
// Alternate pronunciations keep their suffix ("hello(1)") as separate words.
//
// Custom word lists (character names, invented words, corrections) use the
// same file format and are layered over the CMU dictionary with a Lexicon:
//
//   let cmu = Arc::new(CmuDict::load("data/cmudict/cmudict-0.7b")?);
//   let mut lex = Lexicon::new();
//   lex.push_layer(Arc::new(CmuDict::load("my-words.txt")?)); // checked first
//   lex.push_layer(cmu);                                      // then CMU
//   lex.add_word("Zyx", "Z IH1 K S")?;                        // checked before both

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use crate::error::{Error, Result};
use crate::js;
use crate::Phone;

const VOWELS: [&str; 15] = [
    "AA", "AE", "AH", "AO", "AW", "AY", "EH", "ER", "EY", "IH", "IY", "OW", "OY", "UH", "UW",
];

#[derive(Clone, Debug, Default)]
pub struct CmuDict {
    entries: HashMap<String, String>,
}

// One word of `pronounce_text`: `phones` is `None` if the word is not in the
// dictionary.
#[derive(Clone, Debug, PartialEq)]
pub struct WordPronunciation {
    pub word: String,
    pub phones: Option<Vec<Phone>>,
}

fn parse_phone(raw: &str) -> Option<Phone> {
    let (code, stressed) = match raw.as_bytes().last() {
        Some(d @ (b'0' | b'1' | b'2')) => (&raw[..raw.len() - 1], *d == b'1'),
        _ => (raw, false),
    };
    if !VOWELS.contains(&code) && code.encode_utf16().count() > 2 {
        return None;
    }
    Some(Phone { code: code.into(), stressed })
}

impl CmuDict {
    pub fn parse(text: &str) -> Self {
        let mut entries = HashMap::new();
        for line in text.split('\n') {
            let line = line.strip_suffix('\r').unwrap_or(line);
            if line.starts_with(";;;") {
                continue;
            }
            let mut fields = line.split([' ', '\t']).filter(|f| !f.is_empty());
            let Some(word) = fields.next() else { continue };
            let phones: Vec<&str> = fields.take_while(|f| !f.starts_with('#')).collect();
            if phones.is_empty() {
                continue;
            }
            entries.entry(word.to_lowercase()).or_insert_with(|| phones.join(" "));
        }
        Self { entries }
    }

    // UTF-8 text is decoded as such; anything else as Latin-1.
    pub fn from_bytes(bytes: &[u8]) -> Self {
        match std::str::from_utf8(bytes) {
            Ok(text) => Self::parse(text),
            Err(_) => Self::parse(&bytes.iter().map(|&b| b as char).collect::<String>()),
        }
    }

    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let bytes = std::fs::read(path)
            .map_err(|e| Error::new(format!("cannot read dictionary {}: {e}", path.display())))?;
        Ok(Self::from_bytes(&bytes))
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    // The raw phone string for a word, e.g. "HH AH0 L OW1".
    pub fn entry(&self, word: &str) -> Option<&str> {
        self.entries.get(&word.to_lowercase()).map(String::as_str)
    }

    pub fn pronounce(&self, word: &str) -> Option<Vec<Phone>> {
        let entry = self.entry(word)?;
        let phones: Vec<Phone> = entry.split(' ').filter_map(parse_phone).collect();
        (!phones.is_empty()).then_some(phones)
    }

    pub fn pronounce_text(&self, text: &str) -> Vec<WordPronunciation> {
        text.split(js::is_space)
            .filter(|w| !w.is_empty())
            .filter_map(|word| {
                let clean: String =
                    word.chars().filter(|c| c.is_ascii_alphabetic() || *c == '\'' || *c == '-').collect();
                if clean.is_empty() {
                    return None;
                }
                let phones = self.pronounce(&clean);
                Some(WordPronunciation { word: clean, phones })
            })
            .collect()
    }

    pub fn text_to_phonemes(&self, text: &str) -> Vec<Phone> {
        self.pronounce_text(text).into_iter().filter_map(|w| w.phones).flatten().collect()
    }

    pub fn has_word(&self, word: &str) -> bool {
        self.entries.contains_key(&word.to_lowercase())
    }

    // Add or replace a word. `phones` is ARPABET as in the file format, with
    // optional stress digits ("Z IH1 K S"); a bad or empty list is an error.
    pub fn insert(&mut self, word: &str, phones: &str) -> Result<()> {
        let word = word.trim();
        if word.is_empty() || word.contains(js::is_space) {
            return Err(Error::new(format!("not a single word: {word:?}")));
        }
        let fields: Vec<&str> = phones.split(js::is_space).filter(|f| !f.is_empty()).collect();
        if fields.is_empty() {
            return Err(Error::new(format!("no phones for {word:?}")));
        }
        if let Some(bad) = fields.iter().find(|f| parse_phone(f).is_none()) {
            return Err(Error::new(format!("bad phone {bad:?} for {word:?}")));
        }
        self.entries.insert(word.to_lowercase(), fields.join(" "));
        Ok(())
    }

    // Remove a word; returns whether it was there.
    pub fn remove(&mut self, word: &str) -> bool {
        self.entries.remove(&word.to_lowercase()).is_some()
    }
}

// Anything that can pronounce a single word: a CmuDict or a Lexicon.
pub trait Pronounce {
    fn pronounce(&self, word: &str) -> Option<Vec<Phone>>;
}

impl Pronounce for CmuDict {
    fn pronounce(&self, word: &str) -> Option<Vec<Phone>> {
        CmuDict::pronounce(self, word)
    }
}

// Word lists checked in order: the lexicon's own words (add_word), then each
// layer in the order pushed. The first list that has a word decides its
// pronunciation. Layers are shared, so cloning a lexicon to give a character
// its own words does not copy the CMU dictionary.
#[derive(Clone, Debug, Default)]
pub struct Lexicon {
    words: CmuDict,
    layers: Vec<Arc<CmuDict>>,
}

impl Lexicon {
    pub fn new() -> Self {
        Self::default()
    }

    // Add a word list below the ones already added.
    pub fn push_layer(&mut self, dict: Arc<CmuDict>) {
        self.layers.push(dict);
    }

    pub fn layers(&self) -> &[Arc<CmuDict>] {
        &self.layers
    }

    // The lexicon's own words, checked before every layer.
    pub fn words(&self) -> &CmuDict {
        &self.words
    }

    // Add or replace one of the lexicon's own words (see CmuDict::insert).
    pub fn add_word(&mut self, word: &str, phones: &str) -> Result<()> {
        self.words.insert(word, phones)
    }

    pub fn remove_word(&mut self, word: &str) -> bool {
        self.words.remove(word)
    }

    fn lists(&self) -> impl Iterator<Item = &CmuDict> {
        std::iter::once(&self.words).chain(self.layers.iter().map(|d| &**d))
    }

    // The raw phone string from the first list that has the word.
    pub fn entry(&self, word: &str) -> Option<&str> {
        self.lists().find_map(|d| d.entry(word))
    }

    pub fn has_word(&self, word: &str) -> bool {
        self.lists().any(|d| d.has_word(word))
    }
}

impl Pronounce for Lexicon {
    fn pronounce(&self, word: &str) -> Option<Vec<Phone>> {
        self.lists().find(|d| d.has_word(word)).and_then(|d| d.pronounce(word))
    }
}
