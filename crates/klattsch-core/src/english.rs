// English text to a klattsch phoneme string.
//
// This front end is Rust-only (the JS engine's `pronounceText` is a bare
// dictionary lookup that drops punctuation, numbers and unknown words), so it
// has no JS parity fixtures. It:
//   - turns punctuation into klattsch pauses (`,` 100 ms, `;` 200 ms, `.` 300 ms)
//   - reads numbers, ordinals, years, percentages, money ($4.50) and clock
//     times (3:30 p.m.) as words
//   - reads common abbreviations (Dr., St., Jan., etc.), dotted acronyms
//     (U.S., e.g.) and initials (John F. Kennedy) without ending the sentence
//     at their period
//   - pronounces words from the dictionary (a CmuDict, or a Lexicon of custom
//     word lists over it), falling back to acronym spelling (also plurals like
//     NPCs), possessives, hyphen/camel-case/compound splits, and finally the
//     NRL letter-to-sound rules
//   - takes phones written in braces as they are: `I {R IY1 D} books`
//   - removes the dictionary's stress from common function words
//   - adds a basic sentence melody with klattsch pitch directives: a slow
//     downward drift, a fall on the last accent of a statement, a rise on a
//     yes/no question (wh-questions fall), and a small rise before a comma
//
//   let dict = CmuDict::load("data/cmudict/cmudict-0.7b")?;
//   let spoken = english_to_phonemes("Hello, world!", &dict, &EnglishOptions::default());
//   let compiled = compile_string(&spoken.phonemes, &CompileOptions::default())?;

use unicode_normalization::UnicodeNormalization;

use crate::lts;
use crate::pronounce::Pronounce;
use crate::Phone;

#[derive(Clone, Debug, PartialEq)]
pub struct EnglishOptions {
    pub intonation: bool,
    pub unstress_function_words: bool,
    // Sound out words that are not in the dictionary; otherwise skip them.
    pub sound_out_unknown: bool,
    // Total downward pitch drift across a sentence, in Hz.
    pub declination_hz: f64,
    // Fall on the last accent of a statement, in Hz (1.4x for `!`).
    pub final_fall_hz: f64,
    // Rise on the last accent of a question, in Hz.
    pub question_rise_hz: f64,
    // Questions starting with who/what/where/when/why/which/how fall like
    // statements instead of rising.
    pub wh_questions_fall: bool,
    // Rise on the last accent before a comma, in Hz.
    pub comma_rise_hz: f64,
}

impl Default for EnglishOptions {
    fn default() -> Self {
        Self {
            intonation: true,
            unstress_function_words: true,
            sound_out_unknown: true,
            declination_hz: 15.0,
            final_fall_hz: 25.0,
            question_rise_hz: 30.0,
            wh_questions_fall: true,
            comma_rise_hz: 10.0,
        }
    }
}

// How a word's pronunciation was found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WordSource {
    Dictionary,
    // A number, ordinal, year, percentage, amount of money or clock time
    // read as words.
    Number,
    // An all-capitals word, dotted acronym (U.S.) or initial spelled letter
    // by letter.
    Acronym,
    // An abbreviation read as words ("Dr." = doctor). Holds the words.
    Expansion(Vec<String>),
    // Phones written in braces in the text.
    Inline,
    // Built from dictionary words: possessive, hyphenated, camel-case, or a
    // compound such as "multistage" = "multi" + "stage". Holds the parts.
    Parts(Vec<String>),
    // Sounded out with the letter-to-sound rules.
    LetterToSound,
    // Not pronounced (sound_out_unknown is off).
    Skipped,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EnglishWord {
    pub text: String,
    pub source: WordSource,
    pub phones: Vec<Phone>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EnglishResult {
    // klattsch phoneme string, ready for compile_string.
    pub phonemes: String,
    pub words: Vec<EnglishWord>,
}

// Prosodic breaks, weakest to strongest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Break {
    Comma,
    Clause,
    Statement,
    Exclaim,
    Question,
}

impl Break {
    fn ends_sentence(self) -> bool {
        self >= Break::Statement
    }

    fn pause(self) -> &'static str {
        match self {
            Break::Comma => ",",
            Break::Clause => ";",
            _ => ".",
        }
    }
}

// How part of a piece is pronounced.
enum Part {
    // Looked up like any word.
    Word(String),
    // Spelled letter by letter.
    Letters(String),
    // Given directly (inline braces).
    Phones(Vec<Phone>),
}

enum Piece {
    Word(String),
    // Text with a fixed reading: numbers, money, times, abbreviations,
    // dotted acronyms, initials and inline phones.
    Say { text: String, parts: Vec<Part>, source: WordSource },
    Break(Break),
}

impl Piece {
    fn words(text: &str, words: &[&str], source: WordSource) -> Self {
        let parts = words.iter().map(|w| Part::Word((*w).into())).collect();
        Piece::Say { text: text.into(), parts, source }
    }

    fn starts_upper(&self) -> bool {
        match self {
            Piece::Word(text) | Piece::Say { text, .. } => text.starts_with(char::is_uppercase),
            Piece::Break(_) => false,
        }
    }
}

fn is_zero_width(c: char) -> bool {
    matches!(c, '\u{200B}' | '\u{200C}' | '\u{200D}' | '\u{2060}' | '\u{FEFF}')
}

