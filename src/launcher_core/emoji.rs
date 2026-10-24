//! 03 Emoji data and search. See .scratch/launcher-features/issues/ for the ticket.
//!
//! The table is generated from nixpkgs `unicode-emoji` and `cldr-annotations` by
//! `scripts/generate-emoji-data.py` and embedded here, so the launcher reads no Unicode files,
//! downloads nothing and never needs devenv. Search is pure: it ranks the embedded table against
//! the typed text and returns references into it. Recents (frecency) belong to the launcher, which
//! keys `src/launch_history.rs` by [`Emoji::id`].
//!
//! Skin tones are not separate emoji. Each emoji that has them carries its five variants, picked
//! with [`Emoji::with_tone`], and [`find`] maps a variant glyph back to its base emoji.

use std::cell::RefCell;
use std::cmp::Reverse;
use std::collections::HashMap;
use std::sync::OnceLock;

use nucleo_matcher::pattern::{Atom, AtomKind, CaseMatching, Normalization};
use nucleo_matcher::{Matcher, Utf32String};

/// The generated table. Tab separated records in CLDR order; see the generator for the format.
const DATA: &str = include_str!("emoji-data.txt");

/// Emoji group, in the order Unicode lists them (the launcher's default order).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Group {
    SmileysAndEmotion,
    PeopleAndBody,
    AnimalsAndNature,
    FoodAndDrink,
    TravelAndPlaces,
    Activities,
    Objects,
    Symbols,
    Flags,
}

impl Group {
    pub const ALL: [Group; 9] = [
        Group::SmileysAndEmotion,
        Group::PeopleAndBody,
        Group::AnimalsAndNature,
        Group::FoodAndDrink,
        Group::TravelAndPlaces,
        Group::Activities,
        Group::Objects,
        Group::Symbols,
        Group::Flags,
    ];

    /// The name Unicode gives the group, which is also what the data file stores.
    pub fn label(self) -> &'static str {
        match self {
            Group::SmileysAndEmotion => "Smileys & Emotion",
            Group::PeopleAndBody => "People & Body",
            Group::AnimalsAndNature => "Animals & Nature",
            Group::FoodAndDrink => "Food & Drink",
            Group::TravelAndPlaces => "Travel & Places",
            Group::Activities => "Activities",
            Group::Objects => "Objects",
            Group::Symbols => "Symbols",
            Group::Flags => "Flags",
        }
    }

    fn from_label(label: &str) -> Option<Group> {
        Group::ALL.into_iter().find(|group| group.label() == label)
    }
}

/// A skin tone, light to dark, in Unicode order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Tone {
    Light,
    MediumLight,
    Medium,
    MediumDark,
    Dark,
}

impl Tone {
    pub const ALL: [Tone; 5] = [
        Tone::Light,
        Tone::MediumLight,
        Tone::Medium,
        Tone::MediumDark,
        Tone::Dark,
    ];

    /// The name Unicode gives the tone ("light skin tone").
    pub fn label(self) -> &'static str {
        match self {
            Tone::Light => "light skin tone",
            Tone::MediumLight => "medium-light skin tone",
            Tone::Medium => "medium skin tone",
            Tone::MediumDark => "medium-dark skin tone",
            Tone::Dark => "dark skin tone",
        }
    }

    fn index(self) -> usize {
        self as usize
    }
}

/// One emoji with the text people search it by.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Emoji {
    /// The text to copy, as Unicode writes it (a fully-qualified sequence).
    pub glyph: String,
    /// Unicode's name, such as "face with tears of joy".
    pub name: String,
    pub group: Group,
    /// Unicode's subgroup, such as "face-smiling".
    pub subgroup: String,
    /// The Unicode emoji version that introduced it, such as "13.1".
    pub version: String,
    /// CLDR short names and the spoken name, lowercase, without the name itself.
    pub keywords: Vec<String>,
    /// The five skin-tone variants from light to dark, or empty when the emoji has none.
    pub tones: Vec<String>,
}

impl Emoji {
    /// The stable key for recents: the base glyph, whichever tone was copied.
    pub fn id(&self) -> &str {
        &self.glyph
    }

    pub fn has_tones(&self) -> bool {
        !self.tones.is_empty()
    }

