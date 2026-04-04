export function byId(id) {
  return document.getElementById(id);
}

export function isLocalhostLikeHost(hostname) {
  const host = String(hostname || '').toLowerCase();
  return host === 'localhost' || host === '127.0.0.1' || host.endsWith('.localhost');
}

export function isGatewayViewOrigin() {
  return isLocalhostLikeHost(window.location.hostname) && window.location.port === '8080';
}
