<script lang="ts">
  import '../app.css';
  import {page} from '$app/state';
  import {onNavigate} from '$app/navigation';
  import Icon from '$lib/Icon.svelte';
  import Dashboard from '$lib/Dashboard.svelte';
  import {preference, store} from '$lib/api';
  import type {Status} from '$lib/api';
  import {setDashboardContext} from '$lib/playback-context';
  let {children} = $props();
  let hostPlayback = $state<Status | null>();
  let colorScheme = $state<'dark' | 'light'>('dark');
  setDashboardContext({playback: () => hostPlayback, colorScheme: () => colorScheme});
  let dashboardMode = $state(preference<'simple' | 'fancy'>('dashboard-mode','simple'));
  let fancy = $derived(dashboardMode === 'fancy');
  let settingsPage = $derived(['/settings','/appearance'].includes(page.url.pathname));
  let appearanceTab = $derived(page.url.pathname === '/appearance' || page.url.pathname === '/settings' && page.url.searchParams.get('section') === 'appearance');
  let mode = $state(preference('mode','dark'));
  let seed = $state(preference('seed','#ee647c'));
  let glass = $state(preference('glass',false));
  let sidebarOpacity = $state(preference('sidebar-opacity',94));
  let playerOpacity = $state(preference('player-opacity',94));
  let opacity = $state(preference('opacity',94));
  let wallpaper = $state(preference('wallpaper',true));
  let motion = $state(preference('motion',100));
  let wallpaperOpacity = $state(preference('wallpaper-opacity',42));
  let uploadStatus = $state('');
  let uploading = $state(false);
  let wallpaperVersion = $state(0);
  let sidebarWidth = $state(preference('sidebar-width',214));
  let resizing = $state(false);
  let compact = $state(preference('compact',false));


  let settingsFrame = $state<HTMLIFrameElement>();
  let settingsDocument = $state<Document>();
  let embeddedMode = $state<HTMLSelectElement>();
  function loadSettings() {
    const doc = settingsFrame?.contentDocument;
    settingsDocument = doc ?? undefined;
    embeddedMode = undefined;
    const drawer = doc?.getElementById('settings-drawer');
    if (!doc || !drawer) return;

    const group = doc.createElement('section');
    group.className = 'settings-group';
    group.innerHTML = '<h2 class="settings-heading">Dashboard</h2><label class="dashboard-mode-setting">Dashboard mode <select><option value="simple">Simple</option><option value="fancy">Fancy</option></select></label>';
    const select = group.querySelector('select')!;
    select.value = dashboardMode;
    select.onchange = () => { dashboardMode = select.value === 'fancy' ? 'fancy' : 'simple'; };
    drawer.querySelector('.settings-group')?.before(group);
    embeddedMode = select;
  }
  $effect(() => {
    if (embeddedMode) embeddedMode.value = dashboardMode;
  });
  function applySettingsTheme() {
    const target = settingsDocument?.documentElement;
    if (!target) return;
    const root = document.documentElement.style;
    for (const name of Array.from(target.style)) {
      if (name.startsWith('--md-sys-color-')) target.style.removeProperty(name);
    }
    for (const name of Array.from(root)) {
      if (name.startsWith('--md-sys-color-')) target.style.setProperty(name, root.getPropertyValue(name));
    }
    if (!fancy) {
      const base = getComputedStyle(document.documentElement);
      for (const role of ['on-surface','on-surface-variant','surface-container','surface-container-low',
        'surface-container-high','surface-container-highest','outline-variant','primary',
        'secondary-container','on-secondary-container']) {
        const name = '--md-sys-color-' + role;
        target.style.setProperty(name, base.getPropertyValue(name));
      }
    }
    target.style.colorScheme = root.colorScheme || 'dark';
    target.style.setProperty('--panel-opacity', String(fancy ? opacity / 100 : 1));
  }
  $effect(() => {
    const frame = settingsFrame;
    const content = settingsDocument?.querySelector('#settings-drawer') ?? settingsDocument?.body;
    if (!frame || !content) return;
    const fit = () => { frame.style.height = Math.ceil(content.getBoundingClientRect().height) + 2 + 'px'; };
    const observer = new ResizeObserver(fit);
    observer.observe(content);
    fit();
    function scrollPage(event: WheelEvent) {
      if (event.defaultPrevented || event.ctrlKey || !event.deltaY) return;
      event.preventDefault();
      const unit = event.deltaMode === 1 ? 16 : event.deltaMode === 2 ? window.innerHeight : 1;
      window.scrollBy({top:event.deltaY * unit, behavior:'instant'});
    }
    let touchY: number | undefined;
    function startTouch(event: TouchEvent) {
      const target = event.target as HTMLElement | null;
      touchY = event.touches.length === 1 && !target?.closest('input[type="range"],textarea,select')
        ? event.touches[0].screenY : undefined;
    }
    function moveTouch(event: TouchEvent) {
      if (touchY === undefined || event.touches.length !== 1 || !event.cancelable) return;
      const y = event.touches[0].screenY;
      const distance = touchY - y;
      if (Math.abs(distance) < 4) return;
      touchY = y;
      event.preventDefault();
      window.scrollBy({top:distance, behavior:'instant'});
    }
    function scrollWithKeys(event: KeyboardEvent) {
      const target = event.target as HTMLElement | null;
      if (event.defaultPrevented || event.altKey || event.ctrlKey || event.metaKey
          || target?.closest('input,textarea,select,[contenteditable]')) return;
      const distance = event.key === 'PageDown' ? window.innerHeight * .9
        : event.key === 'PageUp' ? -window.innerHeight * .9 : 0;
      if (!distance) return;
      event.preventDefault();
      window.scrollBy({top:distance, behavior:'instant'});
    }
    const doc = content.ownerDocument;
    doc.addEventListener('wheel', scrollPage, {passive:false});
    doc.addEventListener('touchstart', startTouch, {passive:true});
    doc.addEventListener('touchmove', moveTouch, {passive:false});
    doc.addEventListener('keydown', scrollWithKeys);
    return () => {
      observer.disconnect();
      doc.removeEventListener('wheel', scrollPage);
      doc.removeEventListener('touchstart', startTouch);
      doc.removeEventListener('touchmove', moveTouch);
      doc.removeEventListener('keydown', scrollWithKeys);
    };
  });
  $effect(() => {
    const isFancy = fancy;
    const themeMode = mode, themeSeed = seed;
    void settingsDocument; void opacity;
    const root = document.documentElement.style;
    if (!isFancy) {
      colorScheme = 'dark';
      for (const name of Array.from(root)) {
        if (name.startsWith('--md-sys-color-')) root.removeProperty(name);
      }
      root.colorScheme = '';
      applySettingsTheme();
      return;
    }
    const mq = matchMedia('(prefers-color-scheme: dark)');
    let cancelled = false;
    const sync = async () => {
      const dark = themeMode === 'dark' || themeMode === 'system' && mq.matches;
      colorScheme = dark ? 'dark' : 'light';
      const {applyTheme} = await import('$lib/theme');
      if (cancelled) return;
      applyTheme(themeSeed, dark);
      applySettingsTheme();
    };
    void sync(); mq.addEventListener('change',sync);
    return () => { cancelled = true; mq.removeEventListener('change',sync); };
  });
  $effect(() => {
    store('dashboard-mode', dashboardMode);
    store('mode',mode); store('seed',seed); store('glass',glass); store('opacity',opacity);
    store('wallpaper',wallpaper); store('compact',compact); store('motion',motion); store('wallpaper-opacity',wallpaperOpacity);
  });
  $effect(() => {
    document.documentElement.style.setProperty('--motion', (fancy ? motion * 3 : 0) + 'ms');
    store('sidebar-width', sidebarWidth);
    store('sidebar-opacity', sidebarOpacity);
    store('player-opacity', playerOpacity);
  });
  onNavigate(navigation => {
    if (!fancy || !document.startViewTransition || motion === 0 || matchMedia('(prefers-reduced-motion: reduce)').matches) return;
    return new Promise<void>(resolve => {
      document.startViewTransition(async () => {
        resolve();
        await navigation.complete;
      });

    });
  });
  function resizeSidebar(event: PointerEvent & {currentTarget: HTMLButtonElement}) {
    if (event.button !== 0 || compact) return;
    resizing = true;
    event.currentTarget.setPointerCapture(event.pointerId);
  }
  function moveSidebar(event: PointerEvent) {
    if (resizing) sidebarWidth = Math.max(170, Math.min(340, Math.round(event.clientX)));
  }
  async function uploadWallpaper(event: Event & {currentTarget: HTMLInputElement}) {
    const field = event.currentTarget;
    const file = field.files?.[0];
    if (!file) return;
    uploading = true; uploadStatus = '';
    try {
      const response = await fetch('/appearance/wallpaper', {method:'PUT', body:file});
      if (!response.ok) throw new Error(await response.text());
      wallpaper = true; wallpaperVersion = Date.now(); uploadStatus = 'Wallpaper uploaded.';
    } catch (error) { uploadStatus = error instanceof Error ? error.message : 'Upload failed.'; }
    finally { uploading = false; field.value = ''; }
  }
  async function resetWallpaper() {
    uploading = true;
    try {
      const response = await fetch('/appearance/wallpaper', {method:'DELETE'});
      if (!response.ok) throw new Error(await response.text());
      wallpaperVersion = Date.now(); uploadStatus = 'Default wallpaper restored.';
    } catch (error) { uploadStatus = error instanceof Error ? error.message : 'Could not reset wallpaper.'; }
    finally { uploading = false; }
  }