    /// The skin-tone variants in order, empty when the emoji has none.
    pub fn tone_variants(&self) -> impl Iterator<Item = (Tone, &str)> {
        Tone::ALL
            .into_iter()
            .zip(self.tones.iter().map(String::as_str))
    }

    /// The glyph in one tone, or the plain glyph when the emoji has no tones.
    pub fn with_tone(&self, tone: Tone) -> &str {
        self.tones
            .get(tone.index())
            .map_or(&self.glyph, String::as_str)
    }
}

/// Lowercased text prepared once so a search only compares and scores.
struct Searchable {
    name: String,
    name_chars: usize,
    name_words: Vec<String>,
    name_fuzzy: Utf32String,
    keywords: Vec<String>,
    keyword_words: Vec<String>,
    keywords_fuzzy: Vec<Utf32String>,
}

struct Table {
    emoji: Vec<Emoji>,
    searchable: Vec<Searchable>,
    /// Base and variant glyphs to the index of their base emoji.
    by_glyph: HashMap<String, usize>,
}

fn words(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| !c.is_alphanumeric() && c != '+')
        .filter(|word| !word.is_empty())
}

impl Searchable {
    fn new(emoji: &Emoji) -> Self {
        let name = emoji.name.to_ascii_lowercase();
        let keywords: Vec<String> = emoji
            .keywords
            .iter()
            .map(|keyword| keyword.to_ascii_lowercase())
            .collect();
        let mut keyword_words: Vec<String> = keywords
            .iter()
            .flat_map(|keyword| words(keyword).map(str::to_owned))
            .collect();
        keyword_words.sort_unstable();
        keyword_words.dedup();
        Searchable {
            name_chars: name.chars().count(),
            name_words: words(&name).map(str::to_owned).collect(),
            name_fuzzy: Utf32String::from(name.as_str()),
            keywords_fuzzy: keywords
                .iter()
                .map(|keyword| Utf32String::from(keyword.as_str()))
                .collect(),
            name,
            keywords,
            keyword_words,
        }
    }
}

/// Parse the generated table. Strict: a malformed record is an error naming its line.
fn parse(data: &str) -> Result<Vec<Emoji>, String> {
    let mut emoji = Vec::new();
    let mut group = None;
    let mut subgroup = None;
    for (number, line) in data.lines().enumerate() {
        let at = |message: &str| format!("emoji data line {}: {message}", number + 1);
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(label) = line.strip_prefix("@group ") {
            group = Some(Group::from_label(label).ok_or_else(|| at("unknown group"))?);
            subgroup = None;
            continue;
        }
        if let Some(label) = line.strip_prefix("@subgroup ") {
            subgroup = Some(label.to_owned());
            continue;
        }
        let mut fields = line.split('\t');
        let (Some(glyph), Some(name), Some(version), Some(keywords)) =
            (fields.next(), fields.next(), fields.next(), fields.next())
        else {
            return Err(at("expected at least four tab separated fields"));
        };
        let tones: Vec<String> = fields
            .next()
            .unwrap_or("")
            .split(' ')
            .filter(|tone| !tone.is_empty())
            .map(str::to_owned)
            .collect();
        if fields.next().is_some() {
            return Err(at("more than five fields"));
        }
        if glyph.is_empty() || name.is_empty() {
            return Err(at("empty glyph or name"));
        }
        if !tones.is_empty() && tones.len() != Tone::ALL.len() {
            return Err(at("tones must be empty or all five"));
        }
        emoji.push(Emoji {
            glyph: glyph.to_owned(),
            name: name.to_owned(),
            group: group.ok_or_else(|| at("record before any @group"))?,
            subgroup: subgroup
                .clone()
                .ok_or_else(|| at("record before any @subgroup"))?,
            version: version.to_owned(),
            keywords: keywords
                .split('|')
                .filter(|keyword| !keyword.is_empty())
                .map(str::to_owned)
                .collect(),
            tones,
        });
    }
    Ok(emoji)
}

fn table() -> &'static Table {
    static TABLE: OnceLock<Table> = OnceLock::new();
    TABLE.get_or_init(|| {
        let emoji = parse(DATA).expect("the embedded emoji data is generated and checked in");
        let searchable = emoji.iter().map(Searchable::new).collect();
        let mut by_glyph = HashMap::new();
        for (index, entry) in emoji.iter().enumerate() {
            by_glyph.insert(entry.glyph.clone(), index);
        }
        // Variants second, so a glyph that is both a base and someone's variant stays a base.
        for (index, entry) in emoji.iter().enumerate() {
            for tone in &entry.tones {
                by_glyph.entry(tone.clone()).or_insert(index);
            }
        }
        Table {
            emoji,
            searchable,
            by_glyph,
        }
    })
}

