<script lang="ts">
  let {url, name, size = 22}: {url?:string | null; name:string; size?:number} = $props();
  let failed = $state<string | null>(null);
  let supported = $derived(url?.startsWith('https://') ? url : null);
  let source = $derived(supported ? '/api/artwork/channel?url=' + encodeURIComponent(supported) : null);
</script>

<span class="channel-avatar" style:--avatar-size={size + 'px'} aria-hidden="true">
  {#if source && failed !== source}
    <img src={source} alt="" width={size} height={size} loading="lazy" decoding="async" onerror={() => failed = source}>
  {:else}
    {name.trim().slice(0,2).toUpperCase()}
  {/if}
</span>
