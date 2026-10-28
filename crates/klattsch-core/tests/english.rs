// Tests for the English text front end and the NRL letter-to-sound rules.

use std::path::PathBuf;
use std::sync::{Arc, LazyLock};

use klattsch_core::*;

static DICT: LazyLock<CmuDict> = LazyLock::new(|| {
    CmuDict::load(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/cmudict/cmudict-0.7b")).unwrap()
});

const PACER: &str = "The FitnessGram Pacer Test is a multistage aerobic capacity test that progressively gets \
more difficult as it continues. The 20 meter pacer test will begin in 30 seconds. Line up at the start. The \
running speed starts slowly but gets faster each minute after you hear this signal bodeboop. A sing lap should be \
completed every time you hear this sound. ding Remember to run in a straight line and run as long as possible. The \
second time you fail to complete a lap before the sound, your test is over. The test will begin on the word start. \
On your mark. Get ready!\u{2026} Start. ding\u{FEFF} ";

fn speak(text: &str) -> EnglishResult {
    english_to_phonemes(text, &*DICT, &EnglishOptions::default())
}

#[track_caller]
fn compiles_cleanly(phonemes: &str) {
    let r = compile_string(phonemes, &CompileOptions::default()).unwrap();
    assert!(r.warnings.is_empty(), "{:?} in {phonemes}", r.warnings);
}

// The rule engine reproduces the reference implementation's output.
#[test]
fn nrl_rules_match_reference() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/nrl-reference.tsv");
    let mut bad = Vec::new();
    let mut n = 0;
    for line in std::fs::read_to_string(path).unwrap().lines().filter(|l| !l.starts_with('#')) {
        let (word, want) = line.split_once('\t').unwrap();
        let got = lts::rule_phonemes(word).join(" ");
        n += 1;
        if got != want {
            bad.push(format!("{word}: {got} vs {want}"));
        }
    }
    assert!(bad.is_empty(), "{} of {n} differ: {:?}", bad.len(), &bad[..bad.len().min(10)]);
    assert!(n > 4000);
}

#[test]
fn pacer_text() {
    let r = speak(PACER);
    compiles_cleanly(&r.phonemes);

    // Nothing is dropped.
    assert!(r.words.iter().all(|w| w.source != WordSource::Skipped && !w.phones.is_empty()));
    let source = |t: &str| r.words.iter().find(|w| w.text == t).unwrap().source.clone();
    let parts = |p: &[&str]| WordSource::Parts(p.iter().map(|s| s.to_string()).collect());
    assert_eq!(source("FitnessGram"), parts(&["Fitness", "Gram"]));
    assert_eq!(source("multistage"), parts(&["multi", "stage"]));
    assert_eq!(source("bodeboop"), parts(&["bode", "boop"]));
    assert_eq!(source("20"), WordSource::Number);
    assert_eq!(source("30"), WordSource::Number);

    // Punctuation became pauses: 11 sentence ends, one comma.
    let tokens: Vec<&str> = r.phonemes.split(' ').collect();
    assert_eq!(tokens.iter().filter(|t| **t == ".").count(), 11);
    assert_eq!(tokens.iter().filter(|t| **t == ",").count(), 1);
}

#[test]
fn function_words_are_unstressed() {
    let r = speak("the dog is in the house");
    let stressed: Vec<&str> =
        r.words.iter().filter(|w| w.phones.iter().any(|p| p.stressed)).map(|w| w.text.as_str()).collect();
    assert_eq!(stressed, ["dog", "house"]);

    let keep = english_to_phonemes("the dog is", &*DICT, &EnglishOptions { unstress_function_words: false, ..Default::default() });
    assert!(keep.words[2].phones.iter().any(|p| p.stressed), "is keeps its dictionary stress");
}