fn is_apostrophe(c: char) -> bool {
    matches!(c, '\'' | '\u{2019}' | '\u{2018}')
}

fn skip_space(chars: &[char], mut j: usize) -> usize {
    while chars.get(j).is_some_and(|c| c.is_whitespace()) {
        j += 1;
    }
    j
}

// Like skip_space, also skipping closing quotes and brackets.
fn skip_closers(chars: &[char], mut j: usize) -> usize {
    while chars.get(j).is_some_and(|c| c.is_whitespace() || matches!(c, '"' | '\'' | '\u{201D}' | '\u{2019}' | ')' | ']')) {
        j += 1;
    }
    j
}

// Whether the word after position `j` starts with a capital letter.
fn next_is_upper(chars: &[char], j: usize) -> bool {
    chars.get(skip_space(chars, j)).is_some_and(|c| c.is_uppercase())
}

// Whether an abbreviation's period just before `j` also ends the sentence:
// at the end of the text, or before a capitalized word.
fn period_ends_sentence(chars: &[char], j: usize) -> bool {
    chars.get(skip_closers(chars, j)).is_none_or(|c| c.is_uppercase())
}

// A number starting at `start` (a digit, or a minus sign before one): digits,
// thousands separators and a decimal part. Returns the index after it.
fn number_end(chars: &[char], start: usize) -> usize {
    let at = |i: usize| chars.get(i).copied();
    let mut i = start + 1;
    while let Some(d) = at(i) {
        let grouped = d == ',' && (1..=3).all(|k| at(i + k).is_some_and(|x| x.is_ascii_digit()))
            && !at(i + 4).is_some_and(|x| x.is_ascii_digit());
        let decimal = d == '.' && at(i + 1).is_some_and(|x| x.is_ascii_digit());
        if d.is_ascii_digit() || grouped || decimal {
            i += 1;
        } else {
            break;
        }
    }
    i
}

// A clock time at `start`: one or two digits, a colon and two more ("3:30",
// "14:05"). Returns hours, minutes and the index after it.
fn clock(chars: &[char], start: usize) -> Option<(u64, u64, usize)> {
    let digits = |from: usize| chars.get(from..).unwrap_or_default().iter().take_while(|c| c.is_ascii_digit()).count();
    let h_len = digits(start);
    let colon = start + h_len;
    if !(1..=2).contains(&h_len) || chars.get(colon) != Some(&':') || digits(colon + 1) != 2 {
        return None;
    }
    let end = colon + 3;
    // Not part of a longer h:mm:ss.
    if chars.get(end) == Some(&':') && chars.get(end + 1).is_some_and(char::is_ascii_digit) {
        return None;
    }
    let num = |a: usize, b: usize| chars[a..b].iter().collect::<String>().parse::<u64>().ok();
    let (h, m) = (num(start, colon)?, num(colon + 1, end)?);
    (h <= 23 && m <= 59).then_some((h, m, end))
}

// "am"/"pm" after a number, written am, AM, a.m. or A.M., attached or after a
// space. Returns the letters, the index after them, and whether they ended
// with a period.
fn am_pm(chars: &[char], j: usize) -> Option<(&'static str, usize, bool)> {
    let mut k = skip_space(chars, j);
    let letters = match chars.get(k).map(char::to_ascii_lowercase) {
        Some('a') => "AM",
        Some('p') => "PM",
        _ => return None,
    };
    k += 1;
    let dotted = chars.get(k) == Some(&'.');
    if dotted {
        k += 1;
    }
    if chars.get(k).map(char::to_ascii_lowercase) != Some('m') {
        return None;
    }
    k += 1;
    if dotted {
        if chars.get(k) != Some(&'.') {
            return None;
        }
        k += 1;
    }
    if chars.get(k).is_some_and(|c| c.is_alphanumeric()) {
        return None;
    }
    Some((letters, k, dotted))
}

// Single letters each followed by a period, the last period optional
// ("U.S.", "p.m.", "e.g."). Returns the letters, the index after the run, and
// whether it ended with a period.
fn dotted_letters(chars: &[char], start: usize) -> Option<(String, usize, bool)> {
    let mut letters = String::new();
    let mut j = start;
    let mut period = false;
    while chars.get(j).is_some_and(|c| c.is_alphabetic()) && !chars.get(j + 1).is_some_and(|c| c.is_alphanumeric()) {
        letters.push(chars[j]);
        j += 1;
        period = chars.get(j) == Some(&'.');
        if !period {
            break;
        }
        j += 1;
    }
    (letters.chars().count() >= 2 && chars.get(start + 1) == Some(&'.')).then_some((letters, j, period))
}

const CURRENCIES: [(char, [&str; 4]); 3] = [
    ('$', ["dollar", "dollars", "cent", "cents"]),
    ('\u{00A3}', ["pound", "pounds", "penny", "pence"]),
    ('\u{20AC}', ["euro", "euros", "cent", "cents"]),
];
const SCALE_WORDS: [&str; 4] = ["thousand", "million", "billion", "trillion"];

// When an abbreviation is read as words.
#[derive(Clone, Copy, PartialEq)]
enum Abbr {
    // Capitalized, before a capitalized word ("Dr. Smith", "Mr Jones"), with
    // or without a period. The period never ends the sentence.
    Title,
    // A title only when the word before is not capitalized: "St. Louis", but
    // "Main St." is a Place.
    PlaceTitle,
    // Capitalized, after a capitalized word ("Main St.", "Elm Dr"), with or
    // without a period.
    Place,
    // Capitalized, with a period ("Jan.", "Inc.").
    Capitalized,
    // With a period, any case ("etc.").
    Dotted,
    // Always, with or without a period ("vs").
    Always,
    // With a period, before a number ("No. 5").
    BeforeNumber,
}

