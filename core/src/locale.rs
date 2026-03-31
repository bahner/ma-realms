use fluent_bundle::{FluentBundle, FluentResource};
use std::collections::HashMap;
use unic_langid::LanguageIdentifier;

pub fn canonical_locale(input: &str) -> String {
    let normalized = input.trim().replace('_', "-").to_ascii_lowercase();
    match normalized.as_str() {
        "nb" | "nb-no" | "nb-no.utf8" | "nb-no.utf-8" => "nb-NO".to_string(),
        _ => "en".to_string(),
    }
}

const CORE_FTL_EN: &str = include_str!("../locales/en/core.ftl");
const CORE_FTL_NB: &str = include_str!("../locales/nb-NO/core.ftl");

fn split_csv(input: &str) -> Vec<String> {
    input
        .split(',')
        .map(|item| item.trim())
        .filter(|item| !item.is_empty())
        .map(|item| item.to_ascii_lowercase())
        .collect()
}

fn build_bundle(locale: &str) -> Option<FluentBundle<FluentResource>> {
    let canonical = canonical_locale(locale);
    let lang_id: LanguageIdentifier = canonical.parse().ok()?;

    let mut bundle = FluentBundle::new(vec![lang_id]);
    let ftl = if canonical == "nb-NO" {
        CORE_FTL_NB
    } else {
        CORE_FTL_EN
    };
    let resource = FluentResource::try_new(ftl.to_string()).ok()?;
    bundle.add_resource(resource).ok()?;
    Some(bundle)
}

fn fluent_text(locale: &str, key: &str) -> Option<String> {
    let bundle = build_bundle(locale)?;
    let message = bundle.get_message(key)?;
    let pattern = message.value()?;
    let mut errors = Vec::new();
    let value = bundle.format_pattern(pattern, None, &mut errors).to_string();
    if value.trim().is_empty() {
        None
    } else {
        Some(value)
    }
}

#[derive(Debug, Clone)]
pub struct LocaleLexicon {
    pub canonical_locale: String,
    pub here_aliases: Vec<String>,
    pub avatar_aliases: Vec<String>,
    pub say_verbs: Vec<String>,
    pub room_command_aliases: HashMap<String, String>,
}

impl LocaleLexicon {
    pub fn for_locale(locale: &str) -> Self {
        let canonical = canonical_locale(locale);

        let here_aliases = fluent_text(&canonical, "here-aliases")
            .map(|v| split_csv(&v))
            .unwrap_or_else(|| vec!["here".to_string(), "room".to_string(), "world".to_string()]);
        let avatar_aliases = fluent_text(&canonical, "avatar-aliases")
            .map(|v| split_csv(&v))
            .unwrap_or_else(|| vec!["avatar".to_string()]);
        let say_verbs = fluent_text(&canonical, "say-verbs")
            .map(|v| split_csv(&v))
            .unwrap_or_else(|| vec!["say".to_string()]);

        let mut room_command_aliases = HashMap::new();
        let help_aliases = fluent_text(&canonical, "room-command-help-aliases")
            .map(|v| split_csv(&v))
            .unwrap_or_else(|| vec!["help".to_string()]);
        for alias in help_aliases {
            room_command_aliases.insert(alias, "help".to_string());
        }
        let who_aliases = fluent_text(&canonical, "room-command-who-aliases")
            .map(|v| split_csv(&v))
            .unwrap_or_else(|| vec!["who".to_string(), "actors".to_string()]);
        for alias in who_aliases {
            room_command_aliases.insert(alias, "who".to_string());
        }

        Self {
            canonical_locale: canonical,
            here_aliases,
            avatar_aliases,
            say_verbs,
            room_command_aliases,
        }
    }

    pub fn localized_here_alias(&self) -> &str {
        self.here_aliases
            .first()
            .map(|s| s.as_str())
            .unwrap_or("here")
    }

    pub fn localized_say_verb(&self) -> &str {
        self.say_verbs.first().map(|s| s.as_str()).unwrap_or("say")
    }
}

pub fn localized_here_alias(locale: &str) -> String {
    LocaleLexicon::for_locale(locale).localized_here_alias().to_string()
}

pub fn localized_say_verb(locale: &str) -> String {
    LocaleLexicon::for_locale(locale).localized_say_verb().to_string()
}
