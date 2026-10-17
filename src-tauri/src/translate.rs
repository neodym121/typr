//! Translation of the dictation (Post-processing → Translation): the names
//! of the languages offered and whether a text is in one of them already.
//! A dictation spoken in the language it would be translated into isn't
//! sent to the model to translate; with post-processing on it is still
//! edited, without the translation.
//!
//! The check answers one question, "is this text surely in that language",
//! and says yes only when it is sure: a text in doubt (short, mixed, a close
//! language) goes to the model, which is told to keep text already in the
//! language as it is. Mistaking a text for the target language would paste
//! it untranslated; the other mistake costs one request. Libraries that
//! detect any language guess poorly on a dictation's few words (whatlang
//! reads plain Russian as Ukrainian), so the check is built for these
//! languages:
//! - Chinese, Japanese, Hindi, Hebrew and Persian by their writing system
//!   (Persian by the letters Arabic and Urdu write differently);
//! - Russian and Ukrainian by the letters only one of them has, then by
//!   common words found in only one of them;
//! - English, Spanish, French and German by letters outside the language's
//!   alphabet (they rule it out), then by common words found in only one of
//!   them or their neighbours (Italian, Portuguese, Dutch, Polish).

use crate::settings::TranslationLanguage;

impl TranslationLanguage {
    /// English name, for the prompt and the log
    pub fn name(self) -> &'static str {
        match self {
            TranslationLanguage::English => "English",
            TranslationLanguage::Russian => "Russian",
            TranslationLanguage::Ukrainian => "Ukrainian",
            TranslationLanguage::Spanish => "Spanish",
            TranslationLanguage::French => "French",
            TranslationLanguage::German => "German",
            TranslationLanguage::Chinese => "Chinese (Simplified, Mandarin)",
            TranslationLanguage::Japanese => "Japanese",
            TranslationLanguage::Hindi => "Hindi",
            TranslationLanguage::Hebrew => "Hebrew",
            TranslationLanguage::Persian => "Persian (Farsi)",
        }
    }
}

/// Whether `text` is surely written in `language` already.
pub fn is_written_in(text: &str, language: TranslationLanguage) -> bool {
    let letters = Letters::count(text);
    if letters.total == 0 {
        return false;
    }
    match language {
        TranslationLanguage::Chinese => letters.kana == 0 && letters.share(letters.han) >= MAIN_SCRIPT,
        TranslationLanguage::Japanese => {
            letters.kana > 0 && letters.share(letters.kana + letters.han) >= MAIN_SCRIPT
        }
        TranslationLanguage::Hindi => letters.share(letters.devanagari) >= MAIN_SCRIPT,
        TranslationLanguage::Hebrew => letters.share(letters.hebrew) >= MAIN_SCRIPT,
        TranslationLanguage::Persian => {
            letters.share(letters.arabic) >= MAIN_SCRIPT
                && text.chars().any(|c| PERSIAN_LETTERS.contains(c))
                && !text.chars().any(|c| NOT_PERSIAN_LETTERS.contains(c))
        }
        TranslationLanguage::Russian | TranslationLanguage::Ukrainian => {
            letters.share(letters.cyrillic) >= MAIN_SCRIPT && words_say(text, language, CYRILLIC)
        }
        TranslationLanguage::English
        | TranslationLanguage::Spanish
        | TranslationLanguage::French
        | TranslationLanguage::German => {
            letters.share(letters.latin) >= MAIN_SCRIPT && words_say(text, language, LATIN)
        }
    }
}

/// The target language's script must make up this share of the letters
const MAIN_SCRIPT: f32 = 0.8;

/// Letters of a text by writing system.
#[derive(Default)]
struct Letters {
    total: usize,
    latin: usize,
    cyrillic: usize,
    han: usize,
    kana: usize,
    devanagari: usize,
    hebrew: usize,
    arabic: usize,
}

impl Letters {
    fn count(text: &str) -> Self {
        let mut letters = Letters::default();
        for c in text.chars().filter(|c| c.is_alphabetic()) {
            letters.total += 1;
            let script = match c as u32 {
                0x41..=0x5A | 0x61..=0x7A | 0xC0..=0x24F | 0x1E00..=0x1EFF => &mut letters.latin,
                0x400..=0x52F => &mut letters.cyrillic,
                0x3040..=0x30FF | 0x31F0..=0x31FF | 0xFF66..=0xFF9F => &mut letters.kana,
                0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0x20000..=0x2FFFF => &mut letters.han,
                0x900..=0x97F => &mut letters.devanagari,
                0x590..=0x5FF | 0xFB1D..=0xFB4F => &mut letters.hebrew,
                0x600..=0x6FF | 0x750..=0x77F | 0xFB50..=0xFDFF | 0xFE70..=0xFEFF => &mut letters.arabic,
                _ => continue,
            };
            *script += 1;
        }
        letters
    }

