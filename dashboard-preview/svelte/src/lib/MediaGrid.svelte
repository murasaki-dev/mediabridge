<script lang="ts">
  import {artwork, webMediaUrl} from './api';
  import {onDestroy} from 'svelte';
  import type {Media} from './api';
  import {clock, visibleRange} from './logic';
  import Icon from './Icon.svelte';
  import ChannelAvatar from './ChannelAvatar.svelte';
  let {items, play, queue, busy, channelUrl, preview = false, previewLimit = 3, paginated = true}: {paginated?:boolean; previewLimit?:number; preview?:boolean; items: Media[]; play: (item: Media) => void; queue: (item: Media) => void; busy: boolean; channelUrl?:string} = $props();
  let width = $state(0), height = $state(800), scroll = $state(0), pages = $state(1);
  let top = $state(0);
  let viewport = $state<HTMLDivElement>();
  let columns = $derived(Math.max(1, Math.min(preview ? previewLimit : Infinity, Math.floor(width / 205))));
  let count = $derived(Math.min(items.length, preview ? previewLimit : paginated ? pages * 24 : items.length));
  const row = 224;
  let range = $derived(preview ? {start:0,end:count,offset:0} : visibleRange(count, columns, row, Math.max(0,-top), Math.max(0,height - Math.max(0,top))));
  $effect(() => {
    void scroll; void width; void height;
    if (viewport) top = viewport.getBoundingClientRect().top;
  });
  let revealed = $state<string | null>(null);
  let hideTimer: ReturnType<typeof setTimeout>;
  function reveal(url: string) {
    clearTimeout(hideTimer);
    revealed = url;
    hideTimer = setTimeout(() => revealed = null, 1000);
  }
  function selectCard(event: MouseEvent, url: string) {
    if (event.button !== 0 || !event.detail || event.ctrlKey || event.metaKey || event.shiftKey || event.altKey) return;
    event.preventDefault();
    if (matchMedia('(hover: none)').matches) reveal(url);
  }
  function uploaderLink(url: string) {
    if (channelUrl) return channelUrl;
    const parsed = webMediaUrl(url);
    return parsed && /^https?:\/\/(?:www\.)?twitch\.tv\/[A-Za-z0-9_]+\/?$/i.test(parsed) ? parsed : undefined;
  }
  onDestroy(() => clearTimeout(hideTimer));
  function published(value?: string) {
    if (!value) return '';
    if (!/^\d{4}-\d{2}-\d{2}/.test(value)) return value;
    const days = Math.max(0, Math.floor((Date.now() - Date.parse(value)) / 86400000));
    if (!Number.isFinite(days)) return '';
    return days === 0 ? 'Today' : days < 7 ? days + 'd ago' : days < 30 ? Math.floor(days / 7) + 'w ago' : Math.floor(days / 30) + 'mo ago';
  }
  export function reset() { pages = 1; }
</script>
<svelte:window bind:scrollY={scroll} bind:innerHeight={height}/>
<div class="grid-window" class:preview bind:this={viewport} bind:clientWidth={width}>
  <div class="grid-space" style:height={`${Math.ceil(count / columns) * row}px`}>
    <div class="grid-cards" style:grid-template-columns={`repeat(${columns}, minmax(0,1fr))`} style:transform={`translateY(${range.offset}px)`}>
      {#each items.slice(range.start, range.end) as item, index (range.start + index)}
        {@const sourceUrl = webMediaUrl(item.url)}
        {@const bylineUrl = uploaderLink(item.url)}
        <article class="media-card" class:revealed={revealed === item.url}>
          <div class="poster">
            {#if artwork(item)}<img src={artwork(item)} srcset={`${artwork(item)} 320w, ${artwork(item,'hqdefault')} 480w`} sizes="(max-width:600px) 48vw, 280px" alt="" loading="lazy" decoding="async">
            {:else}<Icon name="watch" size={30}/>{/if}
            {#if item.duration_seconds}<span>{clock(item.duration_seconds)}</span>{/if}
            {#if !sourceUrl}<button class="thumbnail-reveal" aria-label={'Show actions for ' + (item.title || item.url)} onclick={() => reveal(item.url)}></button>{/if}
            <div class="thumbnail-actions">
              <button class="icon thumbnail-play" disabled={busy} aria-label={'Play ' + (item.title || item.url)} title="Play" onclick={() => {revealed = null; play(item);}}><Icon name="play" size={19}/></button>
              <button class="icon" disabled={busy} aria-label={'Queue ' + (item.title || item.url)} title="Add to queue" onclick={() => {revealed = null; queue(item);}}><Icon name="plus" size={19}/></button>
            </div>
          </div>
          <strong title={item.title || item.url}>{#if sourceUrl}<a class="media-source-link" href={sourceUrl} rel="external noopener" onclick={event => selectCard(event,item.url)}>{item.title || item.url}</a>{:else}{item.title || item.url}{/if}</strong>
          {#if item.uploader || item.published}<small class="card-byline">{#if item.uploader}
            {#if bylineUrl}<a class="card-channel-link" href={bylineUrl} rel="external noopener" aria-label={'Open ' + item.uploader + ' channel'}><ChannelAvatar url={item.avatar_url} name={item.uploader}/></a>
            {:else}<ChannelAvatar url={item.avatar_url} name={item.uploader}/>{/if}
          {/if}{[item.uploader, published(item.published)].filter(Boolean).join(' · ')}</small>{/if}
        </article>
      {/each}
    </div>
  </div>
</div>
{#if !preview && paginated && count < items.length}<button class="tonal" onclick={() => pages++}>Show 24 more</button>{/if}
