// Stand-in for the npm `cmu-pronouncing-dictionary` package, filled from a
// cmudict text file. Parsing rules must match klattsch-core/src/pronounce.rs:
//   - lines are split on \n (a trailing \r is dropped)
//   - blank lines and lines starting with ';;;' are skipped
//   - the first whitespace-delimited field is the word, lowercased
//   - the remaining fields are the phones, up to a field starting with '#'
//   - lines with no phones are skipped; the first entry for a word wins

export const dictionary = Object.create(null);

export function loadDictionary(text) {
  for (const k of Object.keys(dictionary)) delete dictionary[k];
  for (let line of text.split('\n')) {
    if (line.endsWith('\r')) line = line.slice(0, -1);
    if (line.startsWith(';;;')) continue;
    const fields = line.split(/[ \t]+/).filter(Boolean);
    if (fields.length < 2) continue;
    const word = fields[0].toLowerCase();
    const phones = [];
    for (const f of fields.slice(1)) {
      if (f.startsWith('#')) break;
      phones.push(f);
    }
    if (!phones.length || word in dictionary) continue;
    dictionary[word] = phones.join(' ');
  }
}
