//! Whole-utterance voice choices. The core applies policy, ownership and freshness before calling this matcher.
use crate::Choice;

/// Only a complete, short answer can select a permission. English number words are accepted.
#[must_use]
pub fn answer(text: &str, number: u8, allow: bool) -> Option<Choice> {
    let text = phrase(text)?;
    if text == "no" {
        return Some(Choice::No);
    }
    if !allow || !(10..=99).contains(&number) {
        return None;
    }
    (text == format!("allow {number}") || text == format!("allow {}", number_words(number)))
        .then_some(Choice::Once)
}

/// A deliberate request to hear the same challenge again; it never answers a card.
#[must_use]
pub fn repeat(text: &str) -> bool {
    phrase(text).is_some_and(|t| matches!(t.as_str(), "repeat" | "say it again"))
}

fn phrase(text: &str) -> Option<String> {
    let text = text.trim();
    if text.len() > 64 || !text.is_ascii() || text.chars().any(char::is_control) {
        return None;
    }
    let text = text
        .trim_end_matches(['.', '!'])
        .to_ascii_lowercase()
        .replace('-', " ");
    let words: Vec<_> = text.split_whitespace().collect();
    (words.len() <= 4).then(|| words.join(" "))
}

fn number_words(number: u8) -> String {
    const TEENS: [&str; 10] = [
        "ten",
        "eleven",
        "twelve",
        "thirteen",
        "fourteen",
        "fifteen",
        "sixteen",
        "seventeen",
        "eighteen",
        "nineteen",
    ];
    const TENS: [&str; 10] = [
        "", "", "twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety",
    ];
    const ONES: [&str; 10] = [
        "", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine",
    ];
    if number < 20 {
        return TEENS[usize::from(number - 10)].into();
    }
    let tens = TENS[usize::from(number / 10)];
    let ones = ONES[usize::from(number % 10)];
    if ones.is_empty() {
        tens.into()
    } else {
        format!("{tens} {ones}")
    }
}