</script>
<svelte:head><title>MediaBridge</title><meta name="description" content="Control media playback on your MediaBridge host."><link rel="icon" href="/icon.png"></svelte:head>
{#snippet settingsTabs()}
  <nav class="library-tabs settings-tab-list" aria-label="Settings categories">
    <a class="library-tab" aria-current={!appearanceTab ? 'page' : undefined} href="/settings">Server settings</a>
    <a class="library-tab" aria-current={appearanceTab ? 'page' : undefined} href="/settings?section=appearance">Appearance</a>
  </nav>
{/snippet}
<div class:compact={fancy && compact} class:glass={fancy && glass} class:resizing class:simple={!fancy} class="shell" style:--sidebar-width={(fancy ? sidebarWidth : 196) + 'px'} style:--panel-opacity={fancy ? opacity / 100 : 1} style:--motion={(fancy ? motion * 3 : 0) + 'ms'} style:--wallpaper-opacity={wallpaperOpacity / 100} style:--sidebar-opacity={fancy ? sidebarOpacity / 100 : 1} style:--player-opacity={fancy ? playerOpacity / 100 : 1}>
  <aside class="sidebar">
    <div class="brand-row"><a class="brand" href="/" aria-label="MediaBridge home"><img src="/icon.png" width="34" height="34" alt=""><span>MediaBridge</span></a>{#if fancy}<button class="icon sidebar-collapse" title={compact ? 'Expand sidebar' : 'Collapse sidebar'} aria-label={compact ? 'Expand sidebar' : 'Collapse sidebar'} aria-expanded={!compact} onclick={() => compact = !compact}><Icon name={compact ? 'right' : 'left'} size={16}/></button>{/if}</div>
    <nav aria-label="Main navigation">
      <a aria-current={page.url.pathname === '/' && !page.url.search ? 'page' : undefined} href="/" aria-label="Home"><Icon name="home"/><span>Home</span></a>
      <a aria-current={page.url.pathname === '/search' ? 'page' : undefined} href="/search" aria-label="Search"><Icon name="search"/><span>Search</span></a>
      <a aria-current={page.url.pathname === '/chat' ? 'page' : undefined} href="/chat" aria-label="Live chat"><Icon name="chat"/><span>Chat</span></a>
      <a aria-current={page.url.pathname === '/mousepad' ? 'page' : undefined} href="/mousepad" aria-label="Mousepad"><Icon name="mouse"/><span>Mousepad</span></a>
      {#if fancy}
      <a href="/?view=subscriptions" aria-label="Subscriptions" aria-current={page.url.searchParams.get('view') === 'subscriptions' ? 'page' : undefined}><Icon name="subscriptions"/><span>Subscriptions</span></a>
      <a href="/?view=history" aria-label="History" aria-current={['history','youtube-history'].some(view => page.url.searchParams.get('view') === view) ? 'page' : undefined}><Icon name="history"/><span>History</span></a>
      {:else}
      <a href="/?view=sunshine" aria-current={page.url.searchParams.get('view') === 'sunshine' ? 'page' : undefined}><Icon name="sun"/><span>Sunshine</span></a>
      {/if}
      <a aria-label="Settings" href="/settings" aria-current={settingsPage ? 'page' : undefined}><Icon name="settings"/><span>Settings</span></a>
    </nav>
    {#if fancy}<button class="sidebar-resize" type="button" aria-label="Resize sidebar with left and right arrows" title="Drag to resize sidebar" disabled={compact}
      onpointerdown={resizeSidebar} onpointermove={moveSidebar} onlostpointercapture={() => resizing = false}
      onkeydown={event => { if (event.key === 'ArrowLeft' || event.key === 'ArrowRight') { event.preventDefault(); sidebarWidth = Math.max(170,Math.min(340,sidebarWidth + (event.key === 'ArrowLeft' ? -8 : 8))); } }}></button>{/if}
  </aside>
  {#if fancy && wallpaper}<div class="dashboard-wallpaper" aria-hidden="true" style:background-image={`url(/appearance/wallpaper?v=${wallpaperVersion}), radial-gradient(ellipse at top right, var(--md-sys-color-primary), transparent 65%)`}></div>{/if}
  <main class="main">
  <Dashboard {fancy} onPlayback={value => hostPlayback = value}/>
  {#if settingsPage && appearanceTab}
  <div class="content appearance-page">
    <h1>Settings</h1>
    {@render settingsTabs()}
    {#if fancy}
    <section class="panel appearance-panel"><h2>Theme</h2><label>Color mode <select bind:value={mode}><option value="dark">Dark</option><option value="light">Light</option><option value="system">System</option></select></label>
    <label>Accent color <input type="color" bind:value={seed}></label>
    <div class="swatches" aria-label="Accent presets">
      {#each [{name:'Crimson',color:'#ee647c'},{name:'Iris',color:'#ae8af7'},{name:'Lagoon',color:'#43b9ae'},{name:'Amber',color:'#efac55'}] as preset (preset.color)}
        <button aria-label={preset.name} title={preset.name} aria-pressed={seed === preset.color} style:background={preset.color} onclick={() => seed = preset.color}></button>
      {/each}
    </div>
    </section><section class="panel appearance-panel"><h2>Interface</h2><label>Panel opacity <input type="range" min="0" max="100" bind:value={opacity}><output>{opacity}%</output></label>
    <label>Sidebar opacity <input type="range" min="0" max="100" bind:value={sidebarOpacity}><output>{sidebarOpacity}%</output></label>
    <label>Player bar opacity <input type="range" min="0" max="100" bind:value={playerOpacity}><output>{playerOpacity}%</output></label>
    <label><input type="checkbox" bind:checked={glass}> Blur behind panels</label>
    <label><input type="checkbox" bind:checked={wallpaper}> Show wallpaper</label>
    <label><input type="checkbox" bind:checked={compact}> Compact sidebar</label>
      <label>Transition duration <input type="range" min="0" max="200" step="5" bind:value={motion}><output>{motion === 0 ? 'Off' : motion * 3 + ' ms'}</output></label>
    </section>
    <section class="panel appearance-panel">
      <h2>Wallpaper</h2>
      <label>Wallpaper opacity <input type="range" min="0" max="100" bind:value={wallpaperOpacity}><output>{wallpaperOpacity}%</output></label>
      <label>Upload wallpaper <input type="file" accept="image/png,image/jpeg,image/webp" disabled={uploading} onchange={uploadWallpaper}></label>
      <button class="tonal" disabled={uploading} onclick={resetWallpaper}>Restore default wallpaper</button>
      <p class="muted" role="status">{uploadStatus || 'PNG, JPEG, or WebP, up to 8 MB. Shared by browsers using this dashboard.'}</p>
    </section>
    {:else}
      <p class="panel appearance-panel">Enable Fancy mode in Server settings to customize appearance.</p>
    {/if}
  </div>
  {:else if settingsPage}
  <div class="content appearance-page">
    <h1>Settings</h1>
    {@render settingsTabs()}
    {#if settingsDocument && !settingsDocument.getElementById('settings-drawer')}
    <section class="panel appearance-panel" aria-label="Dashboard">
      <label>Dashboard mode <select bind:value={dashboardMode}><option value="simple">Simple</option><option value="fancy">Fancy</option></select></label>
    </section>
    {/if}
    <iframe class="server-settings-frame" bind:this={settingsFrame} onload={loadSettings} title="MediaBridge server settings" src="/host-settings"></iframe>
    <p class="muted">To import YouTube or Twitch cookies, open Accounts on the MediaBridge host at <code>http://127.0.0.1:8765/</code>.</p>
    {#if settingsDocument?.documentElement.dataset.settingsContext === 'native'}
      <a class="back-home" href="/legacy-dashboard" data-sveltekit-reload>Legacy dashboard</a>
    {/if}
  </div>
  {:else}
    {@render children()}
  {/if}
  </main>
</div>