#[test]
fn melody() {
    let statement = speak("The dog barked.").phonemes;
    assert!(statement.starts_with("b "), "{statement}");
    assert!(statement.contains("AA'-25"), "final fall on the last accent: {statement}");
    assert!(statement.ends_with(" ."), "{statement}");

    let question = speak("Is the dog home?").phonemes;
    assert!(question.contains("OW'+30"), "{question}");

    let comma = speak("Well, maybe.").phonemes;
    assert!(comma.contains("EH'(+10) L b-15 ,"), "continuation rise before the comma: {comma}");

    let exclaim = speak("Start!").phonemes;
    assert!(exclaim.contains("AA'-35"), "{exclaim}");

    // Declination: pitch steps down between accents of a long sentence.
    let long = speak("Large brown dogs run fast.").phonemes;
    assert!(long.contains("b-3.75"), "{long}");

    let flat = english_to_phonemes("The dog barked. Is it?", &*DICT, &EnglishOptions { intonation: false, ..Default::default() });
    assert_eq!(flat.phonemes, "DH AH D AO' G B AA' R K T . IH Z IH T .");
    for s in [statement, question, comma, exclaim, long, flat.phonemes] {
        compiles_cleanly(&s);
    }
}

#[test]
fn word_fallbacks() {
    let r = speak("QXZ pacer's well-known zzqq glorptastic");
    let by = |t: &str| r.words.iter().find(|w| w.text == t).unwrap();
    assert_eq!(by("QXZ").source, WordSource::Acronym);
    assert_eq!(by("QXZ").phones.iter().map(|p| p.code.as_str()).collect::<Vec<_>>().join(" "), "K Y UW EH K S Z IY");
    assert_eq!(by("pacer's").source, WordSource::Parts(vec!["pacer".into(), "'s".into()]));
    assert_eq!(by("pacer's").phones.last().unwrap().code, "Z");
    assert!(matches!(by("well-known").source, WordSource::Dictionary | WordSource::Parts(_)));
    assert_eq!(by("zzqq").source, WordSource::LetterToSound);
    assert_eq!(by("glorptastic").source, WordSource::LetterToSound);
    assert!(by("glorptastic").phones.iter().filter(|p| p.stressed).count() == 1);
    compiles_cleanly(&r.phonemes);

    let skip = english_to_phonemes("zzqq dog", &*DICT, &EnglishOptions { sound_out_unknown: false, ..Default::default() });
    assert_eq!(skip.words[0].source, WordSource::Skipped);
    compiles_cleanly(&skip.phonemes);
}

// Whatever the input, the output is valid klattsch syntax.
#[test]
fn output_always_compiles() {
    let inputs = [
        "", "   ", "...", "?!", ",,,", "Hello", "Hello.", "...Hello...", "A, b; c: d. E! F? G\u{2026}",
        "It's 3:45 p.m. on the 21st, 2024 - 50% off!", "\u{201C}Quoted\u{201D} (parenthetical) \u{2014} dash",
        "Ünïcödé café naïve", "\u{1F600} emoji \u{1F600}", "tab\tand\nnewline", "12,345,678.09", "-3 below",
        "O'Brien's well-to-do co-op", "x", "I", "a", "THE END",
    ];
    for text in inputs {
        let r = speak(text);
        compiles_cleanly(&r.phonemes);
        let flat = english_to_phonemes(text, &*DICT, &EnglishOptions { intonation: false, ..Default::default() });
        compiles_cleanly(&flat.phonemes);
    }
}

fn sources(r: &EnglishResult) -> Vec<(String, WordSource)> {
    r.words.iter().map(|w| (w.text.clone(), w.source.clone())).collect()
}

fn expansion(words: &[&str]) -> WordSource {
    WordSource::Expansion(words.iter().map(|s| s.to_string()).collect())
}

fn codes(phones: &[Phone]) -> String {
    phones.iter().map(|p| p.code.as_str()).collect::<Vec<_>>().join(" ")
}

#[track_caller]
fn sentence_ends(text: &str, want: usize) -> EnglishResult {
    let r = speak(text);
    compiles_cleanly(&r.phonemes);
    let ends = r.phonemes.split(' ').filter(|t| *t == ".").count();
    assert_eq!(ends, want, "sentence ends in {text:?}: {}", r.phonemes);
    r
}

