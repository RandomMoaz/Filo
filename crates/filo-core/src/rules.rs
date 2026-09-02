use crate::error::Result;
use crate::model::FileEntry;
use chrono::{Datelike, Utc};
use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Clone, Deserialize)]
pub struct RuleSet {
    #[serde(rename = "rule", default)]
    pub rules: Vec<Rule>,
}

impl RuleSet {
    pub fn from_toml_file(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)?;
        let set: RuleSet = toml::from_str(&text)?;
        Ok(set)
    }

    pub fn destination_for(&self, entry: &FileEntry) -> Option<String> {
        for rule in &self.rules {
            if rule.matcher.matches(entry) {
                return Some(expand_placeholders(&rule.into, entry));
            }
        }
        None
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Rule {
    #[serde(rename = "match")]
    pub matcher: Matcher,
    pub into: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Matcher {
    #[serde(default)]
    pub extensions: Vec<String>,
    pub older_than_days: Option<i64>,
    pub larger_than_bytes: Option<u64>,
    pub name_regex: Option<String>,
}

impl Matcher {
    pub fn matches(&self, entry: &FileEntry) -> bool {
        if !self.extensions.is_empty() {
            let ext = entry.extension.clone().unwrap_or_default();
            if !self.extensions.iter().any(|e| e.to_lowercase() == ext) {
                return false;
            }
        }
        if let Some(days) = self.older_than_days {
            let age = Utc::now().signed_duration_since(entry.modified).num_days();
            if age < days {
                return false;
            }
        }
        if let Some(min) = self.larger_than_bytes {
            if entry.size < min {
                return false;
            }
        }
        if let Some(pattern) = &self.name_regex {
            match regex::Regex::new(pattern) {
                Ok(re) if re.is_match(&entry.name) => {}
                _ => return false,
            }
        }
        true
    }
}

fn expand_placeholders(template: &str, entry: &FileEntry) -> String {
    template
        .replace("{year}", &entry.modified.year().to_string())
        .replace("{month}", &format!("{:02}", entry.modified.month()))
        .replace("{ext}", &entry.extension.clone().unwrap_or_else(|| "none".into()))
}
