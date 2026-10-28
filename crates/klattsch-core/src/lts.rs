// Letter-to-sound rules for English words not in the dictionary.
//
// The rules are those of Elovitz, Johnson, McHugh & Shore, "Automatic
// Translation of English Text to Phonetics by Means of Letter-to-Sound
// Rules", NRL Report 7948 (1976). The rule table (lts/nrl-rules.txt) and the
// matching algorithm follow Martin O'Leary's transcription,
// https://github.com/mewo2/elovitz (MIT license, see lts/LICENSE-elovitz.md).
//
// Each rule is `left[letters]right=/phonemes/`. At each position the first
// rule whose letters match there, and whose left and right contexts match
// around them, emits its phonemes and consumes its letters. Context symbols:
//   #  one or more vowels          :  zero or more consonants
//   ^  one consonant               *  one or more consonants
//   .  a voiced consonant          $  a consonant followed by e or i
//   +  e, i or y                   %  suffix: er e es ed ing ely
//   &  ch sh s c g z x j           @  th ch sh t s r d l z n j
//   (space) a word boundary

use std::collections::HashMap;
use std::sync::LazyLock;

use regex::Regex;
use unicode_normalization::UnicodeNormalization;

use crate::Phone;

const RULES_TEXT: &str = include_str!("../lts/nrl-rules.txt");

struct Rule {
    left: Regex,
    letters_right: Regex,
    letters_len: usize,
    phonemes: Vec<&'static str>,
}

fn context_pattern(p: &str) -> String {
    p.chars()
        .map(|c| match c {
            '^' => "[bcdfghjklmnpqrstvwxz]".to_string(),
            '#' => "[aeiouy]+".to_string(),
            '+' => "[eiy]".to_string(),
            ':' => "[bcdfghjklmnpqrstvwxz]*".to_string(),
            '*' => "[bcdfghjklmnpqrstvwxz]+".to_string(),
            '.' => "[bdvgjlmnrwz]".to_string(),
            '$' => "[bcdfghjklmnpqrstvwxz][ei]".to_string(),
            '%' => "(?:er|e|es|ed|ing|ely)".to_string(),
            '&' => "(?:ch|sh|[scgzxj])".to_string(),
            '@' => "(?:th|ch|sh|[tsrdlznj])".to_string(),
            c if c.is_ascii_lowercase() || c == ' ' || c == '\'' => c.to_string(),
            c => panic!("unexpected rule symbol {c:?}"),
        })
        .collect()
}

fn parse_rule(line: &'static str) -> Rule {
    let (left, rest) = line.split_once('[').expect("rule has [");
    let (letters, rest) = rest.split_once(']').expect("rule has ]");
    let (right, rest) = rest.split_once('=').expect("rule has =");
    let phonemes = rest.split('/').nth(1).unwrap_or("").split_whitespace().collect();
    Rule {
        left: Regex::new(&format!("{}$", context_pattern(left))).unwrap(),
        letters_right: Regex::new(&format!("^{}{}", context_pattern(letters), context_pattern(right))).unwrap(),
        letters_len: letters.len(),
        phonemes,
    }
}

// Rules grouped by their first letter, keeping table order within a group.
static RULES: LazyLock<HashMap<u8, Vec<Rule>>> = LazyLock::new(|| {
    let mut by_letter: HashMap<u8, Vec<Rule>> = HashMap::new();
    for line in RULES_TEXT.lines().filter(|l| !l.trim().is_empty()) {
        let rule = parse_rule(line);
        let first = line.split_once('[').unwrap().1.as_bytes()[0];
        by_letter.entry(first).or_default().push(rule);
    }
    by_letter
});

// The rules' phoneme codes, as written in the table (lowercase ARPABET plus
// `ax` for schwa, `nx` for ng and `wh`), for each letter of `word`. Letters
// are folded to ASCII (accents dropped); anything else is skipped.
pub fn rule_phonemes(word: &str) -> Vec<&'static str> {
    let folded: String = word.nfkd().filter(|c| c.is_ascii_alphabetic() || *c == '\'').collect();
    let text = format!(" {} ", folded.to_ascii_lowercase());
    let mut out = Vec::new();
    let mut pos = 0;
    while pos < text.len() {
        let rule = RULES
            .get(&text.as_bytes()[pos])
            .and_then(|rules| {
                rules.iter().find(|r| r.left.is_match(&text[..pos]) && r.letters_right.is_match(&text[pos..]))
            });
        match rule {
            Some(r) => {
                out.extend(r.phonemes.iter().copied());
                pos += r.letters_len;
            }
            None => pos += 1,
        }
    }
    out
}

const VOWELS: [&str; 16] = [
    "aa", "ae", "ah", "ao", "aw", "ax", "ay", "eh", "er", "ey", "ih", "iy", "ow", "oy", "uh", "uw",
];

// Sound out a word as klattsch phonemes. The rules give no stress, so the
// first full (non-schwa) vowel is stressed, as in most English words. A
// consonant the rules repeat ("tt" in "Klattsch") is said once.
pub fn sound_out(word: &str) -> Vec<Phone> {
    let mut codes = rule_phonemes(word);
    codes.dedup_by(|b, a| a == b && !VOWELS.contains(a));
    let stressed = codes.iter().position(|c| VOWELS.contains(c) && *c != "ax");
    codes
        .iter()
        .enumerate()
        .map(|(i, c)| Phone {
            code: match *c {
                "ax" => "AH".into(),
                "nx" => "NG".into(),
                "wh" => "W".into(),
                other => other.to_ascii_uppercase(),
            },
            stressed: Some(i) == stressed,
        })
        .collect()
}
