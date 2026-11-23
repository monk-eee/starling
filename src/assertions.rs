use std::{borrow::Cow, sync::LazyLock};

use anyhow::{Result, ensure};
use regex::{Regex, RegexBuilder};
use serde::Deserialize;
use serde_json::Value;
use unicode_normalization::UnicodeNormalization;

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MatchAssertion {
    pub label: String,
    path: String,
    pub first: usize,
    all: Vec<FieldRule>,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct FieldRule {
    path: String,
    #[serde(default)]
    markdown: bool,
    matches: Vec<Pattern>,
}

#[derive(Clone, Deserialize)]
#[serde(try_from = "String")]
struct Pattern(Regex);

impl TryFrom<String> for Pattern {
    type Error = &'static str;

    fn try_from(pattern: String) -> Result<Self, Self::Error> {
        if pattern.len() > 512 {
            return Err("assertion pattern exceeds 512 bytes");
        }
        let re = RegexBuilder::new(&pattern)
            .case_insensitive(true)
            .size_limit(512 * 1024)
            .build()
            .map_err(|_| "invalid or oversized assertion pattern")?;
        if re.is_match("") {
            return Err("assertion patterns must not match empty text");
        }
        Ok(Self(re))
    }
}

pub fn valid_pointer(s: &str) -> bool {
    s.starts_with('/') && s.len() <= 256 && s.split('~').skip(1).all(|p| p.starts_with(['0', '1']))
}

pub fn public_text(s: &str) -> bool {
    !s.trim().is_empty() && s.len() <= 200 && !s.chars().any(char::is_control)
}

fn passage(input: &str) -> String {
    static LINK: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\[([^\]\n]+)\]\([^\s)]+\)").unwrap());
    static URL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)https?://[^\s)`]+").unwrap());
    let normalized: String = input.nfkc().collect();
    let labels = LINK.replace_all(&normalized, "$1");
    let without_urls = URL.replace_all(&labels, "");
    without_urls
        .replace('`', "")
        .replace("&nbsp;", " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

impl MatchAssertion {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            public_text(&self.label),
            "assert_match.label must be 1-200 printable bytes"
        );
        ensure!(
            valid_pointer(&self.path),
            "assert_match.path must be a JSON Pointer"
        );
        ensure!(
            (1..=32).contains(&self.first),
            "assert_match.first must be 1-32"
        );
        ensure!(
            (1..=8).contains(&self.all.len()),
            "assert_match.all needs 1-8 field rules"
        );
        for rule in &self.all {
            ensure!(
                valid_pointer(&rule.path),
                "assert_match field path must be a JSON Pointer"
            );
            ensure!(
                (1..=8).contains(&rule.matches.len()),
                "each assertion field needs 1-8 patterns"
            );
        }
        Ok(())
    }

    pub fn rank(&self, result: &Value) -> Option<usize> {
        result
            .pointer(&self.path)?
            .as_array()?
            .iter()
            .take(self.first)
            .position(|row| {
                self.all.iter().all(|rule| {
                    let Some(value) = row.pointer(&rule.path).and_then(Value::as_str) else {
                        return false;
                    };
                    let text = if rule.markdown {
                        Cow::Owned(passage(value))
                    } else {
                        Cow::Borrowed(value)
                    };
                    !text.trim().is_empty() && rule.matches.iter().all(|p| p.0.is_match(&text))
                })
            })
            .map(|i| i + 1)
    }
}

#[cfg(test)]
mod tests;