const ABBREVIATIONS: &[(&str, &str, Abbr)] = &[
    ("mr", "mister", Abbr::Title),
    ("mrs", "missus", Abbr::Title),
    ("ms", "ms", Abbr::Title),
    ("dr", "doctor", Abbr::PlaceTitle),
    ("dr", "drive", Abbr::Place),
    ("st", "saint", Abbr::PlaceTitle),
    ("st", "street", Abbr::Place),
    ("prof", "professor", Abbr::Title),
    ("mt", "mount", Abbr::Title),
    ("ft", "fort", Abbr::Title),
    ("sgt", "sergeant", Abbr::Title),
    ("capt", "captain", Abbr::Title),
    ("lt", "lieutenant", Abbr::Title),
    ("col", "colonel", Abbr::Title),
    ("gen", "general", Abbr::Title),
    ("rev", "reverend", Abbr::Title),
    ("gov", "governor", Abbr::Title),
    ("sen", "senator", Abbr::Title),
    ("rep", "representative", Abbr::Title),
    ("pres", "president", Abbr::Title),
    ("fr", "father", Abbr::Title),
    ("ave", "avenue", Abbr::Place),
    ("blvd", "boulevard", Abbr::Place),
    ("rd", "road", Abbr::Place),
    ("jr", "junior", Abbr::Place),
    ("sr", "senior", Abbr::Place),
    ("inc", "incorporated", Abbr::Capitalized),
    ("ltd", "limited", Abbr::Capitalized),
    ("corp", "corporation", Abbr::Capitalized),
    ("dept", "department", Abbr::Capitalized),
    ("govt", "government", Abbr::Capitalized),
    ("jan", "january", Abbr::Capitalized),
    ("feb", "february", Abbr::Capitalized),
    ("mar", "march", Abbr::Capitalized),
    ("apr", "april", Abbr::Capitalized),
    ("jun", "june", Abbr::Capitalized),
    ("jul", "july", Abbr::Capitalized),
    ("aug", "august", Abbr::Capitalized),
    ("sep", "september", Abbr::Capitalized),
    ("sept", "september", Abbr::Capitalized),
    ("oct", "october", Abbr::Capitalized),
    ("nov", "november", Abbr::Capitalized),
    ("dec", "december", Abbr::Capitalized),
    ("etc", "et cetera", Abbr::Dotted),
    ("approx", "approximately", Abbr::Dotted),
    ("misc", "miscellaneous", Abbr::Dotted),
    ("vs", "versus", Abbr::Always),
    ("no", "number", Abbr::BeforeNumber),
    ("vol", "volume", Abbr::BeforeNumber),
    ("ch", "chapter", Abbr::BeforeNumber),
    ("fig", "figure", Abbr::BeforeNumber),
];

// The words for an abbreviation, if `word` is read as one here, and whether
// its period can end the sentence. `j` is the index after the word.
fn abbreviation(word: &str, chars: &[char], j: usize, prev_upper: bool) -> Option<(&'static str, bool)> {
    let key = word.to_lowercase();
    let upper = word.starts_with(char::is_uppercase);
    let dot = chars.get(j) == Some(&'.');
    let after = j + usize::from(dot);
    let next_upper = next_is_upper(chars, after);
    let next_digit = chars.get(skip_space(chars, after)).is_some_and(char::is_ascii_digit);
    ABBREVIATIONS.iter().filter(|(a, ..)| *a == key).find_map(|&(_, words, kind)| {
        let (read, may_end) = match kind {
            Abbr::Title => (upper && next_upper, false),
            Abbr::PlaceTitle => (upper && next_upper && !prev_upper, false),
            Abbr::Place => (upper && prev_upper, true),
            Abbr::Capitalized => (upper && dot, true),
            Abbr::Dotted => (dot, true),
            Abbr::Always => (true, true),
            Abbr::BeforeNumber => (dot && next_digit, false),
        };
        read.then_some((words, may_end))
    })
}

// Reads an amount of money: "$4.50" -> four dollars and fifty cents,
// "$2.5 million" -> two point five million dollars.
fn money_words(names: [&str; 4], amount: &str, scale: Option<&str>) -> Vec<String> {
    let (int_part, frac) = amount.split_once('.').unwrap_or((amount, ""));
    let digits: String = int_part.chars().filter(char::is_ascii_digit).collect();
    let whole = digits.parse::<u64>().ok().filter(|_| digits.len() <= 15);
    let cents = match frac.len() {
        0 => Some(0),
        1 => frac.parse::<u64>().ok().map(|c| c * 10),
        2 => frac.parse::<u64>().ok(),
        _ => None,
    };
    let mut out = Vec::new();
    match (whole, cents, scale) {
        (Some(n), Some(c), None) => {
            if n > 0 || c == 0 {
                cardinal(n, &mut out);
                out.push(names[if n == 1 { 0 } else { 1 }].into());
            }
            if c > 0 {
                if n > 0 {
                    out.push("and".into());
                }
                cardinal(c, &mut out);
                out.push(names[if c == 1 { 2 } else { 3 }].into());
            }
        }
        _ => {
            out = number_words(amount);
            out.extend(scale.map(str::to_lowercase));
            out.push(names[1].into());
        }
    }
    out
}

