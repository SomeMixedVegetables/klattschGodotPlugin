// Japanese kana/romaji to klattsch phoneme conversion.
//
//   kana_to_phonemes("こんにちは")  // K O N N I CH I HH A
//   romaji_to_phonemes("konnichiha") // same

use std::collections::HashMap;
use std::sync::LazyLock;

use crate::js;
use crate::Phone;

const GEMINATE: &str = "_GEMINATE";
const LONG: &str = "_LONG";

const KANA_MAP: &[(&str, &[&str])] = &[
    ("あ", &["A"]), ("い", &["I"]), ("う", &["U"]), ("え", &["E"]), ("お", &["O"]),
    ("か", &["K", "A"]), ("き", &["K", "I"]), ("く", &["K", "U"]), ("け", &["K", "E"]), ("こ", &["K", "O"]),
    ("さ", &["S", "A"]), ("し", &["SH", "I"]), ("す", &["S", "U"]), ("せ", &["S", "E"]), ("そ", &["S", "O"]),
    ("た", &["T", "A"]), ("ち", &["CH", "I"]), ("つ", &["T", "S", "U"]), ("て", &["T", "E"]), ("と", &["T", "O"]),
    ("な", &["N", "A"]), ("に", &["N", "I"]), ("ぬ", &["N", "U"]), ("ね", &["N", "E"]), ("の", &["N", "O"]),
    ("は", &["HH", "A"]), ("ひ", &["HH", "I"]), ("ふ", &["F", "U"]), ("へ", &["HH", "E"]), ("ほ", &["HH", "O"]),
    ("ま", &["M", "A"]), ("み", &["M", "I"]), ("む", &["M", "U"]), ("め", &["M", "E"]), ("も", &["M", "O"]),
    ("や", &["Y", "A"]), ("ゆ", &["Y", "U"]), ("よ", &["Y", "O"]),
    ("ら", &["DX", "A"]), ("り", &["DX", "I"]), ("る", &["DX", "U"]), ("れ", &["DX", "E"]), ("ろ", &["DX", "O"]),
    ("わ", &["W", "A"]), ("を", &["O"]), ("ん", &["N"]),
    // dakuten
    ("が", &["G", "A"]), ("ぎ", &["G", "I"]), ("ぐ", &["G", "U"]), ("げ", &["G", "E"]), ("ご", &["G", "O"]),
    ("ざ", &["Z", "A"]), ("じ", &["JH", "I"]), ("ず", &["Z", "U"]), ("ぜ", &["Z", "E"]), ("ぞ", &["Z", "O"]),
    ("だ", &["D", "A"]), ("ぢ", &["JH", "I"]), ("づ", &["Z", "U"]), ("で", &["D", "E"]), ("ど", &["D", "O"]),
    ("ば", &["B", "A"]), ("び", &["B", "I"]), ("ぶ", &["B", "U"]), ("べ", &["B", "E"]), ("ぼ", &["B", "O"]),
    // handakuten
    ("ぱ", &["P", "A"]), ("ぴ", &["P", "I"]), ("ぷ", &["P", "U"]), ("ぺ", &["P", "E"]), ("ぽ", &["P", "O"]),
    // yoon (combo kana)
    ("きゃ", &["K", "Y", "A"]), ("きゅ", &["K", "Y", "U"]), ("きょ", &["K", "Y", "O"]),
    ("しゃ", &["SH", "A"]), ("しゅ", &["SH", "U"]), ("しょ", &["SH", "O"]),
    ("ちゃ", &["CH", "A"]), ("ちゅ", &["CH", "U"]), ("ちょ", &["CH", "O"]),
    ("にゃ", &["N", "Y", "A"]), ("にゅ", &["N", "Y", "U"]), ("にょ", &["N", "Y", "O"]),
    ("ひゃ", &["HH", "Y", "A"]), ("ひゅ", &["HH", "Y", "U"]), ("ひょ", &["HH", "Y", "O"]),
    ("みゃ", &["M", "Y", "A"]), ("みゅ", &["M", "Y", "U"]), ("みょ", &["M", "Y", "O"]),
    ("りゃ", &["DX", "Y", "A"]), ("りゅ", &["DX", "Y", "U"]), ("りょ", &["DX", "Y", "O"]),
    ("ぎゃ", &["G", "Y", "A"]), ("ぎゅ", &["G", "Y", "U"]), ("ぎょ", &["G", "Y", "O"]),
    ("じゃ", &["JH", "A"]), ("じゅ", &["JH", "U"]), ("じょ", &["JH", "O"]),
    ("びゃ", &["B", "Y", "A"]), ("びゅ", &["B", "Y", "U"]), ("びょ", &["B", "Y", "O"]),
    ("ぴゃ", &["P", "Y", "A"]), ("ぴゅ", &["P", "Y", "U"]), ("ぴょ", &["P", "Y", "O"]),
    // small tsu (geminate) handled separately
    ("っ", &[GEMINATE]),
    // long vowel
    ("ー", &[LONG]),
];