#[test]
fn abbreviations_keep_the_sentence_going() {
    let r = sentence_ends("Dr. Smith lives on Main St. near the U.S. border.", 1);
    let s = sources(&r);
    assert!(s.contains(&("Dr".into(), expansion(&["doctor"]))), "{s:?}");
    assert!(s.contains(&("St".into(), expansion(&["street"]))), "{s:?}");
    assert!(s.contains(&("U.S.".into(), WordSource::Acronym)), "{s:?}");

    let r = sentence_ends("We flew to St. Louis. Mr Jones met John F. Kennedy at 5 Elm Dr in Jan. last year.", 2);
    let s = sources(&r);
    assert!(s.contains(&("St".into(), expansion(&["saint"]))), "{s:?}");
    assert!(s.contains(&("Mr".into(), expansion(&["mister"]))), "{s:?}");
    assert!(s.contains(&("F".into(), WordSource::Acronym)), "{s:?}");
    assert!(s.contains(&("Dr".into(), expansion(&["drive"]))), "{s:?}");
    assert!(s.contains(&("Jan".into(), expansion(&["january"]))), "{s:?}");

    // The period still ends the sentence before a capital or at the end.
    let r = sentence_ends("Apples, pears, etc. Then more etc. and so on, e.g. figs, i.e. fruit. See No. 5 vs. ch. 3.", 3);
    let s = sources(&r);
    for (text, words) in [
        ("etc", &["et", "cetera"][..]),
        ("e.g.", &["for", "example"]),
        ("i.e.", &["that", "is"]),
        ("No", &["number"]),
        ("vs", &["versus"]),
        ("ch", &["chapter"]),
    ] {
        assert!(s.contains(&(text.into(), expansion(words))), "{text}: {s:?}");
    }

    // Not abbreviations here: a plain "no", and "Dr" with nothing after it.
    let r = sentence_ends("I said no. Ask the Dr.", 2);
    assert_eq!(r.words[2].source, WordSource::Dictionary);
    assert_eq!(r.words[5].source, WordSource::Dictionary);
}

#[test]
fn money_and_times() {
    // The phones of plain words, for comparison.
    let say = |words: &str| codes(&speak(words).words.iter().flat_map(|w| w.phones.clone()).collect::<Vec<_>>());
    let cases = [
        ("$4.50", "four dollars and fifty cents"),
        ("$1", "one dollar"),
        ("$0.99", "ninety nine cents"),
        ("$0.01", "one cent"),
        ("$0", "zero dollars"),
        ("$1,250.5", "one thousand two hundred fifty dollars and fifty cents"),
        ("$2.5 million", "two point five million dollars"),
        ("$3.125", "three point one two five dollars"),
        ("\u{00A3}3.01", "three pounds and one penny"),
        ("\u{00A3}2.50", "two pounds and fifty pence"),
        ("\u{20AC}12", "twelve euros"),
        ("3:30", "three thirty"),
        ("3:05", "three oh five"),
        ("10:00", "ten o'clock"),
        ("14:00", "fourteen hundred"),
        ("14:45", "fourteen forty five"),
    ];
    for (text, words) in cases {
        let r = speak(text);
        assert_eq!(r.words.len(), 1, "{text}: {:?}", sources(&r));
        assert_eq!(r.words[0].source, WordSource::Number, "{text}");
        assert_eq!(codes(&r.words[0].phones), say(words), "{text}");
    }
    // am/pm is spelled.
    for (text, words, letters) in [
        ("9pm", "nine", "P IY EH M"),
        ("7 AM", "seven", "EY EH M"),
        ("10:00 a.m.", "ten", "EY EH M"),
        ("3:30PM", "three thirty", "P IY EH M"),
    ] {
        let r = speak(text);
        assert_eq!(r.words.len(), 1, "{text}: {:?}", sources(&r));
        assert_eq!(codes(&r.words[0].phones), format!("{} {letters}", say(words)), "{text}");
    }
    // Not times: ratios, h:mm:ss, out-of-range values.
    for text in ["3:2", "1:02:03", "25:00", "9:75"] {
        assert!(speak(text).words.len() > 1, "{text}");
    }
    sentence_ends("Meet me at 3 p.m. Then we eat.", 2);
    sentence_ends("Meet me at 3 p.m. today.", 1);
}