// Reads a clock time: 3:30 -> three thirty, 3:05 -> three oh five,
// 10:00 -> ten o'clock (just ten before am/pm), 14:00 -> fourteen hundred.
fn time_words(h: u64, m: u64, has_am_pm: bool) -> Vec<String> {
    let mut out = Vec::new();
    cardinal(h, &mut out);
    match m {
        0 if has_am_pm => {}
        0 if (1..=12).contains(&h) => out.push("o'clock".into()),
        0 => out.push("hundred".into()),
        1..=9 => {
            out.push("oh".into());
            out.push(ONES[m as usize].into());
        }
        _ => below_thousand(m, &mut out),
    }
    out
}

// Phones written in braces: ARPABET codes with optional stress digits
// ("R IY1 D") or klattsch stress marks ("R IY' D"). Anything else is dropped.
fn inline_phones(text: &str) -> Vec<Phone> {
    text.split_whitespace()
        .filter_map(|tok| {
            let (code, stressed) = match tok.chars().last() {
                Some('1' | '\'') => (&tok[..tok.len() - 1], true),
                Some('0' | '2') => (&tok[..tok.len() - 1], false),
                _ => (tok, false),
            };
            (!code.is_empty() && code.len() <= 3 && code.chars().all(|c| c.is_ascii_alphabetic()))
                .then(|| Phone { code: code.to_ascii_uppercase(), stressed })
        })
        .collect()
}

fn split_pieces(text: &str) -> Vec<Piece> {
    let chars: Vec<char> = text.nfc().filter(|c| !is_zero_width(*c)).collect();
    let at = |i: usize| chars.get(i).copied();
    let mut pieces = Vec::new();
    let push_break = |pieces: &mut Vec<Piece>, b: Break| {
        // Runs like "!…" or "?!" become one break, the strongest of them.
        if let Some(Piece::Break(prev)) = pieces.last_mut() {
            *prev = (*prev).max(b);
        } else {
            pieces.push(Piece::Break(b));
        }
    };
    // After an abbreviation, acronym or am/pm whose period ends at `j`: end
    // the sentence if the text after it says so.
    let period_break = |pieces: &mut Vec<Piece>, j: usize| {
        if period_ends_sentence(&chars, j) {
            push_break(pieces, Break::Statement);
        }
    };
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        // Money: "$4.50", "£3", "€2.5 million".
        if let Some((_, names)) = CURRENCIES.iter().find(|(sym, _)| *sym == c) {
            if at(i + 1).is_some_and(|d| d.is_ascii_digit()) {
                let mut end = number_end(&chars, i + 1);
                let amount: String = chars[i + 1..end].iter().collect();
                let k = skip_space(&chars, end);
                let word: String = chars[k..].iter().take_while(|c| c.is_alphabetic()).collect();
                let scale = SCALE_WORDS.iter().find(|s| word.eq_ignore_ascii_case(s)).copied();
                if scale.is_some() {
                    end = k + word.chars().count();
                }
                let parts = money_words(*names, &amount, scale).into_iter().map(Part::Word).collect();
                pieces.push(Piece::Say { text: chars[i..end].iter().collect(), parts, source: WordSource::Number });
                i = end;
                continue;
            }
        }
        let negative = c == '-' && at(i + 1).is_some_and(|d| d.is_ascii_digit())
            && (i == 0 || !chars[i - 1].is_alphanumeric());
        if c.is_ascii_digit() || negative {
            let start = i;
            let time = if negative { None } else { clock(&chars, i) };
            let mut words = Vec::new();
            match time {
                Some((_, _, end)) => i = end,
                None => {
                    i = number_end(&chars, i);
                    // Ordinal suffix (21st) or percent sign.
                    let rest: String = chars[i..].iter().take(3).collect::<String>().to_lowercase();
                    let suffix = ["st", "nd", "rd", "th"].iter().find(|s| {
                        rest.starts_with(**s) && !at(i + 2).is_some_and(char::is_alphanumeric)
                    });
                    let mut token: String = chars[start..i].iter().collect();
                    if let Some(s) = suffix {
                        token.push_str(s);
                        i += 2;
                    } else if at(i) == Some('%') {
                        token.push('%');
                        i += 1;
                    }
                    words = number_words(&token);
                }
            }
            let ampm = am_pm(&chars, i);
            if let Some((h, m, _)) = time {
                words = time_words(h, m, ampm.is_some());
            }
            let mut parts: Vec<Part> = words.into_iter().map(Part::Word).collect();
            let mut period = false;
            if let Some((letters, end, dot)) = ampm {
                parts.push(Part::Letters(letters.into()));
                i = end;
                period = dot;
            }
            pieces.push(Piece::Say { text: chars[start..i].iter().collect(), parts, source: WordSource::Number });
            if period {
                period_break(&mut pieces, i);
            }
            continue;
        }
        if c.is_alphabetic() {
            // Dotted acronyms: "U.S.", "e.g.".
            if let Some((letters, end, dot)) = dotted_letters(&chars, i) {
                let text: String = chars[i..end].iter().collect();
                let expansion: &[&str] = match letters.to_lowercase().as_str() {
                    "eg" => &["for", "example"],
                    "ie" => &["that", "is"],
                    _ => &[],
                };
                pieces.push(if expansion.is_empty() {
                    Piece::Say { text, parts: vec![Part::Letters(letters)], source: WordSource::Acronym }
                } else {
                    let source = WordSource::Expansion(expansion.iter().map(|w| w.to_string()).collect());
                    Piece::words(&text, expansion, source)
                });
                i = end;
                if dot {
                    period_break(&mut pieces, i);
                }
                continue;
            }
            // An initial before a capitalized word, after a capitalized word
            // or at the start of a sentence: "John F. Kennedy", "J. Smith".
            let sentence_start = pieces.last().is_none_or(|p| matches!(p, Piece::Break(b) if b.ends_sentence()));
            if c.is_uppercase() && at(i + 1) == Some('.') && at(i + 2).is_some_and(char::is_whitespace)
                && next_is_upper(&chars, i + 2)
                && (sentence_start || pieces.last().is_some_and(Piece::starts_upper))
            {
                let text = String::from(c);
                pieces.push(Piece::Say { text: text.clone(), parts: vec![Part::Letters(text)], source: WordSource::Acronym });
                i += 2;
                continue;
            }
            let start = i;
            while let Some(d) = at(i) {
                let joiner = (is_apostrophe(d) || d == '-') && at(i + 1).is_some_and(char::is_alphabetic);
                if d.is_alphabetic() || (joiner && i > start) {
                    i += 1;
                } else {
                    break;
                }
            }
            let word: String = chars[start..i].iter().map(|&d| if is_apostrophe(d) { '\'' } else { d }).collect();
            let prev_upper = pieces.last().is_some_and(Piece::starts_upper);
            if let Some((words, may_end)) = abbreviation(&word, &chars, i, prev_upper) {
                let list: Vec<&str> = words.split(' ').collect();
                let source = WordSource::Expansion(list.iter().map(|w| w.to_string()).collect());
                pieces.push(Piece::words(&word, &list, source));
                if at(i) == Some('.') {
                    i += 1;
                    // A title's period still ends the text's last sentence.
                    if may_end || chars.get(skip_closers(&chars, i)).is_none() {
                        period_break(&mut pieces, i);
                    }
                }
                continue;
            }
            pieces.push(Piece::Word(word));
            continue;
        }
        // Phones in braces: "{R IY1 D}".
        if c == '{' {
            if let Some(len) = chars[i + 1..].iter().position(|&d| d == '}') {
                let text: String = chars[i + 1..i + 1 + len].iter().collect();
                let phones = inline_phones(&text);
                if !phones.is_empty() {
                    let text = text.trim().to_string();
                    pieces.push(Piece::Say { text, parts: vec![Part::Phones(phones)], source: WordSource::Inline });
                }
                i += len + 2;
                continue;
            }
        }
        match c {
            ',' | '(' | ')' => push_break(&mut pieces, Break::Comma),
            ';' | ':' | '\u{2013}' | '\u{2014}' => push_break(&mut pieces, Break::Clause),
            '-' if at(i + 1).is_none_or(char::is_whitespace) => push_break(&mut pieces, Break::Clause),
            '.' | '\u{2026}' => push_break(&mut pieces, Break::Statement),
            '!' => push_break(&mut pieces, Break::Exclaim),
            '?' => push_break(&mut pieces, Break::Question),
            '&' => pieces.push(Piece::Word("and".into())),
            _ => {}
        }
        i += 1;
    }
    pieces
}