/// Every emoji in Unicode's order (grouped, as a picker shows them). Parsed once.
pub fn all() -> &'static [Emoji] {
    &table().emoji
}

/// The emoji for a glyph, accepting either the base glyph or any of its skin-tone variants.
pub fn find(glyph: &str) -> Option<&'static Emoji> {
    let table = table();
    table.by_glyph.get(glyph).map(|&index| &table.emoji[index])
}

/// How well a query matches one emoji; lower sorts first. Fuzzy tiers also carry nucleo's score.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Tier {
    ExactName,
    /// The query is whole words of the name ("heart" in "red heart").
    NameWord,
    ExactKeyword,
    NamePrefix,
    KeywordPrefix,
    /// Every query word starts a word of the name ("tears joy").
    NameWords,
    /// Every query word starts a word of the name or the keywords ("laugh face").
    AnyWords,
    FuzzyName,
    FuzzyKeyword,
}

impl Tier {
    /// Name tiers prefer the shorter, plainer name. Keyword tiers say nothing about the name's
    /// length, so they keep Unicode's order, which lists the common emoji of a group first.
    fn prefers_short_name(self) -> bool {
        matches!(
            self,
            Tier::ExactName | Tier::NameWord | Tier::NamePrefix | Tier::NameWords | Tier::FuzzyName
        )
    }
}

struct Query<'a> {
    text: &'a str,
    words: Vec<&'a str>,
    atoms: Vec<Atom>,
}

impl<'a> Query<'a> {
    /// `text` is the lowercased, single-spaced query and `words` its words.
    fn new(text: &'a str, words: Vec<&'a str>) -> Self {
        let atoms = words
            .iter()
            .map(|word| {
                Atom::new(
                    word,
                    CaseMatching::Ignore,
                    Normalization::Smart,
                    AtomKind::Fuzzy,
                    false,
                )
            })
            .collect();
        Query { text, words, atoms }
    }

    /// Fuzzy score of the text, when every query word matches in it and none of them is scattered.
    ///
    /// Nucleo accepts any subsequence, so "tears" would match "smiling face with hearts". A word
    /// has to fit in a span of its own length plus half again, none extra for three letters or
    /// fewer (dropped letters and typos, not letters picked from across the text).
    fn fuzzy(
        &self,
        text: &Utf32String,
        matcher: &mut Matcher,
        scratch: &mut Vec<u32>,
    ) -> Option<u32> {
        let mut total = 0;
        for atom in &self.atoms {
            scratch.clear();
            total += u32::from(atom.indices(text.slice(..), matcher, scratch)?);
            let (first, last) = (scratch.iter().min()?, scratch.iter().max()?);
            let (span, len) = ((last - first + 1) as usize, atom.needle_text().len());
            let slack = if len <= 3 { 0 } else { len / 2 };
            if span > len + slack {
                return None;
            }
        }
        Some(total)
    }
}

fn classify(
    query: &Query,
    entry: &Searchable,
    matcher: &mut Matcher,
    scratch: &mut Vec<u32>,
) -> Option<(Tier, Reverse<u32>)> {
    let plain = |tier| Some((tier, Reverse(0)));
    if entry.name == query.text {
        return plain(Tier::ExactName);
    }
    let in_name = entry
        .name_words
        .windows(query.words.len())
        .any(|window| window.iter().zip(&query.words).all(|(a, b)| a == b));
    if in_name {
        return plain(Tier::NameWord);
    }
    if entry.keywords.iter().any(|keyword| keyword == query.text) {
        return plain(Tier::ExactKeyword);
    }
    if entry.name.starts_with(query.text) {
        return plain(Tier::NamePrefix);
    }
    if entry
        .keywords
        .iter()
        .any(|keyword| keyword.starts_with(query.text))
    {
        return plain(Tier::KeywordPrefix);
    }
    let starts = |pool: &[String], word: &str| pool.iter().any(|w| w.starts_with(word));
    if query
        .words
        .iter()
        .all(|word| starts(&entry.name_words, word))
    {
        return plain(Tier::NameWords);
    }
    if query
        .words
        .iter()
        .all(|word| starts(&entry.name_words, word) || starts(&entry.keyword_words, word))
    {
        return plain(Tier::AnyWords);
    }
    if let Some(score) = query.fuzzy(&entry.name_fuzzy, matcher, scratch) {
        return Some((Tier::FuzzyName, Reverse(score)));
    }
    entry
        .keywords_fuzzy
        .iter()
        .filter_map(|keyword| query.fuzzy(keyword, matcher, scratch))
        .max()
        .map(|score| (Tier::FuzzyKeyword, Reverse(score)))
}

