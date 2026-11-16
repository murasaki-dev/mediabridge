<script lang="ts">
  import {page} from '$app/state';
  import {liveChat, themedChatEmbed} from '$lib/live-chat';
  import {api} from '$lib/api';
  import {getDashboardContext} from '$lib/playback-context';

  const {playback:getPlayback,colorScheme} = getDashboardContext();
  let current = $derived(getPlayback()?.current);
  let online = $derived(getPlayback() !== null);
  let starting = $derived(getPlayback() === undefined);
  let playing = $derived(['playing','paused'].includes(getPlayback()?.playback_state?.toLowerCase() || ''));
  let reportedLive = $derived(getPlayback()?.is_live);
  let candidate = $derived(current ? liveChat(current,page.url.hostname) : undefined);
  let confirmed = $state<string>();
  let checking = $state(false), error = $state('');
  let chat = $derived(candidate && (reportedLive === true ||
    reportedLive !== false && (candidate.provider === 'Twitch' ? playing : confirmed === candidate.id))
    ? candidate : undefined);
  let embed = $derived(chat ? themedChatEmbed(chat,colorScheme()) : undefined);
  $effect(() => {
    const item = candidate;
    confirmed = undefined; error = ''; checking = false;
    if (item?.provider !== 'YouTube' || reportedLive !== undefined) return;
    const controller = new AbortController();
    checking = true;
    api<{live:boolean}>('/chat/youtube-live?id=' + encodeURIComponent(item.id),undefined,controller.signal)
      .then(result => { if (!controller.signal.aborted && result.live) confirmed = item.id; })
      .catch(() => { if (!controller.signal.aborted) error = 'Could not check live chat for the current YouTube video.'; })
      .finally(() => { if (!controller.signal.aborted) checking = false; });
    return () => controller.abort();
  });
</script>
<svelte:head><title>Live chat · MediaBridge</title></svelte:head>
<div class="content appearance-page">
  <h1>Live chat</h1>
  {#if chat}
    <section class="panel chat-panel" aria-label={chat.provider + ' live chat'}>
      <div class="section-head"><h2>{chat.provider} chat</h2>
        <a class="tonal button" href={chat.direct} target="_blank" rel="noopener">Open on {chat.provider}</a>
      </div>
      {#key embed}<iframe class="chat-frame" title={chat.provider + ' live chat'} src={embed} style:color-scheme={colorScheme()} referrerpolicy="strict-origin-when-cross-origin"></iframe>{/key}
    </section>
  {:else}
    <p class="panel placeholder" role="status">{starting ? 'Checking host playback…' : !online
      ? 'MediaBridge is offline.' : checking ? 'Checking YouTube for live chat…'
      : error || 'Play a YouTube or Twitch livestream with chat enabled to see it here.'}</p>
  {/if}
</div>
<style>
  .chat-panel {padding:12px}
  .chat-frame {display:block;width:100%;height:clamp(350px,63dvh,730px);border:0;border-radius:var(--r-md);
    background:transparent;opacity:clamp(.7,var(--panel-opacity,1),1)}
</style>