// --- numbers ------------------------------------------------------------------

const ONES: [&str; 20] = [
    "zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten",
    "eleven", "twelve", "thirteen", "fourteen", "fifteen", "sixteen", "seventeen", "eighteen", "nineteen",
];
const TENS: [&str; 10] = ["", "", "twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety"];
const SCALES: [&str; 5] = ["", "thousand", "million", "billion", "trillion"];

fn below_thousand(n: u64, out: &mut Vec<String>) {
    let (h, rest) = (n / 100, n % 100);
    if h > 0 {
        out.push(ONES[h as usize].into());
        out.push("hundred".into());
    }
    if rest >= 20 {
        out.push(TENS[(rest / 10) as usize].into());
        if rest % 10 > 0 {
            out.push(ONES[(rest % 10) as usize].into());
        }
    } else if rest > 0 || h == 0 {
        out.push(ONES[rest as usize].into());
    }
}

fn cardinal(n: u64, out: &mut Vec<String>) {
    if n == 0 {
        out.push("zero".into());
        return;
    }
    let mut groups = Vec::new();
    let mut m = n;
    while m > 0 {
        groups.push(m % 1000);
        m /= 1000;
    }
    for (scale, &g) in groups.iter().enumerate().rev() {
        if g > 0 {
            below_thousand(g, out);
            if scale > 0 {
                out.push(SCALES[scale].into());
            }
        }
    }
}

// 1999 -> "nineteen ninety nine"; 2005 -> "two thousand five".
fn year(n: u64, out: &mut Vec<String>) {
    let (hi, lo) = (n / 100, n % 100);
    below_thousand(hi, out);
    match lo {
        0 => out.push("hundred".into()),
        1..=9 => {
            out.push("oh".into());
            out.push(ONES[lo as usize].into());
        }
        _ => below_thousand(lo, out),
    }
}

