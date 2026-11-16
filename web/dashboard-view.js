(() => {
  if (window.__mediabridgeEmbeddedSettings) return;
  const el = id => document.getElementById(id);
  const read = (key, fallback) => {
    try { return JSON.parse(localStorage.getItem('mediabridge.view.' + key)) ?? fallback; }
    catch { return fallback; }
  };
  const save = (key, value) => {
    try { localStorage.setItem('mediabridge.view.' + key, JSON.stringify(value)); } catch {}
  };
  const settings = {
    view: read('view', 'simple'), theme: read('theme', 'crimson'),
    opacity: read('opacity', '88'), background: read('background', ''),
    wallpaperOpacity: read('wallpaperOpacity', '35'), compact: read('compact', false),
    collapsed: read('collapsed', true), density: read('density', 'comfortable'), feed: read('feed', 'auto'),
  };
  const controls = {
    view: 'dashboard-view', theme: 'dashboard-theme', opacity: 'dashboard-opacity',
    background: 'dashboard-background', wallpaperOpacity: 'wallpaper-opacity', compact: 'dashboard-sidebar', density: 'dashboard-density', feed: 'dashboard-feed',
  };
  const panels = ['personal-nav', 'personal-options', 'personal-recommendations', 'personal-bookmarks', 'collapse-media'];
  const cache = new Map();
  let appliedView;
  let current = '', pending = false, stylesheet;
  const personal = () => settings.view === 'personal';

  function apply() {
    const enabled = personal();
    if (enabled && !stylesheet) {
      stylesheet = document.createElement('link');
      stylesheet.rel = 'stylesheet';
      stylesheet.href = '/dashboard-personal.css';
      document.head.append(stylesheet);
    }
    for (const id of panels) el(id).hidden = !enabled;
    document.body.classList.toggle('personal', enabled);
    document.body.classList.toggle('compact-sidebar', settings.compact);
    document.body.classList.toggle('collapsed-media', settings.collapsed);
    document.body.dataset.theme = settings.theme;
    document.body.dataset.density = settings.density;
    el('dashboard-sidebar').value = String(settings.compact);
    el('personal-recommendations').hidden = !enabled || settings.feed === 'off';
    for (const swatch of document.querySelectorAll('[data-theme-preset]')) {
      swatch.setAttribute('aria-pressed', String(swatch.dataset.themePreset === settings.theme));
    }
    document.body.style.setProperty('--interface-opacity', Number(settings.opacity) / 100);
    document.body.style.setProperty('--wallpaper-opacity', Number(settings.wallpaperOpacity) / 100);
    let wallpaper = enabled
      ? 'radial-gradient(ellipse at top right, var(--accent, #6a273a), transparent 65%)'
      : 'none';
    try {
      const url = new URL(settings.background);
      if (enabled && url.protocol === 'https:') wallpaper = `url(${JSON.stringify(url.href)})`;
    } catch {}
    document.body.style.setProperty('--wallpaper', wallpaper);
    el('collapse-media').textContent = settings.collapsed ? 'Expand controls' : 'Collapse controls';
    el('collapse-media').setAttribute('aria-expanded', String(!settings.collapsed));
    el('sidebar-toggle').setAttribute('aria-expanded', String(!settings.compact));
    if (appliedView === enabled) return;
    appliedView = enabled;
    window.dispatchEvent(new Event('mediabridge-view-change'));
    if (enabled) loadRecommendations();
    else el('recommendation-videos').replaceChildren();
  }

  for (const [key, id] of Object.entries(controls)) {
    const input = el(id);
    input.value = settings[key];
    input.addEventListener(input.type === 'range' ? 'input' : 'change', () => {
      settings[key] = key === 'compact' ? input.value === 'true' : input.value;
      save(key, settings[key]);
      apply();
      if (key === 'feed') loadRecommendations();
    });
  }
  for (const [id, key] of [['sidebar-toggle', 'compact'], ['collapse-media', 'collapsed']]) {
    el(id).onclick = () => { settings[key] = !settings[key]; save(key, settings[key]); apply(); };
  }

  el('personal-appearance').onclick = () => {
    el('toggle-settings').click();
    el('appearance-settings').open = true;
    el('dashboard-view').focus();
  };
  for (const swatch of document.querySelectorAll('[data-theme-preset]')) {
    swatch.onclick = () => {
      el('dashboard-theme').value = swatch.dataset.themePreset;
      el('dashboard-theme').dispatchEvent(new Event('change'));
    };
  }
  function videoId(value) {
    try {
      const url = new URL(value);
      const host = url.hostname;
      if (host !== 'youtu.be' && host !== 'youtube.com' && !host.endsWith('.youtube.com')) return null;
      const id = url.searchParams.get('v') || url.pathname.split('/').at(-1);
      return /^[\w-]{11}$/.test(id || '') ? id : null;
    } catch { return null; }
  }
  function button(label, action) {
    const element = document.createElement('button');
    element.type = 'button';
    element.textContent = label;
    element.onclick = action;
    return element;
  }
  function renderRecommendations(result) {
    const cards = result.videos.map(video => {
      const id = videoId(video.url);
      if (!id) return null;
      const card = document.createElement('article');
      const image = document.createElement('img');
      image.src = `https://i.ytimg.com/vi/${id}/mqdefault.jpg`;
      image.alt = '';
      image.loading = 'lazy';
      image.referrerPolicy = 'no-referrer';
      const title = document.createElement('strong');
      title.textContent = video.title || id;
      const action = target => () => {
        el('url').value = `https://www.youtube.com/watch?v=${id}`;
        el('url').dispatchEvent(new Event('input'));
        el(target).click();
      };
      const actions = document.createElement('div');
      actions.className = 'row';
      actions.append(button('Play', action('play-now')), button('Queue', action('play-last')));
      card.append(image, title, actions);
      return card;
    }).filter(Boolean);
    el('recommendation-videos').replaceChildren(...cards);
    el('recommendation-status').textContent = cards.length
      ? (result.signed_in ? 'Personalized by YouTube' : 'Public YouTube recommendations')
      : 'YouTube returned no recommendations.';
  }
  const activeSeed = () => settings.feed === 'home' ? null : (current || null);
  async function loadRecommendations(force = false) {
    if (!personal() || settings.feed === 'off') return;
    const seed = activeSeed();
    el('recommendation-heading').textContent = seed ? 'Related videos' : 'Your YouTube Home';
    if (pending) return;
    if (!force && cache.has(seed)) {
      const result = cache.get(seed);
      if (result) renderRecommendations(result);
      return;
    }
    pending = true;
    el('refresh-recommendations').disabled = true;
    el('recommendation-status').textContent = 'Loading recommendations…';
    try {
      const response = await fetch(seed ? '/recommendations' : '/recommendations/home', {cache:'no-store'});
      if (!response.ok) throw new Error(response.status === 404 && !seed ? 'YouTube Home needs an updated MediaBridge server.' : response.status === 503
        ? 'The recommendation worker is busy. Try Refresh shortly.'
        : 'Recommendations are unavailable on this server.');
      const result = await response.json();
      if (result.seed_video_id !== (seed || null) || !Array.isArray(result.videos)) throw new Error('The playing video changed. Select Refresh to try again.');
      if (cache.size >= 20) cache.delete(cache.keys().next().value);
      cache.set(seed, result);
      if (personal() && activeSeed() === seed) renderRecommendations(result);
    } catch (error) {
      cache.set(seed, null);
      if (personal() && activeSeed() === seed) el('recommendation-status').textContent = error.message;
    } finally {
      pending = false;
      el('refresh-recommendations').disabled = false;
      if (activeSeed() !== seed) loadRecommendations();
    }
  }
  el('refresh-recommendations').onclick = () => loadRecommendations(true);
  window.updatePersonalDashboard = state => {
    const seed = videoId(state.current);
    if (seed === current) return;
    current = seed;
    if (settings.feed === 'home') return;
    el('recommendation-videos').replaceChildren();
    loadRecommendations();
  };

  const bookmarks = {channels: read('channels', []), sunshine: read('sunshine', [])};
  function renderBookmarks(type) {
    const list = el(type === 'channels' ? 'saved-channels' : 'saved-sunshine');
    list.replaceChildren(...bookmarks[type].map((item, index) => {
      const row = document.createElement('li');
      row.className = 'row';
      if (type === 'channels') {
        const link = document.createElement('a');
        link.href = item.url;
        link.target = '_blank';
        link.rel = 'noopener noreferrer';
        link.textContent = item.label;
        row.append(link);
      } else {
        row.append(document.createTextNode(`${item.host} · ${item.app} `), button('Connect', () => {
          el('sunshine-host').value = item.host;
          el('sunshine-app').value = item.app;
          el('sunshine-host').dispatchEvent(new Event('input'));
          el('sunshine-connect').closest('details')?.setAttribute('open', '');
          el('sunshine-connect').click();
        }));
      }
      row.append(button('Remove', () => {
        bookmarks[type].splice(index, 1);
        save(type, bookmarks[type]);
        renderBookmarks(type);
      }));
      return row;
    }));
  }
  el('save-channel').onsubmit = event => {
    event.preventDefault();
    const form = event.currentTarget;
    const url = new URL(form.elements.url.value);
    if (url.protocol !== 'https:' || !['youtube.com', 'www.youtube.com'].includes(url.hostname)
        || !/^\/(?:@[^/]+|(?:channel|c|user)\/[^/]+)/.test(url.pathname)) {
      form.elements.url.setCustomValidity('Enter a YouTube channel URL.');
      form.elements.url.reportValidity();
      return;
    }
    bookmarks.channels.push({label: form.elements.label.value.trim(), url: url.href});
    save('channels', bookmarks.channels);
    renderBookmarks('channels');
    form.reset();
  };
  el('save-channel').elements.url.oninput = event => event.target.setCustomValidity('');
  el('save-sunshine').onclick = () => {
    const host = el('sunshine-host').value.trim();
    const app = el('sunshine-app').value.trim() || 'Desktop';
    if (!host) { el('sunshine-host').focus(); return; }
    if (bookmarks.sunshine.some(item => item.host === host && item.app === app)) return;
    bookmarks.sunshine.push({host, app});
    save('sunshine', bookmarks.sunshine);
    renderBookmarks('sunshine');
  };
  renderBookmarks('channels');
  renderBookmarks('sunshine');
  apply();
})();