    fn share(&self, count: usize) -> f32 {
        count as f32 / self.total as f32
    }
}

/// Persian writes these (peh, tcheh, jeh, gaf, Farsi yeh, keheh)...
const PERSIAN_LETTERS: &str = "پچژگیک";
/// ...and never these: Arabic's yeh, kaf, teh marbuta and alef maksura,
/// Urdu's tteh, ddal, rreh, noon ghunna, yeh barree and do-chashmee heh
const NOT_PERSIAN_LETTERS: &str = "يكةىٹڈڑںےھ";

/// How a language of a group shows itself in a text.
struct Profile {
    /// None for a neighbour, which only counts against the others
    language: Option<TranslationLanguage>,
    /// The lowercase letters beyond a–z this language writes; any other
    /// letter rules it out. None: `never` lists the letters that do
    alphabet: Option<&'static str>,
    never: &'static str,
    /// A word with one of these letters counts for the language
    own_letters: &'static str,
    /// A word ending so counts for the language
    own_ending: &'static str,
    /// Common words found in no other language of the group
    words: &'static [&'static str],
}

impl Profile {
    fn rules_out(&self, letter: char) -> bool {
        match self.alphabet {
            Some(alphabet) => !letter.is_ascii() && !alphabet.contains(letter),
            None => self.never.contains(letter),
        }
    }

    /// How many of `words` point at this language
    fn score(&self, words: &[&str]) -> usize {
        words
            .iter()
            .filter(|word| {
                self.words.contains(word)
                    || word.chars().any(|c| self.own_letters.contains(c))
                    || (!self.own_ending.is_empty() && word.ends_with(self.own_ending))
            })
            .count()
    }
}

/// Whether the letters and words of `text` surely point at `language`
/// among the languages of `group`.
fn words_say(text: &str, language: TranslationLanguage, group: &[Profile]) -> bool {
    let Some(target) = group.iter().find(|p| p.language == Some(language)) else {
        return false;
    };
    let lower = text.to_lowercase();
    if lower.chars().filter(|c| c.is_alphabetic()).any(|c| target.rules_out(c)) {
        return false;
    }
    let words: Vec<&str> = lower.split(|c: char| !c.is_alphabetic()).filter(|w| !w.is_empty()).collect();
    let score = target.score(&words);
    let rival = group
        .iter()
        .filter(|p| p.language != Some(language))
        .map(|p| p.score(&words))
        .max()
        .unwrap_or(0);
    // Well ahead of every other language, and enough of the text says so
    score >= 1 && score >= 3 * rival && score * 6 >= words.len()
}

const CYRILLIC: &[Profile] = &[
    Profile {
        language: Some(TranslationLanguage::Russian),
        alphabet: None,
        never: "іїєґўђјљњћџѓќѕ",
        own_letters: "ыэё",
        own_ending: "шь",
        words: &[
            "что", "чтобы", "это", "этот", "как", "когда", "где", "почему", "зачем", "нет", "да", "он", "она",
            "оно", "они", "его", "ее", "меня", "тебя", "себя", "сегодня", "вчера", "привет", "спасибо",
            "пожалуйста", "хорошо", "ладно", "только", "еще", "можно", "нужно", "надо", "будет", "будут",
            "сейчас", "тоже", "если", "или", "очень", "и", "с", "со", "мне", "был", "была", "было", "есть",
            "который", "которая", "кстати", "сделать", "извини", "пока", "сколько", "потом", "тогда", "здесь",
            "какой", "какая", "много", "ничего",
        ],
    },
    Profile {
        language: Some(TranslationLanguage::Ukrainian),
        alphabet: None,
        never: "ыэёъўђјљњћџѓќѕ",
        own_letters: "іїєґ",
        own_ending: "",
        words: &[
            "що", "щоб", "це", "цей", "ця", "як", "коли", "де", "чому", "вона", "воно", "вони", "його", "мене",
            "ласка", "добре", "гаразд", "лише", "вже", "ще", "можна", "треба", "потрібно", "буде", "будуть",
            "зараз", "теж", "також", "якщо", "або", "дуже", "та", "й", "з", "із", "зі", "був", "була", "були",
            "було", "який", "яка", "яке", "зробити", "вибач", "поки", "багато", "чи",
        ],
    },
    // A neighbour: Bulgarian words count against Russian
    Profile {
        language: None,
        alphabet: None,
        never: "",
        own_letters: "",
        own_ending: "",
        words: &["това", "какво", "защо", "със", "благодаря", "здравей", "са", "съм", "сме"],
    },
];

