<script lang="ts">
  import {slide} from 'svelte/transition';
  import {page} from '$app/state';
  import {goto} from '$app/navigation';
  import Icon from '$lib/Icon.svelte';
  import MediaGrid from '$lib/MediaGrid.svelte';
  import ChannelAvatar from '$lib/ChannelAvatar.svelte';
  import {api, artwork, preference, store} from '$lib/api';
  import type {Media, Status} from '$lib/api';
  import {clock, rememberPosition, savedPosition, shortcut} from '$lib/logic';

  let {fancy,onPlayback}: {fancy:boolean; onPlayback:(status:Status | null)=>void} = $props();
  type Feed = {videos: Media[]; signed_in: boolean; seed_video_id?: string | null; snapshot?:number; next_offset?:number | null; refreshing?:boolean; long_enabled?:boolean; refresh_error?:string | null; can_extend?:boolean};
  let status = $state<Status>({});
  let offline = $state(false), loading = $state(true), busy = $state(false);
  let message = $state(''), link = $state(''), local = $state(true), query = $state('');
  let feedMode = $state<'auto' | 'home'>('auto');
  let feed = $state<Feed | null>(null), feedError = $state(''), feedLoading = $state(false), reload = $state(0);
  let nextPage = $state<HTMLDivElement>();
  let loadingMore = $state(false), refreshingFull = $state(false);
  let grid = $state<MediaGrid>();
  let requestedTab = $derived(page.url.searchParams.get('view') || 'home');
  let sunshinePage = $derived(requestedTab === 'sunshine' || requestedTab === 'subscriptions' && page.url.searchParams.get('provider') === 'sunshine');
  let currentTab = $derived(page.url.pathname === '/search' ? 'search' : page.url.pathname !== '/' ? 'other'
    : !fancy ? sunshinePage ? 'simple-sunshine' : 'simple-home'
    : ['saved','sunshine'].includes(requestedTab) ? 'subscriptions' : requestedTab);
  let libraryTab = $derived(requestedTab === 'sunshine' || page.url.searchParams.get('provider') === 'sunshine' ? 'sunshine'
    : page.url.searchParams.get('provider') === 'twitch' ? 'twitch' : 'youtube');
  let source = $derived(currentTab === 'youtube-history' ? 'youtube-history' : feedMode === 'auto' && status.current && artwork({url:status.current})?.startsWith('https://i.ytimg.com/vi/') ? 'related:' + status.current : 'home');
  let mirroring = $derived(status.mirroring === true || status.playback_state === 'mirroring');
  let playing = $derived(status.playback_state === 'playing');
  let shown = $derived((currentTab === 'queue' ? status.queue_items : currentTab === 'history' ? status.history_items : feed?.videos)?.filter(item => (item.title || item.url).toLowerCase().includes(query.trim().toLowerCase())) || []);
  let media = $derived(status.current ? {url:status.current, title:status.title, thumbnail_url:status.artwork_url} : null);
  let searchTerm = $state(''), searchError = $state(''), searched = $state(false), searching = $state(false);
  let searchResults = $state<Media[]>([]);
  let searchRequest: AbortController | undefined;
  $effect(() => {
    if (currentTab !== 'search') {
      searchRequest?.abort();
      searching = false;
    }
  });
  async function search(event: SubmitEvent) {
    event.preventDefault();
    const term = searchTerm.trim();
    if (!term || searching) return;
    const request = new AbortController();
    searchRequest = request;
    searching = true; searched = true; searchError = ''; searchResults = [];
    try {
      const result = await api<{videos:Media[]}>('/search/youtube?q=' + encodeURIComponent(term),undefined,request.signal);
      if (!request.signal.aborted) searchResults = result.videos;
    } catch (error) {
      if (!request.signal.aborted) searchError = error instanceof Error ? error.message : 'Search failed.';
    } finally { if (searchRequest === request) searching = false; }
  }
  const cache = new Map<string, {time:number; data:Feed}>();
  let activeFeed = '';
  async function requestFeed(key: string, signal: AbortSignal) {
      for (let attempt = 0; ; attempt++) {
        signal.throwIfAborted();
        try {
          if (key !== 'home') return await api<Feed>(key === 'youtube-history' ? '/history/youtube' : '/recommendations',undefined,signal);
          try { return await api<Feed>('/recommendations/home/short',undefined,signal); }
          catch (error) {
            if (!signal.aborted && error instanceof Error && /404|not found|invalid subscription provider/i.test(error.message)) return api<Feed>('/recommendations/home',undefined,signal);
            throw error;
          }
        }
        catch (error) {
          if (signal.aborted || !(error instanceof Error) || !/already running|returned 503/i.test(error.message) || attempt >= 15) throw error;
          await new Promise(resolve => setTimeout(resolve, 1000));
        }
      }
  }
  let progressSavedAt = 0;
  let servers = $state(preference<{host:string; app:string}[]>('servers', []));
  let host = $state(''), app = $state('Desktop');
  let expanded = $state(false);
  type Provider = 'youtube' | 'twitch';
  type Channel = {id:string; title:string; thumbnail_url?:string; recent_rank?:number};
  let provider = $derived<Provider>(libraryTab === 'twitch' ? 'twitch' : 'youtube');
  let channelId = $derived(currentTab === 'subscriptions' ? page.url.searchParams.get('channel') : null);
  let channelFilter = $state('');
  let channelOrder = $state<'recent' | 'name'>('recent');
  let twitchActivity = $state(preference<Record<string,number>>('twitch-channel-activity', {}));
  const channelLink = (channel: string, service: Provider) => `/?view=subscriptions&provider=${service}&channel=${encodeURIComponent(channel)}`;
  const directoryLink = (service: Provider) => `/?view=subscriptions&provider=${service}`;
  function channelSource(channel: string, service: Provider) {
    if (service === 'youtube' && /^UC[A-Za-z0-9_-]{22}$/.test(channel)) return `https://www.youtube.com/channel/${channel}`;
    if (service === 'twitch' && /^[A-Za-z0-9_]{1,25}$/.test(channel)) return `https://www.twitch.tv/${channel}`;
  }
  function openChannel(event: MouseEvent, channel: string, service: Provider) {
    if (event.button !== 0 || event.ctrlKey || event.metaKey || event.shiftKey || event.altKey) return;
    event.preventDefault();
    void goto(channelLink(channel,service));
  }
  function rankChannels(items: Channel[], service: Provider, order = channelOrder) {
    return [...items].sort((a,b) => {
      if (order === 'recent') {
        const rank = service === 'youtube'
          ? (a.recent_rank ?? Number.MAX_SAFE_INTEGER) - (b.recent_rank ?? Number.MAX_SAFE_INTEGER)
          : (twitchActivity[b.id] || 0) - (twitchActivity[a.id] || 0);
        if (rank) return rank;
      }
      return a.title.localeCompare(b.title);
    });
  }

  let homeChannels = $state<Channel[]>([]), homeChannelsError = $state(''), homeChannelsLoading = $state(false);
  const attemptedExtensions = new Set<number>();
  $effect(() => {
    if (currentTab !== 'home' || offline || loading) return;
    const cached = subscriptionCache.get('youtube');
    if (cached) { homeChannels = cached; return; }
    const controller = new AbortController();
    homeChannelsLoading = true; homeChannelsError = '';
    api<{channels:Channel[]}>('/subscriptions/youtube',undefined,controller.signal)
      .then(result => {
        if (!controller.signal.aborted) {homeChannels = result.channels; subscriptionCache.set('youtube',result.channels);}
      })
      .catch(error => {if (!controller.signal.aborted) homeChannelsError = error.message;})
      .finally(() => {if (!controller.signal.aborted) homeChannelsLoading = false;});
    return () => controller.abort();
  });
  let twitchLive = $state<Media[]>([]), twitchError = $state(''), twitchLoading = $state(false);
  let twitchFetched = 0;
  let subscriptions = $state<Channel[]>([]), channelVideos = $state<Media[]>([]);
  let selected = $derived(subscriptions.find(channel => channel.id === channelId));
  let filteredChannels = $derived(rankChannels(subscriptions.filter(channel => channel.title.toLocaleLowerCase().includes(channelFilter.trim().toLocaleLowerCase())),provider));

  let subscriptionError = $state(''), subscriptionsLoading = $state(false), videosLoading = $state(false);
  const subscriptionCache = new Map<Provider, Channel[]>();

