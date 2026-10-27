use unicode_normalization::{UnicodeNormalization, char::is_combining_mark};
pub fn normalize(text: &str) -> String {
    text.to_lowercase()
        .replace('ё', "е")
        .nfd()
        .filter(|c| !is_combining_mark(*c))
        .nfc()
        .collect()
}
