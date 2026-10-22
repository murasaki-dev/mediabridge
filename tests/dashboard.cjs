const fs = require('node:fs');
const vm = require('node:vm');
const assert = require('node:assert/strict');
const html = fs.readFileSync(require('node:path').join(__dirname, '../web/index.html'), 'utf8');
class Element {
  constructor() {
    this.value = ''; this.children = []; this.dataset = {}; this.style = {}; this.options = [];
    const classes = new Set();
    this.classList = {add: x => classes.add(x), remove: x => classes.delete(x), contains: x => classes.has(x), toggle(x, force) {
      const on = force === undefined ? !classes.has(x) : force;
      if (on) classes.add(x); else classes.delete(x); return on;
    }};
    this.parentElement = {classList:this.classList}; this.listeners = {};
  }
  append(...items) { this.children.push(...items); }
  replaceChildren(...items) { this.children = items; }
  setAttribute(name, value) { this[name] = value; }
  removeAttribute(name) { delete this[name]; }
  addEventListener(name, handler) { this.listeners[name] = handler; }
  dispatchEvent(event) { this.listeners[event.type]?.(event); }
  querySelector() { return null; }
}
const nodes = new Map([...html.matchAll(/\bid="([^"]+)"/g)].map(match => [match[1], new Element()]));
const node = id => { assert(nodes.has(id), `Unknown dashboard ID ${id}`); return nodes.get(id); };
node('mode').value = 'local'; node('quality').value = '720'; node('browser').value = 'librewolf';
const posts = [];
let state = {queue_items:[], queued:[], history_items:[], can_seek:true, hardware_decoder:'software', autoplay_enabled:true,
  autoplay_loading:true, autoplay_status:'Checking current video', autoplay_error:'Lookup failed'};
const context = vm.createContext({document:{getElementById:node, createElement:()=>new Element(), createTextNode:text=>text,
  querySelectorAll:()=>[], addEventListener(){}, body:new Element()},
  localStorage:{getItem:()=>null,setItem(){}}, window:{location:{href:'http://localhost/'},setTimeout(){}},
  URL, Event, console, setInterval(){}, setTimeout(){}, clearTimeout(){},
  fetch:async (path, options={}) => {
    if (options.method === 'POST') posts.push({path, body:options.body ? JSON.parse(options.body) : null});
    let data = {};
    if (path === '/status') data = state;
    if (path === '/platform') data = {os:'windows'};
    if (path === '/preferences' || path === '/settings/preferences') data = {default_quality:'720', default_video_codec:'h264',cookies_browser:'off'};
    if (path === '/settings/remote-origins') data = {remote_origin_servers:[]};
    if (path === '/queue_batch') data = {added:JSON.parse(options.body).items.length};
    return {ok:true, json:async()=>data};
  }});
vm.runInContext(html.match(/<script>([\s\S]*?)<\/script>/)[1], context);
async function run() {
  vm.runInContext('renderFlatpakAccess({flatpak:true,steam_host_access_configured:false})',context);
  assert.equal(node('flatpak-host-settings').classList.contains('hidden'),false);
  assert.match(node('flatpak-host-status').textContent,/not configured/);
  vm.runInContext('renderFlatpakAccess({flatpak:true,steam_host_access_configured:true})',context);
  assert.match(node('flatpak-host-status').textContent,/is configured/);
  await node('flatpak-host-recheck').onclick();
  assert.equal(node('flatpak-host-settings').classList.contains('hidden'),true);
  // Let startup fetches finish, then test real dashboard event handlers.
  await new Promise(resolve => setImmediate(resolve));
  await vm.runInContext('refresh()', context);
  assert.equal(node('play-toggle').disabled,true);
  state = {...state,current:'https://example.com/current',playback_state:'playing'};
  await vm.runInContext('refresh()',context);
  assert.equal(node('play-toggle').dataset.control,'pause');
  assert.equal(node('play-toggle')['aria-label'],'Pause');
  state = {...state,playback_state:'paused'};
  await vm.runInContext('refresh()',context);
  assert.equal(node('play-toggle').dataset.control,'resume');
  assert.equal(node('play-toggle')['aria-label'],'Play');
  assert.equal(node('hardware-decoder').textContent, 'Video decoder: software');
  assert.match(node('autoplay-state').textContent, /Loading Mix/);
  assert.equal(node('autoplay-error').textContent, 'Lookup failed');
  node('url').value = 'https://example.com/new.mp4'; node('submitter-id').value = 'caller';
  await node('play-interrupt').onclick();
  assert(posts.some(post => post.path === '/play_interrupt' && post.body.queued_by_user_id === 'caller'));
  node('url').value = 'https://example.com/tail.mp4';
  await node('append-tail').onclick();
  assert(posts.some(post => post.path === '/queue' && post.body.at_end === true));
  node('batch-urls').value = 'https://example.com/a.mp4\nhttps://example.com/b.mp4';
  await node('batch-next').onclick();
  assert.deepEqual(posts.findLast(post => post.path === '/queue_batch').body.items.map(item => item.url), ['https://example.com/a.mp4','https://example.com/b.mp4']);
  state = {...state, history_items:[{url:'https://example.com/old',local:false,browser:'librewolf',subtitles:true,duration_seconds:65}],
    queue_items:[{url:'https://example.com/queued', duration_seconds:null, start_position_seconds:12, metadata_error:'Metadata unavailable',queued_by_user_id:'caller'}], queued:['https://example.com/queued']};
  await vm.runInContext('refresh()', context);
  assert.match(vm.runInContext('describeItem(historyItems[0])', context), /1:05/);
  assert.match(vm.runInContext('describeItem({url:"x",start_position_seconds:12})', context), /Resume at 0:12/);
  node('history-select-all').onclick();
  await node('history-last').onclick();
  const replay = posts.findLast(post => post.path === '/queue_batch').body;
  assert.equal(replay.mode,'last'); assert.equal(replay.items[0].browser,'librewolf'); assert.equal(replay.items[0].subtitles,true);
  assert.equal(node('history-selection-count').textContent, '0 selected');
  node('smb-share-id').value = 'invalid';
  assert.throws(()=>vm.runInContext('applySmbReference({url:"smb://host/share/file"})',context), /24 hexadecimal/);
  assert.equal(vm.runInContext('applySmbReference({url:"https://example.com/a"}).smb_share_id',context),undefined);
  await node('clear-autoplay-error').onclick();
  assert(posts.some(post => post.path === '/autoplay/error/clear'));
  node('close-minutes').value = '12.5'; await node('set-close-timer').onclick();
  assert(posts.some(post => post.path === '/timer' && post.body.minutes === 12.5));
  await node('cancel-close-timer').onclick();
  assert(posts.some(post => post.path === '/timer' && post.body.minutes === null));
  state = {...state, close_remaining_seconds:125, close_timer_error:null};
  await vm.runInContext('refresh()', context);
  assert.match(node('close-countdown').textContent,/2:05/);
  state = {...state, tracks_available:true, playback_epoch:42,
    subtitle_tracks:[{id:3,language:'eng',title:'English',selected:true}],
    audio_tracks:[{id:8,language:'spa',title:'Spanish',selected:true}]};
  await vm.runInContext('refresh()',context);
  assert.equal(node('subtitle-track').value,'3'); assert.equal(node('audio-track').value,'8');
  node('remember-tracks').checked=false; node('subtitle-track').value='off';
  await node('subtitle-track').onchange();
  assert(posts.some(post => post.path==='/subtitles/select' && post.body.id===null && post.body.playback_epoch===42 && post.body.remember===false));
  node('audio-track').value='8'; await node('audio-track').onchange();
  assert(posts.some(post => post.path==='/audio/select' && post.body.id===8));
  node('multiple-audio').checked=true;
  assert.equal(vm.runInContext('itemFromForm("https://example.com/multi").load_multiple_audio_tracks',context),true);
  console.log('Dashboard status rendering, interruption, batches, history replay, attribution, and provider validation passed.');
}
run().catch(error => { console.error(error); process.exitCode = 1; });