static KANA: LazyLock<HashMap<&'static str, &'static [&'static str]>> =
    LazyLock::new(|| KANA_MAP.iter().copied().collect());

fn phones(codes: &[&str]) -> impl Iterator<Item = Phone> {
    codes.iter().map(|c| Phone::new(c))
}

// katakana: shift codepoint range to hiragana
fn kata_to_hira(ch: char) -> char {
    let cp = ch as u32;
    if (0x30A1..=0x30F6).contains(&cp) {
        return char::from_u32(cp - 0x60).unwrap_or(ch);
    }
    if cp == 0x30FC {
        return 'ー';
    }
    ch
}

pub fn kana_to_phonemes(text: &str) -> Option<Vec<Phone>> {
    let hira: Vec<char> = text.chars().map(kata_to_hira).collect();
    let mut result: Vec<Phone> = Vec::new();
    let mut i = 0;
    let mut buf = [0u8; 8];

    while i < hira.len() {
        // try 2-char yoon first
        if i + 1 < hira.len() {
            let pair: String = [hira[i], hira[i + 1]].iter().collect();
            if let Some(codes) = KANA.get(pair.as_str()) {
                result.extend(phones(codes));
                i += 2;
                continue;
            }
        }

        let ch: &str = hira[i].encode_utf8(&mut buf);
        if let Some(codes) = KANA.get(ch) {
            if codes[0] == GEMINATE {
                // double the next consonant (add a pause)
                result.push(Phone::new("_"));
            } else if codes[0] == LONG {
                // extend the previous vowel
                if let Some(prev) = result.last() {
                    result.push(Phone::new(&prev.code.clone()));
                }
            } else {
                result.extend(phones(codes));
            }
        }
        // skip unknown characters (spaces, punctuation)
        i += 1;
    }

    (!result.is_empty()).then_some(result)
}

// Romaji to phoneme mapping
const ROMAJI_MAP: &[(&str, &[&str])] = &[
    ("a", &["A"]), ("i", &["I"]), ("u", &["U"]), ("e", &["E"]), ("o", &["O"]),
    ("ka", &["K", "A"]), ("ki", &["K", "I"]), ("ku", &["K", "U"]), ("ke", &["K", "E"]), ("ko", &["K", "O"]),
    ("sa", &["S", "A"]), ("shi", &["SH", "I"]), ("si", &["SH", "I"]), ("su", &["S", "U"]), ("se", &["S", "E"]), ("so", &["S", "O"]),
    ("ta", &["T", "A"]), ("chi", &["CH", "I"]), ("ti", &["CH", "I"]), ("tsu", &["T", "S", "U"]), ("tu", &["T", "S", "U"]), ("te", &["T", "E"]), ("to", &["T", "O"]),
    ("na", &["N", "A"]), ("ni", &["N", "I"]), ("nu", &["N", "U"]), ("ne", &["N", "E"]), ("no", &["N", "O"]),
    ("ha", &["HH", "A"]), ("hi", &["HH", "I"]), ("fu", &["F", "U"]), ("hu", &["F", "U"]), ("he", &["HH", "E"]), ("ho", &["HH", "O"]),
    ("ma", &["M", "A"]), ("mi", &["M", "I"]), ("mu", &["M", "U"]), ("me", &["M", "E"]), ("mo", &["M", "O"]),
    ("ya", &["Y", "A"]), ("yu", &["Y", "U"]), ("yo", &["Y", "O"]),
    ("ra", &["DX", "A"]), ("ri", &["DX", "I"]), ("ru", &["DX", "U"]), ("re", &["DX", "E"]), ("ro", &["DX", "O"]),
    ("wa", &["W", "A"]), ("wo", &["O"]), ("nn", &["N"]), ("n'", &["N"]),
    ("ga", &["G", "A"]), ("gi", &["G", "I"]), ("gu", &["G", "U"]), ("ge", &["G", "E"]), ("go", &["G", "O"]),
    ("za", &["Z", "A"]), ("ji", &["JH", "I"]), ("zi", &["JH", "I"]), ("zu", &["Z", "U"]), ("ze", &["Z", "E"]), ("zo", &["Z", "O"]),
    ("da", &["D", "A"]), ("di", &["JH", "I"]), ("du", &["Z", "U"]), ("de", &["D", "E"]), ("do", &["D", "O"]),
    ("ba", &["B", "A"]), ("bi", &["B", "I"]), ("bu", &["B", "U"]), ("be", &["B", "E"]), ("bo", &["B", "O"]),
    ("pa", &["P", "A"]), ("pi", &["P", "I"]), ("pu", &["P", "U"]), ("pe", &["P", "E"]), ("po", &["P", "O"]),
    ("kya", &["K", "Y", "A"]), ("kyu", &["K", "Y", "U"]), ("kyo", &["K", "Y", "O"]),
    ("sha", &["SH", "A"]), ("shu", &["SH", "U"]), ("sho", &["SH", "O"]),
    ("cha", &["CH", "A"]), ("chu", &["CH", "U"]), ("cho", &["CH", "O"]),
    ("nya", &["N", "Y", "A"]), ("nyu", &["N", "Y", "U"]), ("nyo", &["N", "Y", "O"]),
    ("hya", &["HH", "Y", "A"]), ("hyu", &["HH", "Y", "U"]), ("hyo", &["HH", "Y", "O"]),
    ("mya", &["M", "Y", "A"]), ("myu", &["M", "Y", "U"]), ("myo", &["M", "Y", "O"]),
    ("rya", &["DX", "Y", "A"]), ("ryu", &["DX", "Y", "U"]), ("ryo", &["DX", "Y", "O"]),
    ("gya", &["G", "Y", "A"]), ("gyu", &["G", "Y", "U"]), ("gyo", &["G", "Y", "O"]),
    ("ja", &["JH", "A"]), ("ju", &["JH", "U"]), ("jo", &["JH", "O"]),
    ("bya", &["B", "Y", "A"]), ("byu", &["B", "Y", "U"]), ("byo", &["B", "Y", "O"]),
    ("pya", &["P", "Y", "A"]), ("pyu", &["P", "Y", "U"]), ("pyo", &["P", "Y", "O"]),
];

// Keys as UTF-16, sorted by length descending for greedy matching (stable, so
// equal lengths keep table order like the JS sort).
static ROMAJI_KEYS: LazyLock<Vec<(Vec<u16>, &'static [&'static str])>> = LazyLock::new(|| {
    let mut keys: Vec<_> = ROMAJI_MAP.iter().map(|(k, v)| (k.encode_utf16().collect::<Vec<u16>>(), *v)).collect();
    keys.sort_by_key(|k| std::cmp::Reverse(k.0.len()));
    keys
});

