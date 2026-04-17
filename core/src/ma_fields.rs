use std::collections::BTreeMap;

use did_ma::{Document, Ipld};

/// Get the `ma` map, if present and actually a map.
fn get_ma_map(doc: &Document) -> Option<&BTreeMap<String, Ipld>> {
    match doc.ma.as_ref()? {
        Ipld::Map(m) => Some(m),
        _ => None,
    }
}

/// Get a string from the `ma` map.
fn get_str<'a>(map: &'a BTreeMap<String, Ipld>, key: &str) -> Option<&'a str> {
    match map.get(key)? {
        Ipld::String(s) => Some(s.as_str()),
        _ => None,
    }
}

/// Get a u64 from the `ma` map.
fn get_u64(map: &BTreeMap<String, Ipld>, key: &str) -> Option<u64> {
    match map.get(key)? {
        Ipld::Integer(n) => u64::try_from(*n).ok(),
        _ => None,
    }
}

/// Ensure `document.ma` is an IPLD map, creating it if absent.
fn ensure_ma(doc: &mut Document) -> &mut BTreeMap<String, Ipld> {
    if doc.ma.is_none() {
        doc.set_ma(Ipld::Map(BTreeMap::new()));
    }
    match doc.ma.as_mut().unwrap() {
        Ipld::Map(m) => m,
        _ => unreachable!("set_ma guarantees Map"),
    }
}

/// Remove a key from `document.ma`, clearing `ma` entirely if it becomes empty.
fn remove_field(doc: &mut Document, key: &str) {
    if let Some(Ipld::Map(m)) = doc.ma.as_mut() {
        m.remove(key);
        if m.is_empty() {
            doc.clear_ma();
        }
    }
}

// ── Getters ──

pub fn ma_type(doc: &Document) -> Option<&str> {
    get_str(get_ma_map(doc)?, "type")
}

pub fn ma_language(doc: &Document) -> Option<&str> {
    get_str(get_ma_map(doc)?, "language")
}

pub fn ma_current_inbox(doc: &Document) -> Option<&str> {
    get_str(get_ma_map(doc)?, "currentInbox")
}

pub fn ma_presence_hint(doc: &Document) -> Option<&str> {
    get_str(get_ma_map(doc)?, "presenceHint")
}

pub fn ma_services(doc: &Document) -> Option<&Ipld> {
    get_ma_map(doc)?.get("services")
}

pub fn ma_world(doc: &Document) -> Option<&str> {
    get_str(get_ma_map(doc)?, "world")
}

pub fn ma_version(doc: &Document) -> Option<&str> {
    get_str(get_ma_map(doc)?, "version")
}

pub fn ma_ping_interval_secs(doc: &Document) -> Option<u64> {
    get_u64(get_ma_map(doc)?, "pingIntervalSecs")
}

pub fn ma_requested_ttl(doc: &Document) -> Option<u64> {
    get_u64(get_ma_map(doc)?, "requestedTtl")
}

// ── Setters ──

pub fn set_ma_type(doc: &mut Document, value: &str) {
    ensure_ma(doc).insert("type".into(), Ipld::String(value.into()));
}

pub fn set_ma_language(doc: &mut Document, value: &str) {
    ensure_ma(doc).insert("language".into(), Ipld::String(value.into()));
}

pub fn set_ma_current_inbox(doc: &mut Document, value: &str) {
    ensure_ma(doc).insert("currentInbox".into(), Ipld::String(value.into()));
}

pub fn set_ma_presence_hint(doc: &mut Document, value: &str) {
    ensure_ma(doc).insert("presenceHint".into(), Ipld::String(value.into()));
}

pub fn set_ma_services(doc: &mut Document, value: Ipld) {
    ensure_ma(doc).insert("services".into(), value);
}

pub fn set_ma_world(doc: &mut Document, value: Ipld) {
    ensure_ma(doc).insert("world".into(), value);
}

pub fn set_ma_version(doc: &mut Document, value: &str) {
    ensure_ma(doc).insert("version".into(), Ipld::String(value.into()));
}

pub fn set_ma_ping_interval_secs(doc: &mut Document, secs: u64) {
    ensure_ma(doc).insert("pingIntervalSecs".into(), Ipld::Integer(secs as i128));
}

pub fn set_ma_requested_ttl(doc: &mut Document, secs: u64) {
    ensure_ma(doc).insert("requestedTtl".into(), Ipld::Integer(secs as i128));
}

// ── Clearers ──

pub fn clear_ma_language(doc: &mut Document) {
    remove_field(doc, "language");
}

pub fn clear_ma_lang(doc: &mut Document) {
    remove_field(doc, "lang");
}

pub fn clear_ma_presence_hint(doc: &mut Document) {
    remove_field(doc, "presenceHint");
}

pub fn clear_ma_current_inbox(doc: &mut Document) {
    remove_field(doc, "currentInbox");
}

pub fn clear_ma_world(doc: &mut Document) {
    remove_field(doc, "world");
}

pub fn clear_ma_requested_ttl(doc: &mut Document) {
    remove_field(doc, "requestedTtl");
}