#[test]
fn wh_questions_fall() {
    let wh = speak("Where are you going?").phonemes;
    assert!(wh.contains("OW'-25"), "{wh}");
    let yes_no = speak("Are you going?").phonemes;
    assert!(yes_no.contains("OW'+30"), "{yes_no}");
    let opts = EnglishOptions { wh_questions_fall: false, ..Default::default() };
    let rising = english_to_phonemes("Where are you going?", &*DICT, &opts);
    assert!(rising.phonemes.contains("OW'+30"), "{}", rising.phonemes);
}

#[test]
fn acronyms_and_camel_case() {
    let r = speak("NPCs and URLs in GDExtension and XMLHttpRequest");
    let s = sources(&r);
    let parts = |p: &[&str]| WordSource::Parts(p.iter().map(|s| s.to_string()).collect());
    assert!(s.contains(&("NPCs".into(), parts(&["NPC", "s"]))), "{s:?}");
    assert!(s.contains(&("GDExtension".into(), parts(&["GD", "Extension"]))), "{s:?}");
    assert_eq!(codes(&r.words[0].phones), "EH N P IY S IY Z");
    compiles_cleanly(&r.phonemes);
}

#[test]
fn inline_phones() {
    let r = speak("I {R IY1 D} books, {k l ae' ch}! {} {??}");
    let s = sources(&r);
    assert_eq!(s[1], ("R IY1 D".into(), WordSource::Inline));
    assert_eq!(s[3], ("k l ae' ch".into(), WordSource::Inline));
    assert_eq!(s.len(), 4, "empty or invalid braces say nothing: {s:?}");
    assert!(r.phonemes.contains("R IY' D"), "{}", r.phonemes);
    assert!(r.phonemes.contains("K L AE'-35 CH"), "{}", r.phonemes);
    compiles_cleanly(&r.phonemes);
    // An unclosed brace is ignored.
    compiles_cleanly(&speak("odd { brace").phonemes);
}

#[test]
fn lexicon_layers() {
    let cmu = Arc::new(
        CmuDict::load(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/cmudict/cmudict-0.7b")).unwrap(),
    );
    let game = Arc::new(CmuDict::parse(";;; game words\nKLATTSCH  K L AE1 CH\nREAD  R IY1 D\n"));
    let mut lex = Lexicon::new();
    lex.push_layer(game);
    lex.push_layer(cmu.clone());
    assert_eq!(lex.layers().len(), 2);

    let r = english_to_phonemes("Klattsch can read", &lex, &EnglishOptions::default());
    assert!(r.words.iter().all(|w| w.source == WordSource::Dictionary));
    assert_eq!(lex.entry("read"), Some("R IY1 D"), "the game list wins over CMU");
    assert_eq!(lex.entry("hello"), cmu.entry("hello"));

    // A character's own words, over shared layers.
    let mut npc = lex.clone();
    npc.add_word("Zyx", "Z IH1 K S").unwrap();
    npc.add_word("klattsch", "K L AA1 CH").unwrap();
    assert_eq!(npc.entry("ZYX"), Some("Z IH1 K S"));
    assert_eq!(npc.entry("klattsch"), Some("K L AA1 CH"));
    assert_eq!(lex.entry("klattsch"), Some("K L AE1 CH"), "the original lexicon is unchanged");
    assert!(npc.has_word("zyx") && !lex.has_word("zyx"));
    assert_eq!(npc.words().len(), 2);
    assert!(Arc::ptr_eq(&npc.layers()[1], &cmu), "layers are shared, not copied");
    assert!(npc.remove_word("zyx") && !npc.remove_word("zyx"));

    for (word, phones) in [("", "AA1"), ("two words", "AA1"), ("x", ""), ("x", "AA1 NOPE")] {
        assert!(npc.add_word(word, phones).is_err(), "{word:?} {phones:?}");
    }
    let mut d = CmuDict::default();
    assert!(d.is_empty());
    d.insert("a", "EY1").unwrap();
    d.insert("A", "AH0").unwrap();
    assert_eq!(d.entry("a"), Some("AH0"), "insert replaces");
    assert!(Pronounce::pronounce(&d, "a").is_some());
    assert!(Pronounce::pronounce(&Lexicon::new(), "a").is_none());
}
