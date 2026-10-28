// Standalone WAV renderer
//
//   klattsch "HH AH L OW" hello.wav
//   klattsch "b140 AY+30 . AY-30" sweep.wav
//
// Same arguments and output as the JS bin/klattsch.mjs, plus options for the
// sample rate, phoneme banks, English/Japanese text input, and JSON output.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use klattsch_core::{
    compile_string_with_registry, encode_wav, english_to_phonemes, japanese_to_phonemes, render_mix, BankRef,
    BankRegistry, CmuDict, CompileOptions, EnglishOptions, Lexicon, Phone, WavMetadata, WavOptions, WordSource, SOFTWARE,
};

const DEFAULT_DICT: &str = "data/cmudict/cmudict-0.7b";

const USAGE: &str = "usage: klattsch <phoneme-string> [output.wav]
  e.g. klattsch \"HH AH L OW\" hello.wav

options:
  --sample-rate <hz>   output sample rate (default 48000)
  --bank <name>        initial phoneme bank (default klatt1980-en)
  --load-bank <file>   register a bank JSON file (repeatable)
  --english            input is English text: punctuation pauses, numbers, CMU dictionary
                       with letter-to-sound fallback, and sentence intonation
  --dict <file>        CMU dictionary for --english (default data/cmudict/cmudict-0.7b)
  --words <file>       custom word list in CMU format, checked before the dictionary
                       (repeatable; earlier files win)
  --no-intonation      with --english, keep a flat pitch (pauses only)
  --japanese           input is kana or romaji (use with a ja-* bank)
  --json               print the compiled schedule as JSON instead of writing a WAV";

enum Input {
    Phonemes,
    English,
    Japanese,
}

struct Args {
    text: String,
    out_path: PathBuf,
    sample_rate: u32,
    bank: Option<String>,
    load_banks: Vec<PathBuf>,
    input: Input,
    dict: PathBuf,
    words: Vec<PathBuf>,
    intonation: bool,
    json: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut positional = Vec::new();
    let mut args = Args {
        text: String::new(),
        out_path: PathBuf::from("klattsch.wav"),
        sample_rate: 48000,
        bank: None,
        load_banks: Vec::new(),
        input: Input::Phonemes,
        dict: PathBuf::from(DEFAULT_DICT),
        words: Vec::new(),
        intonation: true,
        json: false,
    };
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        let mut value = |name: &str| it.next().ok_or_else(|| format!("{name} needs a value"));
        match a.as_str() {
            "--sample-rate" => {
                let v = value("--sample-rate")?;
                args.sample_rate = v.parse().ok().filter(|r| *r > 0).ok_or(format!("bad sample rate: {v}"))?;
            }
            "--bank" => args.bank = Some(value("--bank")?),
            "--load-bank" => args.load_banks.push(value("--load-bank")?.into()),
            "--dict" => args.dict = value("--dict")?.into(),
            "--words" => args.words.push(value("--words")?.into()),
            "--english" => args.input = Input::English,
            "--japanese" => args.input = Input::Japanese,
            "--no-intonation" => args.intonation = false,
            "--json" => args.json = true,
            "-h" | "--help" => return Err(String::new()),
            _ if a.starts_with("--") => return Err(format!("unknown option: {a}")),
            _ => positional.push(a),
        }
    }
    let mut positional = positional.into_iter();
    args.text = positional.next().filter(|t| !t.is_empty()).ok_or_else(String::new)?;
    if let Some(out) = positional.next() {
        args.out_path = out.into();
    }
    Ok(args)
}

// Phones as a klattsch phoneme string, marking stress with `'`.
fn phone_string(phones: &[Phone]) -> String {
    phones
        .iter()
        .map(|p| if p.stressed { format!("{}'", p.code) } else { p.code.clone() })
        .collect::<Vec<_>>()
        .join(" ")
}