$effect(() => {
    if (!['home','twitch-live'].includes(currentTab) || loading || offline || Date.now() - twitchFetched < 60000) return;
    const controller = new AbortController();
    twitchLoading = true;
    twitchError = '';
    api<{videos:Media[]}>('/subscriptions/twitch/live', undefined, controller.signal)
      .then(result => { if (!controller.signal.aborted) twitchLive = result.videos; })
      .catch(error => {
        if (!controller.signal.aborted) twitchError = /404|not found|invalid subscription provider/i.test(error.message)
          ? 'Twitch Live needs the new server build.' : error.message;
      })
      .finally(() => { if (!controller.signal.aborted) { twitchLoading = false; twitchFetched = Date.now(); } });
    return () => controller.abort();
  });
  $effect(() => {
    if (currentTab !== 'subscriptions' || libraryTab === 'sunshine') return;
    const service = provider;
    channelVideos = []; subscriptionError = '';
    const cached = subscriptionCache.get(service);
    if (cached) { subscriptions = cached; return; }
    subscriptions = []; subscriptionsLoading = true;
    const controller = new AbortController();
    api<{channels: Channel[]}>('/subscriptions/' + service, undefined, controller.signal)
      .then(result => { if (!controller.signal.aborted) { subscriptionCache.set(service,result.channels); subscriptions = result.channels; } })
      .catch(error => { if (!controller.signal.aborted) subscriptionError = error.message; })
      .finally(() => { if (!controller.signal.aborted) subscriptionsLoading = false; });
    return () => { controller.abort(); subscriptionsLoading = false; };
  });
  $effect(() => {
    if (currentTab !== 'subscriptions' || libraryTab === 'sunshine' || !channelId) return;
    const controller = new AbortController();
    channelVideos = []; videosLoading = true; subscriptionError = '';
    api<{videos:Media[]}>(`/subscriptions/${provider}/${encodeURIComponent(channelId)}`, undefined, controller.signal)
            .then(result => {
        if (controller.signal.aborted) return;
        channelVideos = result.videos;
        if (provider === 'twitch') {
          const latest = Math.max(0, ...result.videos.map(video => Date.parse(video.published || '') || 0));
          if (latest && channelId) {
            twitchActivity = {...twitchActivity, [channelId]:latest};
            store('twitch-channel-activity',twitchActivity);
          }
        }
      })
      .catch(error => { if (!controller.signal.aborted) subscriptionError = error.message; })
      .finally(() => { if (!controller.signal.aborted) videosLoading = false; });
    return () => { controller.abort(); videosLoading = false; };
  });

  async function refresh(signal?: AbortSignal) {
    try {
      const result = await api<Status>('/status', undefined, signal);
      if (signal?.aborted) return;
      status = result;
      onPlayback(result);
      offline = false;
      if (result.current && result.can_seek && Date.now() - progressSavedAt >= 5000) {
        rememberPosition(localStorage, result.current, result.position_seconds || 0);
        progressSavedAt = Date.now();
      }
    } catch { if (!signal?.aborted) {offline = true; onPlayback(null);} }
    finally { if (!signal?.aborted) loading = false; }
  }
  $effect(() => {
    const fast = fancy;
    let request: AbortController | undefined;
    let timer: ReturnType<typeof setTimeout>;
    const poll = async () => {
      if (document.hidden || request) return;
      const current = new AbortController();
      request = current;
      await refresh(current.signal);
      if (request !== current) return;
      request = undefined;
      if (!document.hidden) timer = setTimeout(poll, fast ? 1600 : playing || mirroring ? 2500 : 10000);
    };
    const stopPolling = () => { clearTimeout(timer); request?.abort(); request = undefined; };
    const visibility = () => { if (document.hidden) stopPolling(); else void poll(); };
    document.addEventListener('visibilitychange',visibility);
    void poll();
    return () => { stopPolling(); document.removeEventListener('visibilitychange',visibility); };
  });

  $effect(() => {
    const key = source;
    void reload;
    if (!['home','recommendations','youtube-history'].includes(currentTab) || loading || offline) return;
    const cached = cache.get(key);
    feedError = '';
    if (cached && Date.now() - cached.time < 120000) { activeFeed = key; feed = cached.data; feedLoading = false; return; }
    const controller = new AbortController();
    if (activeFeed !== key) feed = null;
    activeFeed = key;
    feedLoading = true;
    requestFeed(key,controller.signal)
      .then(data => {
        if (controller.signal.aborted) return;
        if (cache.size >= 12) cache.delete(cache.keys().next().value!);
        cache.set(key,{time:Date.now(),data}); feed = data;
      })
      .catch(error => { if (!controller.signal.aborted) feedError = error.message; })
      .finally(() => { if (!controller.signal.aborted) feedLoading = false; });
    return () => controller.abort();
  });
  $effect(() => { void query; void currentTab; void source; grid?.reset(); });

  let extensionExhausted = $state(false);
  let extensionVisits = 0;
  const hasMoreHome = $derived(Boolean(feed?.next_offset || feed?.refreshing || feed?.can_extend && !extensionExhausted));
  async function loadMoreHome() {
    const current = feed;
    if (!current || current.snapshot === undefined || loadingMore || source !== 'home') return;
    loadingMore = true;
    try {
      let next: Feed;
      if (current.next_offset != null) {
        next = await api<Feed>(`/recommendations/home/short?snapshot=${current.snapshot}&offset=${current.next_offset}`);
      } else {
        if (!current.refreshing) {
          if (!current.can_extend || extensionVisits >= 2 || attemptedExtensions.has(current.snapshot)) { extensionExhausted = true; return; }
          attemptedExtensions.add(current.snapshot);
          extensionVisits++;
          await api('/recommendations/home/extend',{});
        }
        for (let i=0; ; i++) {
          next = await api<Feed>(`/recommendations/home/short?offset=${current.videos.length}`);
          if (!fancy || !next.refreshing || next.snapshot !== current.snapshot || i >= 29 || source !== 'home') break;
          await new Promise(resolve => setTimeout(resolve,1000));
        }
      }
      if (!fancy || source !== 'home' || feed?.snapshot !== current.snapshot) return;
      if (!next.videos.length && !next.next_offset && !next.refreshing) extensionExhausted = true;
      const seen = new Set(current.videos.map(item => item.url));
      feed = {...next, videos:[...current.videos,...next.videos.filter(item => !seen.has(item.url))]};
      cache.set('home',{time:Date.now(),data:feed});
    } catch (error) {
      feedError = error instanceof Error ? error.message : 'Could not load more recommendations.';
    } finally { loadingMore = false; }
  }
  $effect(() => {
    if (currentTab !== 'recommendations' || !nextPage || source !== 'home' || !hasMoreHome || feedError || refreshingFull || loadingMore || offline) return;
    const observer = new IntersectionObserver(entries => {
      if (entries.some(entry => entry.isIntersecting)) void loadMoreHome();
    }, {rootMargin:'160px'});
    observer.observe(nextPage);
    return () => observer.disconnect();
  });

  async function refreshFullFeed(full = true) {
    refreshingFull = true; feedError = '';
    try {
      extensionExhausted = false; extensionVisits = 0; attemptedExtensions.clear();
      const path = full ? '/recommendations/home/long' : '/recommendations/home/short';
      await api(path,{});
      for (let i=0; i<32; i++) {
        if (!fancy) return;
        const data = await api<Feed>(path);
        if (!data.refreshing) {
          if (data.refresh_error) throw new Error(data.refresh_error);
          cache.set('home',{time:Date.now(),data});
          if (source === 'home') feed = data;
          return;
        }
        await new Promise(resolve => setTimeout(resolve,1000));
      }
      throw new Error('The full feed is still refreshing. Try again shortly.');
    } catch (error) {
      if (source === 'home') feedError = error instanceof Error ? error.message : 'Full refresh failed.';
    } finally { refreshingFull = false; }
  }
  function refreshDashboardHome() {
    if (source === 'home' && feed?.snapshot !== undefined) void refreshFullFeed(Boolean(feed.long_enabled));
    else { cache.delete(source); reload++; }
  }
  async function command(path: string, body: unknown = {}, feedback = '') {
    if (busy || offline) return;
    busy = true; message = '';
    try { await api(path, body); message = feedback; await refresh(); }
    catch (error) { message = error instanceof Error ? error.message : 'The action failed.'; }
    finally { busy = false; }
  }
  function play(item: Media, resume = false) {
    const seconds = resume ? savedPosition(localStorage,item.url) : 0;
    void command('/play_preserve_queue',{url:item.url,title:item.title,local,start_position_seconds:seconds || undefined},'Playback requested.');
  }
  function queue(item: Media) { void command('/queue',{url:item.url,title:item.title,local},'Added to queue.'); }
  function submit(enqueue: boolean) {
    const url = link.trim();
    if (!url) return;
    try { if (!['https:','http:','smb:'].includes(new URL(url).protocol)) throw new Error(); }
    catch { message = 'Enter a complete HTTP, HTTPS, or SMB link.'; return; }
    if (enqueue) queue({url}); else play({url});
  }
  function dockDuration() {
    return !fancy || matchMedia('(prefers-reduced-motion: reduce)').matches ? 0 : 3 * preference('motion',100);
  }
  function seek(seconds: number) {
    if (status.can_seek) void command('/seek',{position:Math.max(0,Math.min(status.duration_seconds || 0,seconds))});
  }
  function keyboard(event: KeyboardEvent) {
    const editing = event.target instanceof Element && !!event.target.closest('input,select,textarea,button,a,[contenteditable],dialog');
    const action = shortcut(event.key,editing,event.altKey || event.ctrlKey || event.metaKey || event.shiftKey);
    if (!action || !status.current || offline || document.querySelector('dialog[open]')) return;
    if (action !== 'toggle' && !status.can_seek) return;
    event.preventDefault();
    if (action === 'toggle') void command(playing ? '/pause' : '/resume');
    else seek((status.position_seconds || 0) + (action === 'back' ? -10 : 10));
  }
  function saveServer(event: SubmitEvent) {
    event.preventDefault();
    const address = host.trim(), application = app.trim() || 'Desktop';
    if (!address || address.startsWith('-') || /\s/.test(address)) { message = 'Enter a Sunshine host name or IP address.'; return; }
    if (!servers.some(server => server.host === address && server.app === application)) servers = [...servers,{host:address,app:application}];
    store('servers',servers); host = '';
  }