fn unit_in(u: Option<u16>, set: &str) -> bool {
    u.is_some_and(|u| set.encode_utf16().any(|s| s == u))
}

// Romaji scanning works on UTF-16 code units, as the JS does, so that
// doubled-character (geminate) detection treats astral characters the same.
pub fn romaji_to_phonemes(text: &str) -> Option<Vec<Phone>> {
    let lower: Vec<u16> = text.to_lowercase().chars().filter(|c| !js::is_space(*c)).collect::<String>().encode_utf16().collect();
    let at = |j: usize| lower.get(j).copied();
    let n = u16::from(b'n');
    let mut result: Vec<Phone> = Vec::new();
    let mut i = 0;

    while i < lower.len() {
        // nn = syllabic N + next syllable starting with n
        if lower[i] == n && at(i + 1) == Some(n) {
            result.push(Phone::new("N"));
            i += 1;
            continue;
        }

        // geminate: doubled consonant (not nn)
        if i + 1 < lower.len() && lower[i] == lower[i + 1] && !unit_in(Some(lower[i]), "aeioun") {
            result.push(Phone::new("_"));
            i += 1;
            continue;
        }

        // n before consonant or end (not followed by a vowel or y)
        if lower[i] == n && i + 1 < lower.len() && !unit_in(at(i + 1), "aeiouny") {
            result.push(Phone::new("N"));
            i += 1;
            continue;
        }
        if lower[i] == n && i + 1 == lower.len() {
            result.push(Phone::new("N"));
            i += 1;
            continue;
        }

        let matched = ROMAJI_KEYS.iter().find(|(key, _)| lower[i..].starts_with(key));
        match matched {
            Some((key, codes)) => {
                result.extend(phones(codes));
                i += key.len();
            }
            None => i += 1,
        }
    }

    (!result.is_empty()).then_some(result)
}

pub fn is_kana(text: &str) -> bool {
    text.chars().any(|c| ('\u{3040}'..='\u{30FF}').contains(&c))
}

pub fn japanese_to_phonemes(text: &str) -> Option<Vec<Phone>> {
    if is_kana(text) {
        return kana_to_phonemes(text);
    }
    let compact: Vec<char> = text.chars().filter(|c| !js::is_space(*c)).collect();
    if !compact.is_empty() && compact.iter().all(|c| c.is_ascii_alphabetic() || *c == '\'') {
        return romaji_to_phonemes(text);
    }
    kana_to_phonemes(text)
}