thread_local! {
    /// Nucleo's scratch space is large; keep one per thread instead of allocating per keystroke.
    static MATCHER: RefCell<Matcher> = RefCell::new(Matcher::new(nucleo_matcher::Config::DEFAULT));
}

/// The best `limit` emoji for the typed text, best first.
///
/// An empty or blank query returns the first `limit` emoji in Unicode's grouped order. Otherwise
/// ranking is, from best: exact name, exact keyword, name prefix, keyword prefix, every query word
/// starting a word of the name, every query word starting a word of the name or keywords, fuzzy
/// match on the name, fuzzy match on a keyword. Case is ignored for ASCII. Within a tier a better
/// fuzzy score wins, then the shorter name, then Unicode's order, so results are deterministic.
pub fn search(query: &str, limit: usize) -> Vec<&'static Emoji> {
    let table = table();
    let lowered = query.to_ascii_lowercase();
    let words: Vec<&str> = lowered.split_whitespace().collect();
    if words.is_empty() {
        return table.emoji.iter().take(limit).collect();
    }
    if limit == 0 {
        return Vec::new();
    }
    let text = words.join(" ");
    let query = Query::new(&text, words);
    let mut scratch = Vec::new();
    let mut ranked: Vec<_> = MATCHER.with_borrow_mut(|matcher| {
        table
            .searchable
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| {
                let (tier, score) = classify(&query, entry, matcher, &mut scratch)?;
                let length = if tier.prefers_short_name() {
                    entry.name_chars
                } else {
                    0
                };
                Some((tier, score, length, index))
            })
            .collect()
    });
    ranked.sort_unstable();
    ranked
        .into_iter()
        .take(limit)
        .map(|(.., index)| &table.emoji[index])
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn glyphs(query: &str, limit: usize) -> Vec<&'static str> {
        search(query, limit)
            .into_iter()
            .map(|emoji| emoji.glyph.as_str())
            .collect()
    }

    fn names(query: &str, limit: usize) -> Vec<&'static str> {
        search(query, limit)
            .into_iter()
            .map(|emoji| emoji.name.as_str())
            .collect()
    }

    fn position(results: &[&Emoji], glyph: &str) -> Option<usize> {
        results.iter().position(|emoji| emoji.glyph == glyph)
    }

    #[test]
    fn the_embedded_table_parses_with_plausible_counts() {
        assert!(
            DATA.len() < 200_000,
            "data file grew to {} bytes",
            DATA.len()
        );
        let emoji = all();
        assert!(
            (1_800..2_100).contains(&emoji.len()),
            "{} emoji",
            emoji.len()
        );
        let with_tones = emoji.iter().filter(|emoji| emoji.has_tones()).count();
        assert!((250..450).contains(&with_tones), "{with_tones} with tones");
        for group in Group::ALL {
            assert!(emoji.iter().any(|emoji| emoji.group == group), "{group:?}");
        }
        let flags = emoji.iter().filter(|e| e.group == Group::Flags).count();
        assert!((250..300).contains(&flags), "{flags} flags");
    }

    #[test]
    fn every_emoji_is_complete_and_unique() {
        let mut seen = std::collections::HashSet::new();
        for emoji in all() {
            assert!(
                !emoji.glyph.is_empty() && !emoji.name.is_empty(),
                "{emoji:?}"
            );
            assert!(!emoji.subgroup.is_empty(), "{emoji:?}");
            assert!(
                emoji.version.split_once('.').is_some_and(|(major, minor)| {
                    major.parse::<u8>().is_ok() && minor.parse::<u8>().is_ok()
                }),
                "{emoji:?}"
            );
            assert!(
                emoji.tones.is_empty() || emoji.tones.len() == 5,
                "{emoji:?}"
            );
            assert!(seen.insert(&emoji.glyph), "duplicate {}", emoji.glyph);
            for tone in &emoji.tones {
                assert!(seen.insert(tone), "duplicate tone variant {tone}");
            }
            assert!(
                emoji.keywords.iter().all(|k| k == &k.to_ascii_lowercase()
                    && !k.is_empty()
                    && k != &emoji.name.to_ascii_lowercase()),
                "{emoji:?}"
            );
        }
        // No components or unqualified forms slipped through: a bare skin-tone swatch or hair
        // component would be a one-code-point glyph in these ranges.
        assert!(
            all()
                .iter()
                .all(|e| !('\u{1F3FB}'..='\u{1F3FF}').contains(&e.glyph.chars().next().unwrap()))
        );
    }

    #[test]
    fn default_order_is_grouped_and_starts_with_the_first_smiley() {
        let emoji = all();
        assert_eq!(emoji[0].glyph, "😀");
        assert_eq!(emoji[0].group, Group::SmileysAndEmotion);
        assert_eq!(emoji.last().unwrap().group, Group::Flags);
        // Groups never interleave: each appears as one run, in Unicode's order.
        let mut runs = emoji.iter().map(|e| e.group).collect::<Vec<_>>();
        runs.dedup();
        assert_eq!(runs, Group::ALL);
    }

    #[test]
    fn an_empty_query_lists_the_default_order_up_to_the_limit() {
        for query in ["", "   ", "\t"] {
            let results = search(query, 5);
            assert_eq!(results.len(), 5);
            assert!(results.iter().zip(all()).all(|(a, b)| std::ptr::eq(*a, b)));
        }
        assert_eq!(search("", 100_000).len(), all().len());
    }

    #[test]
    fn limit_caps_the_results_and_zero_returns_none() {
        assert_eq!(search("face", 7).len(), 7);
        assert!(search("face", 0).is_empty());
        assert!(search("", 0).is_empty());
        assert!(
            search("face", 3)
                .iter()
                .zip(search("face", 10))
                .all(|(a, b)| *a == b)
        );
    }

    #[test]
    fn grin_finds_grinning_faces_first() {
        let results = search("grin", 10);
        assert_eq!(results[0].glyph, "😀", "{:?}", names("grin", 10));
        // The leading results all say "grin" in their name or keywords.
        assert!(results.iter().take(6).all(|e| {
            e.name.contains("grin") || e.keywords.iter().any(|k| k.starts_with("grin"))
        }));
    }

    #[test]
    fn aliases_come_through_cldr_keywords() {
        let lol = search("lol", 5);
        assert!(position(&lol, "😂").is_some(), "{:?}", names("lol", 5));
        assert!(lol.iter().all(|e| e.keywords.iter().any(|k| k == "lol")));
        let sad = search("sad", 10);
        assert!(position(&sad, "😢").is_some(), "{:?}", names("sad", 10));
        let heart = search("heart", 5);
        assert!(position(&heart, "❤️").is_some(), "{:?}", names("heart", 5));
        assert!(search("lmao", 3).iter().any(|e| e.glyph == "😂"));
        assert!(search("+1", 3).iter().any(|e| e.glyph == "👍"));
    }

    #[test]
    fn an_exact_name_beats_keyword_and_prefix_matches() {
        let results = search("thumbs up", 5);
        assert_eq!(results[0].glyph, "👍");
        assert_eq!(results[0].name, "thumbs up");
        // "thumbs down" does not rank above the exact name, and nothing but thumbs matches first.
        assert!(results.iter().all(|e| e.name.contains("thumbs")));
        assert_eq!(search("red heart", 1)[0].glyph, "❤️");
    }

    #[test]
    fn tiers_rank_exact_then_prefix_then_words_then_fuzzy() {
        let table = table();
        let tier = |query: &str, glyph: &str| {
            let words: Vec<&str> = query.split_whitespace().collect();
            let text = words.join(" ");
            let query = Query::new(&text, words);
            let index = all().iter().position(|e| e.glyph == glyph).unwrap();
            MATCHER
                .with_borrow_mut(|matcher| {
                    classify(&query, &table.searchable[index], matcher, &mut Vec::new())
                })
                .map(|(tier, _)| tier)
        };
        assert_eq!(tier("thumbs up", "👍"), Some(Tier::ExactName));
        assert_eq!(tier("lol", "😂"), Some(Tier::ExactKeyword));
        assert_eq!(tier("grinning", "😀"), Some(Tier::NameWord));
        assert_eq!(tier("heart", "❤️"), Some(Tier::NameWord));
        assert_eq!(tier("grin", "😁"), Some(Tier::ExactKeyword));
        assert_eq!(tier("grinn", "😀"), Some(Tier::NamePrefix));
        assert_eq!(tier("face with tears", "😂"), Some(Tier::NameWord));
        assert_eq!(tier("face with te", "😂"), Some(Tier::NamePrefix));
        assert_eq!(tier("hilar", "😂"), Some(Tier::KeywordPrefix));
        assert_eq!(tier("tears joy", "😂"), Some(Tier::NameWords));
        assert_eq!(tier("hilarious face", "😂"), Some(Tier::AnyWords));
        assert_eq!(tier("thmbs up", "👍"), Some(Tier::FuzzyName));
        assert_eq!(tier("hilaros", "😂"), Some(Tier::FuzzyKeyword));
        assert_eq!(tier("qqq", "😂"), None);
    }

    #[test]
    fn a_name_match_outranks_a_keyword_only_match() {
        // "crying face" is a name; 😂 only reaches it through the keyword "crying".
        let results = search("crying", 40);
        let face = position(&results, "😢").unwrap();
        let joy = position(&results, "😂").unwrap();
        assert!(face < joy, "{:?}", names("crying", 40));
        // A name prefix beats a keyword-only prefix ("flag" names versus keyword "flag").
        let flag = search("flag:", 3);
        assert!(flag.iter().all(|e| e.name.starts_with("flag:")));
    }

    #[test]
    fn fuzzy_matching_forgives_typos_and_dropped_letters() {
        assert_eq!(glyphs("thums up", 1), ["👍"], "{:?}", names("thums up", 3));
        assert_eq!(glyphs("red hart", 1), ["❤️"], "{:?}", names("red hart", 3));
        assert!(search("grnning", 5).iter().any(|e| e.glyph == "😀"));
    }

    #[test]
    fn multi_word_queries_need_every_word() {
        let results = search("face with tears", 5);
        assert_eq!(results[0].glyph, "😂", "{:?}", names("face with tears", 5));
        assert!(results.iter().all(|e| e.name.contains("tears")));
        // Word order and word starts do not matter.
        assert!(search("tears joy", 3).iter().any(|e| e.glyph == "😂"));
        assert!(search("joy tears", 3).iter().any(|e| e.glyph == "😂"));
        // Words may come from the name and the keywords together.
        assert!(search("hilarious face", 5).iter().any(|e| e.glyph == "😂"));
        // Extra whitespace is ignored.
        assert_eq!(
            glyphs("  face   with  tears ", 5),
            glyphs("face with tears", 5)
        );
        assert!(search("face with zebra", 5).is_empty());
    }

    #[test]
    fn flags_are_found_by_keyword_and_by_country() {
        let flags = search("flag", 40);
        assert_eq!(flags.len(), 40);
        assert!(
            flags
                .iter()
                .all(|e| e.name.contains("flag") || e.keywords.iter().any(|k| k == "flag"))
        );
        assert!(flags.iter().any(|e| e.group == Group::Flags));
        assert_eq!(
            glyphs("flag united states", 1),
            ["🇺🇸"],
            "{:?}",
            names("flag united states", 3)
        );
        assert_eq!(glyphs("flag: japan", 1), ["🇯🇵"]);
    }

    #[test]
    fn case_is_ignored_and_results_are_deterministic() {
        for query in ["Thumbs Up", "THUMBS UP", "tHuMbS uP"] {
            assert_eq!(glyphs(query, 10), glyphs("thumbs up", 10), "{query}");
        }
        assert_eq!(glyphs("heart", 25), glyphs("heart", 25));
        // Whole-word name matches come first and the plainer name wins a tie.
        let results = search("heart", 3);
        assert_eq!(results[0].glyph, "❤️");
        assert!(results.iter().all(|e| e.name.contains("heart")));
    }

    #[test]
    fn nonsense_matches_nothing() {
        assert!(search("zzzzzzzz", 10).is_empty());
        assert!(search("qxqxqx", 10).is_empty());
    }

    #[test]
    fn odd_input_does_not_panic() {
        for query in [
            "^",
            "$",
            "!",
            "'",
            "\\",
            "\u{0}",
            "é",
            "😀",
            "a\u{200d}b",
            "🇺🇸",
        ] {
            let _ = search(query, 5);
        }
        assert_eq!(search("😀", 3).len(), search("😀", 3).len());
    }

    #[test]
    fn tone_variants_list_five_in_order() {
        let thumbs = find("👍").unwrap();
        assert!(thumbs.has_tones());
        let variants: Vec<_> = thumbs.tone_variants().collect();
        assert_eq!(
            variants,
            [
                (Tone::Light, "👍🏻"),
                (Tone::MediumLight, "👍🏼"),
                (Tone::Medium, "👍🏽"),
                (Tone::MediumDark, "👍🏾"),
                (Tone::Dark, "👍🏿"),
            ]
        );
        assert_eq!(thumbs.with_tone(Tone::Medium), "👍🏽");
        assert_eq!(thumbs.id(), "👍");
        // Without tones the plain glyph is returned and the list is empty.
        let face = find("😀").unwrap();
        assert!(!face.has_tones());
        assert_eq!(face.tone_variants().count(), 0);
        assert_eq!(face.with_tone(Tone::Dark), "😀");
        assert_eq!(Tone::Light.label(), "light skin tone");
    }

    #[test]
    fn find_maps_variants_back_to_the_base_emoji() {
        assert_eq!(find("👍🏿").unwrap().glyph, "👍");
        // The weight lifter's base carries a variation selector its variants do not.
        let lifter = find("🏋️").unwrap();
        assert_eq!(lifter.name, "person lifting weights");
        assert_eq!(find("🏋🏽").unwrap().glyph, lifter.glyph);
        assert_eq!(find("🇺🇸").unwrap().name, "flag: United States");
        assert!(find("not an emoji").is_none());
    }

    #[test]
    fn tone_variants_keep_their_shape_and_mixed_tones_are_absent() {
        for emoji in all().iter().filter(|e| e.has_tones()) {
            for (tone, glyph) in emoji.tone_variants() {
                let modifier = char::from_u32(0x1F3FB + tone as u32).unwrap();
                assert!(glyph.contains(modifier), "{} {tone:?}", emoji.name);
                let others = ('\u{1F3FB}'..='\u{1F3FF}').filter(|&c| c != modifier);
                for other in others {
                    assert!(!glyph.contains(other), "{} mixes tones", emoji.name);
                }
            }
        }
        // Tone modifiers only ever appear in the variants, never in a base glyph.
        assert!(all().iter().all(|e| !e.glyph.contains('\u{1F3FB}')));
    }

    #[test]
    fn search_never_returns_tone_variants_as_results() {
        for emoji in search("thumbs", 50) {
            assert!(!emoji.glyph.contains('\u{1F3FB}'));
        }
    }

    #[test]
    fn groups_round_trip_through_their_labels() {
        for group in Group::ALL {
            assert_eq!(Group::from_label(group.label()), Some(group));
        }
        assert_eq!(Group::from_label("Component"), None);
    }

    #[test]
    fn parsing_rejects_malformed_records() {
        let ok = "@group Flags\n@subgroup flag\n🏁\tchequered flag\t1.0\tflag|race\t\n";
        let parsed = parse(ok).unwrap();
        assert_eq!(parsed[0].keywords, ["flag", "race"]);
        assert_eq!(parsed[0].group, Group::Flags);
        // A record without the trailing tone field still parses.
        assert!(parse("@group Flags\n@subgroup flag\n🏁\tchequered flag\t1.0\tflag").is_ok());
        for bad in [
            "@group Nope\n",
            "🏁\tchequered flag\t1.0\tflag\t\n",
            "@group Flags\n🏁\tchequered flag\t1.0\tflag\t\n",
            "@group Flags\n@subgroup flag\n🏁\tchequered flag\n",
            "@group Flags\n@subgroup flag\n\tname\t1.0\t\t\n",
            "@group Flags\n@subgroup flag\n🏁\tn\t1.0\t\t👍🏻 👍🏼\n",
            "@group Flags\n@subgroup flag\n🏁\tn\t1.0\t\t\textra\n",
        ] {
            assert!(parse(bad).is_err(), "{bad:?}");
        }
    }
}