</script>
<svelte:window onkeydown={keyboard}/>
<section class="player-dock" aria-label="Host player" style:--track-art={fancy && media && artwork(media,'hqdefault') ? `url(${JSON.stringify(artwork(media,'hqdefault'))})` : 'none'}>
  <div class="player-strip">
    <div class="track-identity">
      {#if fancy && media && artwork(media)}<img alt="" src={artwork(media)} width="48" height="36">{:else}<Icon name="watch"/>{/if}
      <div class="top-track"><span class="eyeline">NOW PLAYING</span><strong>{status.title || status.current || (mirroring ? 'Moonlight stream' : 'Nothing playing')}</strong></div>
    </div>
    <div class="player-actions">
      <button class="icon" aria-label="Previous" title="Previous" disabled={!status.previous_available || busy || offline} onclick={() => command('/previous')}><Icon name="previous" size={17}/></button>
      <button class="icon player-primary" aria-label={playing ? 'Pause' : 'Resume'} disabled={!status.current || busy || offline} onclick={() => command(playing ? '/pause' : '/resume')}><Icon name={playing ? 'pause' : 'play'} size={18}/></button>
      <button class="icon" aria-label="Next" title="Next" disabled={offline || busy || !status.queue_items?.length} onclick={() => command('/next')}><Icon name="next" size={17}/></button>
    </div>
    <div class="player-timeline">
      <span>{clock(status.position_seconds)}</span>
      <input type="range" aria-label="Seek" min="0" max={status.duration_seconds || 1} value={status.position_seconds || 0} step="1" disabled={!status.can_seek || offline || busy} onchange={event => seek(event.currentTarget.valueAsNumber)}>
      <span>{clock(status.duration_seconds)}</span>
    </div>
    <label class="player-volume" title="Volume"><Icon name="volume" size={17}/><input type="range" aria-label="Volume" min="0" max="1" step=".01" value={status.volume ?? 1} disabled={offline || busy || status.volume == null} onchange={event => command('/volume',{volume:event.currentTarget.valueAsNumber})}></label>
    <button class="icon" aria-label="Close all playback" title="Close all playback" disabled={offline || busy || (!status.current && !mirroring)} onclick={() => command('/close')}><Icon name="close" size={18}/></button>
    {#if currentTab !== 'simple-home'}<button class="icon" title={expanded ? 'Collapse queue' : 'Expand queue'} aria-label={expanded ? 'Collapse queue' : 'Expand queue'} aria-expanded={expanded} aria-controls="dock-queue" onclick={() => expanded = !expanded}><Icon name={expanded ? 'up' : 'down'} size={19}/></button>{/if}
  </div>
  {#if expanded && currentTab !== 'simple-home'}
    <div class="dock-queue" id="dock-queue" transition:slide={{duration:dockDuration()}}>
      {@render playbackQueue()}
    </div>
  {/if}
</section>
{#snippet playbackQueue()}
    <form class="panel link-form" onsubmit={event => {event.preventDefault(); submit(false);}}>
      <Icon name="link"/><input aria-label="Media link" placeholder="Paste a video or media link" required bind:value={link}>
      <label class="mode"><select aria-label="Playback device" value={String(local)} onchange={event => local = event.currentTarget.value === 'true'}><option value="true">Host player</option><option value="false">Host browser</option></select></label>
      <button class="filled" disabled={offline || busy || !link.trim()}><Icon name="play"/> Play</button>
      <button type="button" class="tonal" disabled={offline || busy || !link.trim()} onclick={() => submit(true)}><Icon name="plus"/> Queue</button>
    </form>
      <div class="section-head"><h2>Up next <small>{status.queue_items?.length || 0} queued</small></h2>
        {#if fancy}<a class="tonal button" href="/?view=history">History</a>{/if}<span class="connection" class:offline><i></i>{loading ? 'Connecting' : offline ? 'Offline' : 'Connected'}</span>
        <button class="tonal" disabled={offline || busy} onclick={() => command('/autoplay',{enabled:!status.autoplay_enabled})}>Autoplay {status.autoplay_enabled ? 'on' : 'off'}</button>
      </div>
      {#if !status.queue_items?.length}<p class="muted">The queue is empty.</p>{/if}
      <ol>
        {#each status.queue_items || [] as item, index}
          <li>
            <span class="muted">{index + 1}</span>
            {#if fancy && artwork(item)}<img src={artwork(item)} alt="" width="56" height="34" loading="lazy" decoding="async">{/if}
            <strong title={item.title || item.url}>{item.title || item.url}</strong>
            <div class="player-actions">
              <button class="icon" title="Play now" aria-label={'Play queue item ' + (index + 1)} disabled={offline || busy} onclick={() => command('/queue/play',{index})}><Icon name="play" size={16}/></button>
              <button class="icon" title="Move up" aria-label={'Move queue item ' + (index + 1) + ' up'} disabled={offline || busy || index === 0} onclick={() => command('/queue/move',{index,to:index-1})}><Icon name="up" size={16}/></button>
              <button class="icon" title="Move down" aria-label={'Move queue item ' + (index + 1) + ' down'} disabled={offline || busy || index + 1 === status.queue_items?.length} onclick={() => command('/queue/move',{index,to:index+1})}><Icon name="down" size={16}/></button>
              <button class="icon" title="Remove" aria-label={'Remove queue item ' + (index + 1)} disabled={offline || busy} onclick={() => command('/queue/remove',{index})}><Icon name="close" size={16}/></button>
            </div>
          </li>
        {/each}
      </ol>
{/snippet}
{#snippet sunshineControls()}
    <section class="panel saved" id="saved-sunshine"><h2>Sunshine servers</h2>
      <form class="link-form" onsubmit={saveServer}><input aria-label="Sunshine host" placeholder="Host name or IP" required bind:value={host}><input aria-label="Sunshine app" placeholder="Desktop" bind:value={app}><button class="filled"><Icon name="plus"/> Save</button></form>
      {#if !servers.length}<p class="muted">Bookmark a paired Sunshine server to launch it from here.</p>{/if}
      {#each servers as server (`${server.host}/${server.app}`)}<div class="saved-row"><Icon name="sun"/><div class="grow"><strong>{server.host}</strong><small>{server.app}</small></div><button class="tonal" disabled={offline || busy} onclick={() => command('/sunshine',server,'Moonlight launch requested.')}>Connect</button><button class="icon" aria-label={'Remove '+server.host} onclick={() => {servers = servers.filter(item => item !== server); store('servers',servers);}}><Icon name="close"/></button></div>{/each}
    </section>
{/snippet}
{#if currentTab !== 'other'}<div class="content">
  {#if offline}<div class="notice" role="status">MediaBridge is offline. Playback controls are unavailable while reconnecting.</div>{/if}
  {#if message || status.playback_error}<div class="notice" role="status">{message || status.playback_error}<button class="icon" aria-label="Dismiss" disabled={!message} onclick={() => message = ''}><Icon name="close"/></button></div>{/if}
  {#if ['recommendations','subscriptions','twitch-live','sunshine','history','youtube-history'].includes(currentTab) && !channelId}
    <a class="back-home" href="/"><Icon name="left" size={13}/> Back to Home</a>
  {/if}
  {#if currentTab === 'simple-home'}
    <section class="panel dock-queue simple-queue" aria-labelledby="simple-playback-title">
      <h1 id="simple-playback-title">Playback</h1>
      {@render playbackQueue()}
    </section>
  {:else if currentTab === 'simple-sunshine'}
    {@render sunshineControls()}
  {:else if currentTab === 'search'}
    <section aria-labelledby="search-heading">
      <div class="section-head"><h1 id="search-heading">YouTube search</h1></div>
      <form class="panel link-form" onsubmit={search}>
        <input type="search" aria-label="Search YouTube" placeholder="Search YouTube" required maxlength="120" bind:value={searchTerm}>
        <button class="filled" disabled={searching || !searchTerm.trim()}><Icon name="search" size={18}/> Search</button>
      </form>
      {#if searching}<p class="placeholder" role="status">Searching YouTube…</p>
      {:else if searchError}<p class="placeholder" role="status">{searchError}</p>
      {:else if searched && !searchResults.length}<p class="placeholder">No videos found.</p>
      {:else if searchResults.length}<MediaGrid items={searchResults} busy={busy || offline} play={item => play(item)} {queue}/>{/if}
    </section>
  {:else if currentTab === 'home'}
    <section class="panel home-card" aria-labelledby="home-youtube-title">
      <div class="section-head"><Icon name="watch"/><h2 id="home-youtube-title">Recommended from YouTube</h2><button class="icon" aria-label="Refresh Home recommendations" title="Refresh Home recommendations" disabled={offline || refreshingFull || feedLoading} onclick={refreshDashboardHome}><Icon name="refresh" size={17}/></button><a class="see-all" href="/?view=recommendations">See all <Icon name="right" size={15}/></a></div>
      {#if feedLoading && !shown.length}<p class="muted" role="status">Loading recommendations…</p>
      {:else if feedError && !shown.length}<p class="muted" role="status">{feedError}</p>
      {:else if shown.length}<MediaGrid items={shown} preview previewLimit={5} busy={busy || offline} play={item => play(item)} {queue}/>
      {:else}<p class="muted">No recommendations available.</p>{/if}
    </section>
    <div class="home-split">
      <section class="panel home-card" aria-labelledby="home-saved-title">
        <div class="section-head"><Icon name="bookmark"/><h2 id="home-saved-title">YouTube subscriptions</h2><a class="see-all" href="/?view=subscriptions">See all <Icon name="right" size={15}/></a></div>
        {#if homeChannels.length}
          <div class="channel-bubbles">
            {#each rankChannels(homeChannels,'youtube','recent').slice(0,5) as channel (channel.id)}
              <a href={channelSource(channel.id,'youtube') ?? channelLink(channel.id,'youtube')} title={channel.title}
                onclick={event => openChannel(event,channel.id,'youtube')}>
                <ChannelAvatar url={channel.thumbnail_url} name={channel.title} size={52}/>
                <small>{channel.title}</small>
              </a>
            {/each}
          </div>
        {:else}<p class="muted">{homeChannelsLoading ? 'Loading subscriptions…' : homeChannelsError || 'No subscribed channels were returned.'}</p>{/if}
      </section>
      <section class="panel home-card" aria-labelledby="home-sunshine-title">
        <div class="section-head"><Icon name="sun"/><h2 id="home-sunshine-title">Sunshine</h2><a class="see-all" href="/?view=subscriptions&amp;provider=sunshine">See all <Icon name="right" size={15}/></a></div>
        {#each servers.slice(0,3) as server (`${server.host}/${server.app}`)}
          <div class="home-server"><Icon name="bookmark" size={16}/><span title={server.host}>{server.host} · {server.app}</span><button class="tonal" disabled={offline || busy} onclick={() => command('/sunshine',server,'Moonlight launch requested.')}>Connect</button></div>
        {:else}<p class="muted">Bookmark a Sunshine server to connect from here.</p>{/each}
      </section>
    </div>
    <section class="panel home-card twitch-card" aria-labelledby="home-twitch-title">
      <div class="section-head"><Icon name="subscriptions"/><h2 id="home-twitch-title">Twitch channels live</h2><a class="see-all" href="/?view=twitch-live">See all <Icon name="right" size={15}/></a></div>
      {#if twitchLoading}<p class="muted" role="status">Checking followed channels…</p>
      {:else if twitchError}<p class="muted" role="status">{twitchError}</p>
      {:else if twitchLive.length}<MediaGrid items={twitchLive} preview previewLimit={5} busy={busy || offline} play={item => play(item)} {queue}/>
      {:else}<p class="muted">No followed channels are live.</p>{/if}
    </section>
  {:else if currentTab === 'twitch-live'}
    <section class="panel home-card" aria-labelledby="twitch-live-title">
      <div class="section-head"><h2 id="twitch-live-title">Twitch channels live</h2><a class="see-all" href="/?view=subscriptions&provider=twitch">Followed channels <Icon name="right" size={15}/></a></div>
      {#if twitchLoading}<p class="muted" role="status">Checking followed channels…</p>
      {:else if twitchError}<p class="muted" role="status">{twitchError}</p>
      {:else if twitchLive.length}<MediaGrid items={twitchLive} busy={busy || offline} play={item => play(item)} {queue}/>
      {:else}<p class="muted">No followed channels are live.</p>{/if}
    </section>
  {:else if currentTab === 'subscriptions'}
    <section aria-labelledby="subscriptions-title">
      <div class="section-head"><h2 id="subscriptions-title">Subscriptions</h2>
      </div>
      <nav class="library-tabs" aria-label="Subscription categories">
        <a class="library-tab" aria-current={libraryTab === 'youtube' ? 'page' : undefined} href="/?view=subscriptions">YouTube subscriptions</a>
        <a class="library-tab" aria-current={libraryTab === 'twitch' ? 'page' : undefined} href="/?view=subscriptions&amp;provider=twitch">Twitch subscriptions</a>
        <a class="library-tab" aria-current={libraryTab === 'sunshine' ? 'page' : undefined} href="/?view=subscriptions&amp;provider=sunshine">Saved Sunshine servers</a>
      </nav>
      {#if libraryTab === 'sunshine'}
        {@render sunshineControls()}
      {:else}
      {#if channelId}
        <a class="back-home" href={directoryLink(provider)}><Icon name="left" size={13}/> Back to Subscriptions</a>
        <div class="channel-title">
          {#if selected}
            {@const channelUrl = channelSource(selected.id,provider)}
            {#if channelUrl}<a href={channelUrl} rel="external noopener" aria-label={'Open ' + selected.title + ' channel'}><ChannelAvatar url={selected.thumbnail_url} name={selected.title} size={48}/></a>
            {:else}<ChannelAvatar url={selected.thumbnail_url} name={selected.title} size={48}/>{/if}
          {/if}
          <h2>{selected?.title || channelVideos[0]?.uploader || 'Channel videos'}</h2>
        </div>
        {#if subscriptionError}<div class="placeholder panel" role="status">{subscriptionError}</div>
        {:else if videosLoading}<div class="placeholder panel" role="status">Loading channel videos…</div>
        {:else if channelVideos.length}<MediaGrid items={channelVideos} channelUrl={channelSource(channelId,provider)} busy={busy || offline} play={item => play(item)} {queue}/>
        {:else}<div class="placeholder panel">No recent videos are available.</div>{/if}
      {:else}
        <div class="channel-directory-controls">
          <input type="search" aria-label="Filter channels" placeholder="Find a channel" bind:value={channelFilter}>
          <select aria-label="Sort channels" bind:value={channelOrder}><option value="recent">Latest available upload</option><option value="name">Channel name</option></select>
        </div>
        {#if subscriptionError}<div class="placeholder panel" role="status">{subscriptionError}</div>
        {:else if subscriptionsLoading}<div class="placeholder panel" role="status">Loading channels…</div>
        {:else if !filteredChannels.length}<div class="placeholder panel">{channelFilter ? 'No channels match your filter.' : 'No channels were returned.'}</div>
        {:else}
          <div class="channel-directory">
            {#each filteredChannels as channel (channel.id)}
              <a class="panel channel-tile" href={channelSource(channel.id,provider) ?? channelLink(channel.id,provider)}
                onclick={event => openChannel(event,channel.id,provider)}>
                <ChannelAvatar url={channel.thumbnail_url} name={channel.title} size={56}/>
                <strong>{channel.title}</strong>
              </a>
            {/each}
          </div>
        {/if}
      {/if}
      {/if}
    </section>
  {:else}
    <section aria-labelledby="feed-title">
      {#if currentTab === 'history' || currentTab === 'youtube-history'}
        <h1>History</h1>
        <nav class="library-tabs" aria-label="History source">
          <a class="library-tab" aria-current={currentTab === 'history' ? 'page' : undefined} href="/?view=history">MediaBridge history</a>
          <a class="library-tab" aria-current={currentTab === 'youtube-history' ? 'page' : undefined} href="/?view=youtube-history">YouTube history</a>
        </nav>
      {/if}
      <div class="section-head">
        <h2 id="feed-title">{currentTab === 'queue' ? 'Up next' : currentTab === 'youtube-history' ? 'YouTube History' : currentTab === 'history' ? 'Playback history' : source === 'home' ? 'YouTube Home' : 'Related videos'}</h2>
        <input class="filter" aria-label="Filter videos" placeholder="Filter this list" bind:value={query}>
        {#if currentTab === 'recommendations'}
          {#if source === 'home' && feed?.long_enabled}<button class="tonal" disabled={offline || refreshingFull} onclick={() => refreshFullFeed()}>{refreshingFull ? 'Refreshing…' : 'Refresh full feed'}</button>{/if}
          <select aria-label="Recommendation source" bind:value={feedMode}><option value="auto">Automatic</option><option value="home">YouTube Home</option></select>
        {/if}
        {#if currentTab === 'recommendations' || currentTab === 'youtube-history'}
          <button class="icon" aria-label={currentTab === 'recommendations' ? 'Refresh recommendations' : 'Refresh history'} title="Refresh feed" disabled={offline || feedLoading} onclick={() => {cache.delete(source); reload++;}}><Icon name="refresh"/></button>
        {:else if currentTab === 'queue'}
          <button class="tonal" disabled={busy || offline} onclick={() => command('/autoplay',{enabled:!status.autoplay_enabled})}>Autoplay {status.autoplay_enabled ? 'on' : 'off'}</button>
        {/if}
      </div>
      {#if loading || ['home','recommendations','youtube-history'].includes(currentTab) && feedLoading && !shown.length}<div class="placeholder panel" role="status">Loading {currentTab === 'recommendations' ? 'recommendations' : 'media'}…</div>
      {:else if ['home','recommendations','youtube-history'].includes(currentTab) && feedError && !shown.length}<div class="placeholder panel" role="status">{feedError} <button class="tonal" disabled={offline} onclick={() => {cache.delete(source); reload++;}}>Retry</button></div>
      {:else if !shown.length}<div class="placeholder panel">{query ? 'No videos match this filter.' : currentTab === 'queue' ? 'The queue is empty. Add a link or a recommendation.' : currentTab === 'history' ? 'Played videos will appear here.' : currentTab === 'youtube-history' ? 'No recent videos were found in your YouTube history.' : 'No recommendations are available. Sign in on the MediaBridge host for a personalized feed.'}</div>
      {:else}<MediaGrid bind:this={grid} items={shown} paginated={currentTab !== 'recommendations'} busy={busy || offline} play={item => play(item,currentTab === 'history')} {queue}/>{/if}
      {#if shown.length && feedError}<p class="muted" role="status">{feedError}</p>{/if}
      {#if currentTab === 'recommendations' && source === 'home' && feed && shown.length && !hasMoreHome && !feedLoading && !loadingMore && !query.trim() && !feedError}
        <p class="muted feed-end" role="status">
          {feed.can_extend === undefined && feed.long_enabled
            ? 'End of the cached feed. Install the server build with feed extension to load more here.'
            : feed.long_enabled === false
              ? 'End of the cached feed. Background Home refresh is off.'
              : 'No more unique videos are available right now.'}
        </p>
      {/if}
      {#if currentTab === 'recommendations' && source === 'home' && hasMoreHome}
        <div class="home-sentinel" bind:this={nextPage}>
          {#if loadingMore}<small role="status">Loading more videos…</small>{/if}
        </div>
      {/if}
    </section>
  {/if}
</div>
{/if}