fn ordinal_word(w: &str) -> String {
    match w {
        "one" => "first".into(),
        "two" => "second".into(),
        "three" => "third".into(),
        "five" => "fifth".into(),
        "eight" => "eighth".into(),
        "nine" => "ninth".into(),
        "twelve" => "twelfth".into(),
        w if w.ends_with('y') => format!("{}ieth", &w[..w.len() - 1]),
        w => format!("{w}th"),
    }
}

fn number_words(token: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut t = token;
    if let Some(rest) = t.strip_prefix('-') {
        out.push("minus".into());
        t = rest;
    }
    let percent = t.ends_with('%');
    let t = t.trim_end_matches('%');
    let ordinal = ["st", "nd", "rd", "th"].iter().any(|s| t.ends_with(s));
    let t = if ordinal { &t[..t.len() - 2] } else { t };
    let (int_part, frac) = t.split_once('.').unwrap_or((t, ""));
    let digits: String = int_part.chars().filter(char::is_ascii_digit).collect();
    match digits.parse::<u64>() {
        Ok(n) if digits.len() <= 15 => {
            let looks_like_year = digits.len() == 4 && !int_part.contains(',') && frac.is_empty() && !ordinal
                && !percent && ((1100..=1999).contains(&n) || (2010..=2099).contains(&n));
            if looks_like_year {
                year(n, &mut out);
            } else {
                cardinal(n, &mut out);
            }
        }
        // Too long to read as a number: read the digits.
        _ => out.extend(digits.chars().map(|d| ONES[d.to_digit(10).unwrap() as usize].to_string())),
    }
    if ordinal {
        if let Some(last) = out.last_mut() {
            *last = ordinal_word(last);
        }
    }
    if !frac.is_empty() {
        out.push("point".into());
        out.extend(frac.chars().map(|d| ONES[d.to_digit(10).unwrap() as usize].to_string()));
    }
    if percent {
        out.push("percent".into());
    }
    out
}

// --- words ----------------------------------------------------------------------

// Short, frequent words that are normally unaccented in running speech. The
// dictionary gives most of them primary stress, which makes every word sound
// emphasized.
const FUNCTION_WORDS: &[&str] = &[
    "a", "an", "the", "and", "or", "but", "nor", "so", "as", "at", "by", "for", "from", "in", "into",
    "of", "on", "onto", "to", "with", "than", "then", "if", "is", "am", "are", "was", "were", "be",
    "been", "being", "it", "its", "it's", "i", "i'm", "me", "my", "you", "you're", "your", "he",
    "he's", "him", "his", "she", "she's", "her", "we", "we're", "us", "our", "they", "they're",
    "them", "their", "this", "that", "that's", "these", "those", "there", "will", "would", "can",
    "could", "shall", "should", "may", "might", "must", "do", "does", "did", "has", "have", "had",
];

const LETTER_NAMES: [&str; 26] = [
    "EY", "B IY", "S IY", "D IY", "IY", "EH F", "JH IY", "EY CH", "AY", "JH EY", "K EY", "EH L", "EH M",
    "EH N", "OW", "P IY", "K Y UW", "AA R", "EH S", "T IY", "Y UW", "V IY", "D AH B AH L Y UW", "EH K S",
    "W AY", "Z IY",
];
const VOWEL_CODES: [&str; 15] = [
    "AA", "AE", "AH", "AO", "AW", "AY", "EH", "ER", "EY", "IH", "IY", "OW", "OY", "UH", "UW",
];

fn spell(word: &str) -> Vec<Phone> {
    let letters: Vec<char> = word.chars().filter(char::is_ascii_alphabetic).collect();
    let mut out = Vec::new();
    for (i, c) in letters.iter().enumerate() {
        let name = LETTER_NAMES[(c.to_ascii_uppercase() as u8 - b'A') as usize];
        let last = i + 1 == letters.len();
        // Acronyms are accented on their last letter (F B I).
        out.extend(name.split(' ').map(|code| Phone {
            code: code.into(),
            stressed: last && VOWEL_CODES.contains(&code),
        }));
    }
    out
}

fn plural_s(phones: &mut Vec<Phone>) {
    let last = phones.last().map(|p| p.code.as_str()).unwrap_or("");
    match last {
        "S" | "Z" | "SH" | "ZH" | "CH" | "JH" => {
            phones.push(Phone::new("IH"));
            phones.push(Phone::new("Z"));
        }
        "P" | "T" | "K" | "F" | "TH" => phones.push(Phone::new("S")),
        _ => phones.push(Phone::new("Z")),
    }
}

struct Lookup<'a> {
    dict: &'a dyn Pronounce,
    opts: &'a EnglishOptions,
}

