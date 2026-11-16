<script lang="ts">
  let {name, size = 20}: {name: string; size?: number} = $props();
  const paths: Record<string,string> = {
    home:'M3 10 12 3l9 7v10h-6v-6H9v6H3z', queue:'M3 5h18M3 12h18M3 19h12',
    watch:'M3 4h18v14H3zM8 22h8m-6-14 5 3-5 3z', settings:'M4 6h16M4 12h16M4 18h16M8 3v6m8 0v6m-5 0v6',
    mouse:'M12 3a6 6 0 0 0-6 6v6a6 6 0 0 0 12 0V9a6 6 0 0 0-6-6zm0 3v4',
    search:'M15.5 15.5 21 21M17 10a7 7 0 1 1-14 0 7 7 0 0 1 14 0z',
    chat:'M4 4h16v12H9l-5 4zM8 9h8M8 12h5',
    play:'m8 4 13 8-13 8z', pause:'M9 5v14m6-14v14', next:'m5 5 11 7-11 7zm14 0v14',
    plus:'M12 5v14M5 12h14', refresh:'M20 7v5h-5M4 17v-5h5M5 8a8 8 0 0 1 13-2l2 4M4 14l2 4a8 8 0 0 0 13-2',
    stop:'M6 6h12v12H6z', bookmark:'M6 3h12v18l-6-4-6 4z', link:'M10 14l4-4M8 16l-2 2a3 3 0 0 1-4-4l4-4m10-2 2-2a3 3 0 0 1 4 4l-4 4',
    previous:'m18 5-11 7 11 7zM4 5v14', up:'m6 15 6-6 6 6', down:'m6 9 6 6 6-6', volume:'M4 9h4l5-4v14l-5-4H4zm12 0a4 4 0 0 1 0 6m2-8a7 7 0 0 1 0 10',
    subscriptions:'M4 8h16v12H4zM7 4h10m-7 7 5 3-5 3z', right:'m9 6 6 6-6 6', left:'m15 6-6 6 6 6',
    history:'M4 11a8 8 0 1 1 2 6m-2-2v-4h4m4-4v5l3 2',
    close:'m6 6 12 12M18 6 6 18', sun:'M12 3v3m0 12v3M3 12h3m12 0h3M6 6l2 2m8 8 2 2m0-12-2 2m-8 8-2 2M9 12a3 3 0 1 0 6 0 3 3 0 1 0-6 0',
  };
</script>
<svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d={paths[name] || paths.play}/></svg>