const LATIN: &[Profile] = &[
    Profile {
        language: Some(TranslationLanguage::English),
        alphabet: Some(""),
        never: "",
        own_letters: "",
        own_ending: "",
        words: &[
            "the", "and", "is", "are", "were", "you", "your", "have", "has", "had", "with", "this", "that",
            "what", "would", "should", "could", "which", "there", "please", "from", "for", "of", "it", "my",
            "we", "they", "be", "been", "will", "not", "can", "just", "about", "how", "why", "when", "where",
            "who", "to", "at", "by", "if", "or", "but", "she", "him", "her", "our", "their", "them", "does",
            "did", "going", "want", "know", "think", "thanks", "thank", "hello", "yes", "get", "got", "need",
            "let", "all", "some", "any", "here", "now", "today", "tomorrow", "send",
        ],
    },
    Profile {
        language: Some(TranslationLanguage::Spanish),
        alphabet: Some("áéíóúüñ"),
        never: "",
        own_letters: "ñ",
        own_ending: "",
        words: &[
            "el", "los", "las", "una", "unos", "unas", "del", "y", "pero", "más", "muy", "ya", "eso", "esto",
            "hay", "estoy", "soy", "qué", "cómo", "cuándo", "dónde", "también", "hola", "gracias", "bueno",
            "vale", "hoy", "mañana", "yo", "él", "ella", "nosotros", "usted", "ustedes", "sí", "pues", "ahora",
            "puedo", "puedes", "quiero", "tengo", "tienes", "voy", "vamos", "algo", "nada", "todo", "sin",
            "cuando", "donde", "así", "aquí", "mí", "mucho", "hacer", "están",
        ],
    },
    Profile {
        language: Some(TranslationLanguage::French),
        alphabet: Some("àâæçéèêëîïôœùûüÿ"),
        never: "",
        own_letters: "œç",
        own_ending: "",
        words: &[
            "le", "les", "une", "des", "du", "et", "est", "sont", "je", "elle", "nous", "vous", "ils", "elles",
            "ce", "cette", "ces", "qui", "quoi", "ne", "pas", "pour", "avec", "dans", "sur", "mais", "très",
            "aussi", "oui", "merci", "bonjour", "salut", "au", "aux", "mon", "tes", "sa", "ses", "leur", "ça",
            "être", "avoir", "fait", "faire", "aujourd", "hui", "demain", "ai", "suis", "peux", "veux", "vais",
            "chez", "comme", "alors", "donc", "rien", "tout", "toujours", "encore", "déjà", "ici", "moi", "toi",
        ],
    },
    Profile {
        language: Some(TranslationLanguage::German),
        alphabet: Some("äöüß"),
        never: "",
        own_letters: "äöüß",
        own_ending: "",
        words: &[
            "der", "die", "das", "den", "dem", "ein", "eine", "einen", "einem", "einer", "und", "oder", "aber",
            "ist", "sind", "war", "bin", "bist", "hat", "habe", "haben", "ich", "du", "er", "sie", "wir", "ihr",
            "mich", "mir", "dich", "dir", "uns", "nicht", "kein", "keine", "mit", "von", "zu", "zum", "zur",
            "auf", "bei", "nach", "aus", "wie", "wer", "wo", "wann", "warum", "auch", "noch", "schon", "nur",
            "sehr", "ja", "nein", "bitte", "danke", "heute", "morgen", "hallo", "kannst", "kann", "wird",
            "werden", "dass", "wenn", "im", "um", "ob", "mal", "gut", "jetzt", "hier", "doch", "komme",
        ],
    },
    // Neighbours: Italian, Portuguese, Dutch and Polish words count against
    // the four above
    Profile {
        language: None,
        alphabet: None,
        never: "",
        own_letters: "ìò",
        own_ending: "",
        words: &[
            "gli", "di", "che", "è", "e", "per", "sono", "ho", "hai", "ciao", "grazie", "questo", "questa",
            "perché", "anche", "molto", "io", "lei", "noi", "voi", "loro", "ci", "della", "delle", "dei", "nel",
            "nella", "alla", "sei", "siamo", "fare", "oggi", "domani", "bene", "allora", "più", "già",
            "quindi", "senza",
        ],
    },
    Profile {
        language: None,
        alphabet: None,
        never: "",
        own_letters: "ãõ",
        own_ending: "",
        words: &[
            "uma", "dos", "das", "é", "e", "não", "em", "na", "com", "você", "vocês", "ele", "ela", "nós",
            "mas", "muito", "também", "obrigado", "obrigada", "olá", "isso", "isto", "eu", "estou", "sou",
            "são", "tem", "tenho", "vou", "hoje", "amanhã", "aqui", "então", "ainda", "já", "sim", "bom",
            "pra", "meu", "minha", "seu", "sua",
        ],
    },
    Profile {
        language: None,
        alphabet: None,
        never: "",
        own_letters: "",
        own_ending: "",
        words: &[
            "het", "een", "van", "ik", "niet", "dat", "op", "voor", "zijn", "maar", "ook", "wat", "dit", "hij",
            "zij", "wij", "jij", "heb", "hebt", "heeft", "naar", "bij", "nog", "wel", "geen", "goed", "vandaag",
            "thuis", "werk",
        ],
    },
    Profile {
        language: None,
        alphabet: None,
        never: "",
        own_letters: "ąćęłńśźż",
        own_ending: "",
        words: &[
            "w", "z", "nie", "się", "że", "jest", "co", "jak", "ale", "tak", "po", "mnie", "ty", "ona", "czy",
            "już", "dla", "od", "jestem", "będzie", "może", "tylko", "bardzo", "dzień", "dobry", "dziękuję",
            "cześć", "proszę", "teraz", "tutaj", "więc", "który", "która", "które",
        ],
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use TranslationLanguage::*;

    const RUSSIAN: &str = "Привет! Я сегодня задержусь на работе, так что ужинайте без меня.";
    const UKRAINIAN: &str = "Привіт! Я сьогодні затримаюся на роботі, тож вечеряйте без мене.";
    const ENGLISH: &str = "Hi! I'm going to be late at work today, so have dinner without me.";
    const SPANISH: &str = "¡Hola! Hoy voy a llegar tarde del trabajo, así que cenad sin mí.";
    const GERMAN: &str = "Hallo! Ich komme heute spät von der Arbeit, also esst ohne mich zu Abend.";
    const ITALIAN: &str = "Ciao! Oggi farò tardi al lavoro, quindi cenate senza di me.";
    const JAPANESE: &str = "こんにちは！今日は仕事で遅くなるので、先に夕食を食べてください。";

    #[test]
    fn test_a_text_in_the_target_language_is_recognized() {
        let cases = [
            (RUSSIAN, Russian),
            ("Можешь скинуть файл?", Russian),
            ("Отправь мне, пожалуйста, отчёт до пятницы.", Russian),
            ("Олег, привет, как дела?", Russian),
            (UKRAINIAN, Ukrainian),
            ("Що ти робиш?", Ukrainian),
            (ENGLISH, English),
            ("Send me the report by Friday, please.", English),
            ("Thanks.", English),
            (SPANISH, Spanish),
            ("Gracias.", Spanish),
            ("Salut ! Je vais rentrer tard du travail aujourd'hui, alors dînez sans moi.", French),
            (GERMAN, German),
            ("你好！我今天下班会晚一点，所以你们先吃晚饭吧。", Chinese),
            (JAPANESE, Japanese),
            ("नमस्ते! आज मुझे काम से आने में देर होगी, इसलिए आप लोग मेरे बिना खाना खा लेना।", Hindi),
            ("שלום! היום אני אאחר בעבודה, אז תאכלו ארוחת ערב בלעדיי.", Hebrew),
            ("سلام! امروز دیر از سر کار برمی‌گردم، پس شام را بدون من بخورید.", Persian),
        ];
        for (text, language) in cases {
            assert!(is_written_in(text, language), "{:?} not recognized: {}", language, text);
        }
    }

    #[test]
    fn test_another_language_or_a_doubt_goes_to_the_model() {
        let cases = [
            (RUSSIAN, English),
            (RUSSIAN, Ukrainian),
            (UKRAINIAN, Russian),
            ("Що ти робиш?", Russian),
            (ENGLISH, Spanish),
            (ENGLISH, German),
            (ITALIAN, Spanish),
            (ITALIAN, French),
            ("Eu vou chegar tarde do trabalho hoje, então jantem sem mim.", Spanish),
            ("Ik kom vandaag laat thuis van het werk.", German),
            (SPANISH, English),
            (GERMAN, English),
            (JAPANESE, Chinese),
            ("مرحبا! سأتأخر في العمل اليوم، لذلك تناولوا العشاء بدوني.", Persian),
            ("Здравей! Днес ще закъснея на работа, така че вечеряйте без мен.", Russian),
            // Mostly another script
            ("Открой Google Docs и скинь ссылку", English),
            // Nothing to tell by
            ("Да.", Ukrainian),
            ("Окей", Russian),
            ("", English),
            ("12345", English),
        ];
        for (text, language) in cases {
            assert!(!is_written_in(text, language), "taken for {:?}: {}", language, text);
        }
    }
}