impl Lookup<'_> {
    fn dictionary_only(&self, word: &str) -> Option<Vec<Phone>> {
        self.dict.pronounce(word)
    }

    // A pronunciation from the dictionary alone (exact, or by splitting).
    fn resolve(&self, word: &str) -> Option<(WordSource, Vec<Phone>)> {
        if let Some(p) = self.dictionary_only(word) {
            return Some((WordSource::Dictionary, p));
        }
        let letters = word.chars().filter(|c| c.is_alphabetic()).count();
        if letters >= 2 && word.chars().all(|c| c.is_ascii_uppercase()) {
            return Some((WordSource::Acronym, spell(word)));
        }
        // Acronym plurals: "NPCs", "URLs".
        if let Some(base) = word.strip_suffix('s') {
            if base.chars().count() >= 2 && base.chars().all(|c| c.is_ascii_uppercase()) {
                let mut p = spell(base);
                plural_s(&mut p);
                return Some((WordSource::Parts(vec![base.into(), "s".into()]), p));
            }
        }
        if let Some(base) = word.strip_suffix("'s").or_else(|| word.strip_suffix("'S")) {
            if let Some((_, mut p)) = self.resolve(base) {
                plural_s(&mut p);
                return Some((WordSource::Parts(vec![base.into(), "'s".into()]), p));
            }
        }
        if word.contains('-') {
            return self.join_parts(word.split('-').filter(|s| !s.is_empty()).map(String::from).collect());
        }
        let camel = camel_case_parts(word);
        if camel.len() > 1 {
            if let Some(r) = self.join_parts(camel) {
                return Some(r);
            }
        }
        self.compound(word)
    }

    fn join_parts(&self, parts: Vec<String>) -> Option<(WordSource, Vec<Phone>)> {
        let mut phones = Vec::new();
        for part in &parts {
            phones.extend(self.resolve(part)?.1);
        }
        Some((WordSource::Parts(parts), phones))
    }

    // Two dictionary words of at least three letters each, preferring the most
    // even split ("multistage" -> "multi" + "stage").
    fn compound(&self, word: &str) -> Option<(WordSource, Vec<Phone>)> {
        let chars: Vec<char> = word.chars().collect();
        let n = chars.len();
        let mut best: Option<(usize, Vec<Phone>, Vec<Phone>)> = None;
        for split in 3..=n.saturating_sub(3) {
            let (a, b): (String, String) = (chars[..split].iter().collect(), chars[split..].iter().collect());
            if let (Some(pa), Some(pb)) = (self.dictionary_only(&a), self.dictionary_only(&b)) {
                let balance = split.min(n - split);
                if best.as_ref().is_none_or(|(s, ..)| balance > (*s).min(n - *s)) {
                    best = Some((split, pa, pb));
                }
            }
        }
        let (split, mut pa, pb) = best?;
        pa.extend(pb);
        let parts = vec![chars[..split].iter().collect(), chars[split..].iter().collect()];
        Some((WordSource::Parts(parts), pa))
    }

    fn word(&self, text: &str) -> EnglishWord {
        let (source, mut phones) = match self.resolve(text) {
            Some(found) => found,
            None if self.opts.sound_out_unknown => (WordSource::LetterToSound, lts::sound_out(text)),
            None => (WordSource::Skipped, Vec::new()),
        };
        if self.opts.unstress_function_words && FUNCTION_WORDS.contains(&text.to_lowercase().as_str()) {
            phones.iter_mut().for_each(|p| p.stressed = false);
        }
        EnglishWord { text: text.into(), source, phones }
    }
}

// "FitnessGram" -> ["Fitness", "Gram"], "GDExtension" -> ["GD", "Extension"]
// (a capital run ends before a capital that starts a lowercase word).
fn camel_case_parts(word: &str) -> Vec<String> {
    let chars: Vec<char> = word.chars().collect();
    let lower = |i: usize| chars.get(i).is_some_and(|c| c.is_lowercase());
    let mut parts = vec![String::new()];
    for (i, &c) in chars.iter().enumerate() {
        let prev_upper = i > 0 && chars[i - 1].is_uppercase();
        if c.is_uppercase() && i > 0 && (lower(i - 1) || (prev_upper && lower(i + 1) && lower(i + 2))) {
            parts.push(String::new());
        }
        parts.last_mut().unwrap().push(c);
    }
    parts
}

// --- melody and output -----------------------------------------------------------