// Number.prototype.toFixed: like `{:.N}` but ties round away from zero.
fn to_fixed(x: f64, digits: usize) -> String {
    let exact = format!("{:.*}", digits + 1100, x.abs());
    let (keep, rest) = exact.split_at(exact.find('.').unwrap() + 1 + digits);
    let tie = rest.starts_with('5') && rest[1..].bytes().all(|b| b == b'0');
    if tie {
        let step = 10f64.powi(-(digits as i32));
        let up = format!("{:.*}", digits, keep.parse::<f64>().unwrap() + step);
        return if x < 0.0 { format!("-{up}") } else { up };
    }
    format!("{:.*}", digits, x)
}

fn run(args: Args) -> Result<(), String> {
    let registry = BankRegistry::new();
    for path in &args.load_banks {
        let name = registry.load_file(path).map_err(|e| e.to_string())?;
        eprintln!("loaded bank {name} from {}", path.display());
    }

    let text = match args.input {
        Input::Phonemes => args.text.clone(),
        Input::English => {
            let mut lexicon = Lexicon::new();
            for path in args.words.iter().chain([&args.dict]) {
                lexicon.push_layer(Arc::new(CmuDict::load(path).map_err(|e| e.to_string())?));
            }
            let opts = EnglishOptions { intonation: args.intonation, ..Default::default() };
            let spoken = english_to_phonemes(&args.text, &lexicon, &opts);
            // Report every word that was not a plain dictionary hit.
            let notes: Vec<String> = spoken
                .words
                .iter()
                .filter_map(|w| match &w.source {
                    WordSource::Parts(parts) => Some(format!("{} = {}", w.text, parts.join(" + "))),
                    WordSource::LetterToSound => Some(format!("{} (sounded out)", w.text)),
                    WordSource::Acronym => Some(format!("{} (spelled)", w.text)),
                    _ => None,
                })
                .collect();
            if !notes.is_empty() {
                eprintln!("not in dictionary: {}", notes.join(", "));
            }
            eprintln!("phonemes: {}", spoken.phonemes);
            spoken.phonemes
        }
        Input::Japanese => {
            let s = phone_string(&japanese_to_phonemes(&args.text).unwrap_or_default());
            eprintln!("phonemes: {s}");
            s
        }
    };

    let opts = CompileOptions { bank: args.bank.map(BankRef::Name), ..Default::default() };
    let result = compile_string_with_registry(&text, &opts, &registry).map_err(|e| e.to_string())?;
    if !result.warnings.is_empty() {
        eprintln!("warnings: {}", result.warnings.join(", "));
    }

    if args.json {
        println!("{}", serde_json::to_string_pretty(&result.to_json()).unwrap());
        return Ok(());
    }

    // Render each voice section to its own buffer and sum. encode_wav peak-normalizes
    // the mix, so summing pre-clipped voices is safe.
    let buf = render_mix(&result, f64::from(args.sample_rate)).map_err(|e| e.to_string())?;
    let wav = encode_wav(&buf, args.sample_rate, &WavOptions {
        metadata: Some(WavMetadata { software: Some(SOFTWARE.into()), comment: Some(args.text.clone()) }),
        ..WavOptions::default()
    });
    std::fs::write(&args.out_path, &wav.bytes)
        .map_err(|e| format!("cannot write {}: {e}", args.out_path.display()))?;
    eprintln!(
        "wrote {}: {} KB, {}s, normalize gain {}x",
        args.out_path.display(),
        to_fixed(wav.bytes.len() as f64 / 1024.0, 0),
        to_fixed(result.total_ms / 1000.0, 2),
        to_fixed(wav.gain, 2),
    );
    Ok(())
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(a) => a,
        Err(msg) => {
            if !msg.is_empty() {
                eprintln!("{msg}");
            }
            eprintln!("{USAGE}");
            return ExitCode::from(1);
        }
    };
    match run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::from(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::to_fixed;

    #[test]
    fn to_fixed_rounds_ties_up() {
        assert_eq!(to_fixed(2.125, 2), "2.13");
        assert_eq!(to_fixed(2.5, 0), "3");
        assert_eq!(to_fixed(1.005, 2), "1.00"); // 1.005 is below the tie in binary
        assert_eq!(to_fixed(0.59, 2), "0.59");
        assert_eq!(to_fixed(-2.5, 0), "-3");
    }
}
