export function normalizeLanguageOrder(value, fallback = 'nb_NO:en_UK') {
  const normalized = String(value || '')
    // Backward compatibility: migrate old separators to GNU LANGUAGE style.
    .replace(/[;,]+/g, ':')
    .split(/:+/)
    .map((entry) => String(entry || '').trim())
    .filter(Boolean)
    .join(':');
  return normalized || String(fallback || 'nb_NO:en_UK');
}

export function normalizeUiLang(value) {
  const normalized = String(value || '').trim().replace(/_/g, '-').toLowerCase();
  if (['nb', 'nb-no', 'no'].includes(normalized)) {
    return 'nb';
  }
  if (['en', 'en-us', 'en-gb'].includes(normalized)) {
    return 'en';
  }
  if (['se', 'sv', 'sv-se'].includes(normalized)) {
    return 'se';
  }
  if (['da', 'da-dk'].includes(normalized)) {
    return 'da';
  }
  return '';
}

export function uiLangFromLanguage(languageValue, fallback = 'en') {
  const lang = String(languageValue || '').trim().replace(/_/g, '-').toLowerCase();
  if (lang.startsWith('nb') || lang.startsWith('nn') || lang === 'no') {
    return 'nb';
  }
  if (lang.startsWith('en')) {
    return 'en';
  }
  return String(fallback || 'en');
}

export function roomLanguageKey(uiLang) {
  return String(uiLang || '').trim().toLowerCase() === 'nb' ? 'nb' : 'en';
}