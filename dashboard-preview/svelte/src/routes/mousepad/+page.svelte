<script lang="ts">
  import {onMount} from 'svelte';
  import Icon from '$lib/Icon.svelte';
  import {api} from '$lib/api';

  type MouseSettings = {backend?:string; enabled?:boolean; pending?:boolean; error?:string};
  type MouseButton = 'left' | 'middle' | 'right';
  type Input = {action:'move_relative'; dx:number; dy:number}
    | {action:'click_current'; button:MouseButton};

  let settings = $state<MouseSettings>();
  let checking = $state(true), requesting = $state(false), active = $state(false);
  let setupError = $state(''), inputError = $state('');
  let ready = $derived(!checking && (settings?.backend !== 'portal' || settings.enabled === true));
  const controller = new AbortController();
  let timer: ReturnType<typeof setTimeout>;
  let frame = 0, sending = false;
  const inputs: Input[] = [];
  let contact: {id:number; x:number; y:number; distance:number} | undefined;

  async function readSettings(enable = false) {
    clearTimeout(timer);
    try {
      settings = await api<MouseSettings>('/settings/mouse',enable ? {enabled:true} : undefined,controller.signal);
      setupError = settings.error || '';
      if (settings.pending && !controller.signal.aborted) timer = setTimeout(readSettings,1200);
    } catch (error) {
      if (!controller.signal.aborted) setupError = error instanceof Error ? error.message : 'Could not check host mouse controls.';
    } finally { checking = false; requesting = false; }
  }
  onMount(() => {
    void readSettings();
    return () => { controller.abort(); clearTimeout(timer); cancelAnimationFrame(frame); inputs.length = 0; };
  });

  async function sendInputs() {
    if (sending || controller.signal.aborted) return;
    sending = true;
    try {
      let input: Input | undefined;
      while (!controller.signal.aborted && (input = inputs.shift())) {
        await api('/mouse',input,controller.signal);
      }
      inputError = '';
    } catch (error) {
      inputs.length = 0;
      if (!controller.signal.aborted) inputError = error instanceof Error ? error.message : 'Could not send mouse input.';
    } finally { sending = false; }
  }
  function move(dx:number,dy:number) {
    if (!ready || (!dx && !dy)) return;
    const last = inputs.at(-1);
    const clamp = (value:number) => Math.max(-32767,Math.min(32767,value));
    if (last?.action === 'move_relative') {
      last.dx = clamp(last.dx + dx);
      last.dy = clamp(last.dy + dy);
    } else inputs.push({action:'move_relative',dx:clamp(dx),dy:clamp(dy)});
    if (!frame) frame = requestAnimationFrame(() => { frame = 0; void sendInputs(); });
  }
  function click(button:MouseButton) {
    if (!ready) return;
    inputs.push({action:'click_current',button});
    void sendInputs();
  }
  function start(event:PointerEvent & {currentTarget:HTMLButtonElement}) {
    if (!ready || event.button !== 0 || contact) return;
    event.currentTarget.setPointerCapture(event.pointerId);
    contact = {id:event.pointerId,x:event.clientX,y:event.clientY,distance:0};
    active = true;
  }
  function drag(event:PointerEvent) {
    if (contact?.id !== event.pointerId) return;
    const dx = event.clientX - contact.x, dy = event.clientY - contact.y;
    const distance = Math.hypot(dx,dy);
    if (contact.distance >= 4 || distance >= 4) {
      move(Math.round(dx * 1.8),Math.round(dy * 1.8));
      contact.x = event.clientX;
      contact.y = event.clientY;
      contact.distance = 4;
    } else {
      contact.distance = distance;
    }
  }
  function end(event:PointerEvent, cancelled = false) {
    if (contact?.id !== event.pointerId) return;
    const tapped = contact.distance < 4;
    contact = undefined; active = false;
    if (!cancelled && tapped) click('left');
  }
  function arrowMove(event:KeyboardEvent) {
    const directions: Record<string,[number,number]> = {ArrowLeft:[-12,0],ArrowRight:[12,0],ArrowUp:[0,-12],ArrowDown:[0,12]};
    if (!(event.key in directions)) return;
    event.preventDefault();
    move(...directions[event.key]);
  }
</script>

<svelte:head><title>Mousepad · MediaBridge</title></svelte:head>
<div class="content appearance-page">
  <h1>Mousepad</h1>
  <section class="panel mouse-controls" aria-label="Host mouse controls">
    {#if settings?.backend === 'portal' && !settings.enabled}
      <div class="mouse-permission">
        <p>{settings.pending ? 'Approve desktop control on the MediaBridge host.' : 'Desktop control needs approval on the MediaBridge host.'}</p>
        <button class="tonal" disabled={requesting || settings.pending} onclick={() => { requesting = true; void readSettings(true); }}>Enable desktop control</button>
      </div>
    {/if}
    <button type="button" class="mousepad" class:active disabled={!ready} aria-label="Host touchpad" aria-describedby="mousepad-instructions"
      onpointerdown={start} onpointermove={drag} onpointerup={event => end(event)}
      onpointercancel={event => end(event,true)} onlostpointercapture={event => end(event,true)}
      onkeydown={arrowMove} onclick={event => { if (event.detail === 0) click('left'); }}>
      <Icon name="mouse" size={32}/>
      <span id="mousepad-instructions">{checking ? 'Checking host mouse controls…' : 'Drag to move · Tap to click'}</span>
    </button>
    <div class="mouse-buttons">
      <button class="tonal" disabled={!ready} onclick={() => click('left')}>Left click</button>
      <button class="tonal" disabled={!ready} onclick={() => click('middle')}>Middle click</button>
      <button class="tonal" disabled={!ready} onclick={() => click('right')}>Right click</button>
    </div>
    {#if setupError || inputError}<p class="muted" role="status">{inputError || setupError}</p>{/if}
  </section>
</div>

<style>
  .mouse-controls {display:grid;gap:12px;padding:14px}
  .mouse-permission {display:flex;align-items:center;justify-content:space-between;gap:10px;flex-wrap:wrap}
  .mousepad {display:grid;place-content:center;justify-items:center;gap:9px;min-height:clamp(180px,44dvh,420px);
    border:1px dashed var(--md-sys-color-outline-variant);border-radius:var(--r-md);
    background:color-mix(in srgb,var(--md-sys-color-surface-container-lowest) calc(var(--panel-opacity) * 100%),transparent);
    color:var(--md-sys-color-on-surface-variant);touch-action:none;user-select:none;cursor:crosshair}
  .mousepad.active {border-style:solid;border-color:var(--md-sys-color-primary);color:var(--md-sys-color-primary)}
  .mouse-buttons {display:grid;grid-template-columns:repeat(3,minmax(0,1fr));gap:8px}
  .mouse-buttons button {padding-inline:5px}
</style>
