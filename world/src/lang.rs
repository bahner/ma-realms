use std::collections::HashMap;
use std::sync::OnceLock;

fn parse_ftl_messages(raw: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = trimmed.split_once('=') {
            out.insert(key.trim().to_string(), value.trim().to_string());
        }
    }
    out
}

fn world_lang_catalogs() -> &'static HashMap<String, HashMap<String, String>> {
    static CATALOGS: OnceLock<HashMap<String, HashMap<String, String>>> = OnceLock::new();
    CATALOGS.get_or_init(|| {
        let mut map = HashMap::new();
        map.insert(
            "en_UK".to_string(),
            parse_ftl_messages(include_str!("../lang/en_UK.ftl")),
        );
        map.insert(
            "nb_NO".to_string(),
            parse_ftl_messages(include_str!("../lang/nb_NO.ftl")),
        );
        map
    })
}

fn canonical_tag(raw: &str) -> String {
    let base = raw
        .split('@')
        .next()
        .unwrap_or_default()
        .split('.')
        .next()
        .unwrap_or_default()
        .trim();

    if base.is_empty() {
        return String::new();
    }

    let normalized = base.replace('-', "_");
    let mut parts = normalized.splitn(2, '_');
    let lang = parts.next().unwrap_or_default().to_ascii_lowercase();
    let region = parts.next().unwrap_or_default().to_ascii_uppercase();
    if lang.is_empty() {
        return String::new();
    }
    if region.is_empty() {
        lang
    } else {
        format!("{}_{}", lang, region)
    }
}

fn supported_world_languages() -> &'static Vec<String> {
    static SUPPORTED: OnceLock<Vec<String>> = OnceLock::new();
    SUPPORTED.get_or_init(|| {
        let mut values = world_lang_catalogs().keys().cloned().collect::<Vec<_>>();
        values.sort_by(|left, right| left.cmp(right));
        if let Some(pos) = values.iter().position(|v| v == "en_UK") {
            let preferred = values.remove(pos);
            values.insert(0, preferred);
        }
        values
    })
}

pub(crate) fn supported_world_languages_text() -> String {
    supported_world_languages().join(", ")
}

pub(crate) fn collapse_world_language_order_strict(profile: &str) -> Option<String> {
    let supported = supported_world_languages();
    let mut out: Vec<String> = Vec::new();

    for token in profile.split(|ch| ch == ';' || ch == ',' || ch == ':') {
        let canonical = canonical_tag(token);
        if canonical.is_empty() {
            continue;
        }

        if canonical.contains('_') {
            if let Some(exact) = supported
                .iter()
                .find(|entry| entry.eq_ignore_ascii_case(canonical.as_str()))
            {
                if !out.iter().any(|seen| seen.eq_ignore_ascii_case(exact)) {
                    out.push(exact.clone());
                }
                continue;
            }
        }

        let mut parts = canonical.splitn(2, '_');
        let lang = parts.next().unwrap_or_default();
        if lang.is_empty() {
            continue;
        }

        for candidate in supported.iter().filter(|entry| {
            entry
                .split('_')
                .next()
                .map(|prefix| prefix.eq_ignore_ascii_case(lang))
                .unwrap_or(false)
        }) {
            if !out.iter().any(|seen| seen.eq_ignore_ascii_case(candidate)) {
                out.push(candidate.clone());
            }
        }
    }

    if out.is_empty() {
        None
    } else {
        Some(out.join(";"))
    }
}

pub(crate) fn collapse_world_language_order(profile: &str) -> String {
    collapse_world_language_order_strict(profile).unwrap_or_else(|| "en_UK".to_string())
}

pub(crate) fn world_lang_from_profile(profile: &str) -> &'static str {
    let collapsed = collapse_world_language_order(profile);
    collapsed
        .split(';')
        .map(str::trim)
        .find_map(|candidate| {
            if candidate.eq_ignore_ascii_case("nb_NO") {
                Some("nb_NO")
            } else if candidate.eq_ignore_ascii_case("en_UK") {
                Some("en_UK")
            } else {
                None
            }
        })
        .unwrap_or("en_UK")
}

pub(crate) fn tr_world(lang: &str, key: &str, fallback: &str) -> String {
    let catalogs = world_lang_catalogs();
    catalogs
        .get(lang)
        .and_then(|entry| entry.get(key))
        .or_else(|| catalogs.get("en_UK").and_then(|entry| entry.get(key)))
        .cloned()
        .unwrap_or_else(|| fallback.to_string())
}