fn hz(x: f64) -> String {
    let s = format!("{:.2}", x.abs());
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

const WH_WORDS: [&str; 9] = ["who", "whom", "whose", "what", "which", "where", "when", "why", "how"];

enum Item {
    Word(usize),
    Break(Break),
}

// Emits one sentence: its items, ending with an optional end break.
fn emit_sentence(items: &[Item], words: &[EnglishWord], opts: &EnglishOptions, out: &mut Vec<String>) {
    let end = match items.last() {
        Some(Item::Break(b)) if b.ends_sentence() => Some(*b),
        _ => None,
    };
    // Wh-questions end with a fall, like statements.
    let first = items.iter().find_map(|i| match i {
        Item::Word(w) => Some(words[*w].text.to_lowercase()),
        Item::Break(_) => None,
    });
    let wh = opts.wh_questions_fall && first.is_some_and(|w| WH_WORDS.contains(&w.as_str()));
    let contour_of = |b: Break| if wh && b == Break::Question { Break::Statement } else { b };
    // For each stressed phone, the break that follows its phrase, if it is the
    // phrase's last accent.
    let mut accents: Vec<(usize, usize)> = Vec::new(); // (word index, phone index)
    let mut phrase_final: Vec<((usize, usize), Break)> = Vec::new();
    let mut last_in_phrase: Option<(usize, usize)> = None;
    for item in items {
        match item {
            Item::Word(w) => {
                for (pi, p) in words[*w].phones.iter().enumerate() {
                    if p.stressed {
                        accents.push((*w, pi));
                        last_in_phrase = Some((*w, pi));
                    }
                }
            }
            Item::Break(b) => {
                if let Some(a) = last_in_phrase.take() {
                    phrase_final.push((a, contour_of(*b)));
                }
            }
        }
    }
    if let Some(a) = last_in_phrase {
        // No closing punctuation: end like a statement.
        phrase_final.push((a, contour_of(end.unwrap_or(Break::Statement))));
    }
    let nucleus = phrase_final.last().map(|(a, _)| *a);
    let step = if accents.len() > 1 { opts.declination_hz / (accents.len() - 1) as f64 } else { 0.0 };

    if opts.intonation {
        out.push("b".into()); // back to the base pitch
    }
    for item in items {
        match item {
            Item::Break(b) => out.push(b.pause().into()),
            Item::Word(w) => {
                let mut drift = 0.0;
                for (pi, p) in words[*w].phones.iter().enumerate() {
                    let mut tok = p.code.clone();
                    if p.stressed {
                        tok.push('\'');
                        if opts.intonation {
                            let contour = phrase_final.iter().find(|(a, _)| *a == (*w, pi)).map(|(_, b)| *b);
                            match contour {
                                Some(Break::Comma) => tok.push_str(&format!("(+{})", hz(opts.comma_rise_hz))),
                                Some(Break::Clause) => tok.push_str(&format!("(-{})", hz(opts.final_fall_hz * 0.5))),
                                Some(Break::Statement) => tok.push_str(&format!("-{}", hz(opts.final_fall_hz))),
                                Some(Break::Exclaim) => tok.push_str(&format!("-{}", hz(opts.final_fall_hz * 1.4))),
                                Some(Break::Question) => tok.push_str(&format!("+{}", hz(opts.question_rise_hz))),
                                None => {}
                            }
                            if Some((*w, pi)) != nucleus {
                                drift += step;
                            }
                        }
                    }
                    out.push(tok);
                }
                let holds_nucleus = nucleus.is_some_and(|(nw, _)| nw == *w);
                if opts.intonation && !holds_nucleus && drift > 0.0 && hz(drift) != "0" {
                    out.push(format!("b-{}", hz(drift)));
                }
            }
        }
    }
}

// `dict` is a CmuDict, or a Lexicon to layer custom word lists over it.
pub fn english_to_phonemes(text: &str, dict: &dyn Pronounce, opts: &EnglishOptions) -> EnglishResult {
    let lookup = Lookup { dict, opts };
    let mut words: Vec<EnglishWord> = Vec::new();
    let mut sentences: Vec<Vec<Item>> = vec![Vec::new()];
    for piece in split_pieces(text) {
        let current = sentences.last_mut().unwrap();
        match piece {
            Piece::Word(w) => {
                current.push(Item::Word(words.len()));
                words.push(lookup.word(&w));
            }
            Piece::Say { text, parts, source } => {
                let phones = parts
                    .into_iter()
                    .flat_map(|part| match part {
                        Part::Word(w) => lookup.word(&w).phones,
                        Part::Letters(l) => spell(&l),
                        Part::Phones(p) => p,
                    })
                    .collect();
                current.push(Item::Word(words.len()));
                words.push(EnglishWord { text, source, phones });
            }
            Piece::Break(b) => {
                // A break before any word in the sentence carries no pause.
                if current.iter().any(|i| matches!(i, Item::Word(_))) {
                    current.push(Item::Break(b));
                    if b.ends_sentence() {
                        sentences.push(Vec::new());
                    }
                }
            }
        }
    }
    let mut out = Vec::new();
    for s in sentences.iter().filter(|s| !s.is_empty()) {
        emit_sentence(s, &words, opts, &mut out);
    }
    EnglishResult { phonemes: out.join(" "), words }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_read_as_words() {
        let cases = [
            ("20", "twenty"), ("30", "thirty"), ("0", "zero"), ("105", "one hundred five"),
            ("1,250", "one thousand two hundred fifty"), ("1999", "nineteen ninety nine"),
            ("2005", "two thousand five"), ("2024", "twenty twenty four"), ("1900", "nineteen hundred"),
            ("1906", "nineteen oh six"), ("3.14", "three point one four"), ("-7", "minus seven"),
            ("1st", "first"), ("2nd", "second"), ("3rd", "third"), ("12th", "twelfth"), ("20th", "twentieth"),
            ("21st", "twenty first"), ("50%", "fifty percent"), ("1000000", "one million"),
            ("1234567890123456789", "one two three four five six seven eight nine zero one two three four five six seven eight nine"),
        ];
        for (n, want) in cases {
            assert_eq!(number_words(n).join(" "), want, "{n}");
        }
    }

    #[test]
    fn punctuation_becomes_breaks() {
        let kinds: Vec<String> = split_pieces("Hi, you; there: OK!… Why?! 3.5% ok - yes (no) end.")
            .iter()
            .map(|p| match p {
                Piece::Word(w) => w.clone(),
                Piece::Say { text, .. } => format!("#{text}"),
                Piece::Break(b) => format!("{b:?}"),
            })
            .collect();
        assert_eq!(
            kinds.join(" "),
            "Hi Comma you Clause there Clause OK Exclaim Why Question #3.5% ok Clause yes Comma no Comma end Statement"
        );
    }

    #[test]
    fn camel_case() {
        assert_eq!(camel_case_parts("FitnessGram"), ["Fitness", "Gram"]);
        assert_eq!(camel_case_parts("NASA"), ["NASA"]);
        assert_eq!(camel_case_parts("iPhone"), ["i", "Phone"]);
    }
}
