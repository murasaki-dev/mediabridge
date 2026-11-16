window.__mediabridgeEmbeddedSettings = true;
document.documentElement.dataset.settingsContext = window.__mediabridgePreviewSettings ? 'preview' : 'native';
if (window.__mediabridgePreviewSettings) {
  const nativeFetch = window.fetch.bind(window);
  window.fetch = (input, options) => nativeFetch(
    typeof input === 'string' && /^\/(?!\/)/.test(input) ? '/host-api' + input : input, options);
}
document.addEventListener('DOMContentLoaded', () => {
  const settings = document.getElementById('settings-drawer');
  if (!settings) return;
  settings.removeAttribute('inert');
  settings.removeAttribute('aria-hidden');
  settings.removeAttribute('aria-modal');
  settings.setAttribute('role', 'region');
  for (const card of settings.querySelectorAll(':scope > details.settings-group')) {
    const summary = card.querySelector(':scope > summary');
    if (!summary) continue;
    const heading = document.createElement('h2');
    heading.className = 'settings-heading';
    heading.textContent = summary.textContent.trim();
    card.open = true;
    summary.hidden = true;
    summary.after(heading);
  }
  if (!window.__mediabridgePreviewSettings) {
    const accounts = document.getElementById('account-setup-card');
    const slot = document.getElementById('account-settings-slot');
    if (accounts && slot) slot.append(accounts);
    const done = document.getElementById('finish-account-setup');
    if (done) done.hidden = true;
  }
});
