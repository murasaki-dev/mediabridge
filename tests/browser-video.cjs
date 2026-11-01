const fs = require('node:fs');
const vm = require('node:vm');
const assert = require('node:assert/strict');
const path = require('node:path');
const source = fs.readFileSync(path.join(__dirname, '../main.rs'), 'utf8');
const script = source.match(/const STATE_SCRIPT: &str = r#"([\s\S]*?)"#;/)[1];

function readState(fullscreen, playerFullscreen = false, theaterActive = false, chatCollapsed = false) {
  const listeners = {};
  const video = {currentTime: 2, duration: 2, paused: true, ended: true,
    addEventListener: (name, callback) => { listeners[name] = callback; }};
  const player = {classList: {contains: () => playerFullscreen}};
  const button = label => ({getAttribute: name => name === 'aria-label' ? label : null,
    getBoundingClientRect: () => ({x:0,y:0,width:30,height:30})});
  const theater = button(theaterActive ? 'Exit Theatre Mode' : 'Theatre Mode');
  const chat = button(chatCollapsed ? 'Expand Chat' : 'Collapse Chat');
  const context = {window: {}, location: {href: 'https://example.com/video'},
    document: {title: 'Video', fullscreenElement: fullscreen?.(video),
      querySelector: selector => selector === 'video' ? video : selector.startsWith('.html5') ? player :
        selector.includes('player-theatre-mode-button') ? theater :
        selector.includes('right-column__toggle-collapse-btn') ? chat : null,
      querySelectorAll: () => []}};
  const state = JSON.parse(vm.runInNewContext(script, context));
  assert.equal(state.ended, true);
  assert.equal(typeof listeners.ended, 'function');
  return state;
}

assert.equal(readState(video => ({contains: element => element === video})).embeddedFullscreen, true,
  'A fullscreen wrapper containing the video counts as fullscreen');
assert.equal(readState(null, true).embeddedFullscreen, true,
  'YouTube player fullscreen is recognized without a document fullscreen element');
assert.equal(readState(null).embeddedFullscreen, false);
console.log('Browser video script: fullscreen containers and end detection passed.');

async function checkControls() {
  const volumeScript=source.match(/const BROWSER_VOLUME_SCRIPT: &str = r#"([\s\S]*?)"#;/)[1];
  const video={volume:0.8,muted:true,currentTime:31,play:async()=>{video.paused=false;}};
  const document={querySelector:selector=>selector==='video'?video:null,fullscreenElement:null};
  const volume=vm.runInNewContext(`(${volumeScript})`,{document});
  assert.equal(JSON.parse(volume('volume',0.3)).volume,0.3);
  assert.equal(video.muted,false,'Setting positive volume must unmute');
  video.muted=true;
  assert.equal(JSON.parse(volume('volume_up',0)).volume,0.05);
  assert.equal(JSON.parse(volume('volume',10)).volume,1);
  assert.equal(JSON.parse(volume('volume_down',0)).volume,0.95);

  const fullscreenScript=source.match(/const YOUTUBE_FULLSCREEN_SCRIPT: &str = r#"([\s\S]*?)"#;/)[1];
  const player={contains:node=>node===video,requestFullscreen:async()=>{document.fullscreenElement=player;}};
  document.querySelector=selector=>selector==='video'?video:player;
  assert.equal(JSON.parse(await vm.runInNewContext(fullscreenScript,{document})).fullscreen,true);
  document.fullscreenElement=null;
  player.requestFullscreen=async()=>{throw Error('permission denied');};
  const blocked=JSON.parse(await vm.runInNewContext(fullscreenScript,{document}));
  assert.equal(blocked.fullscreen,false);
  assert.match(blocked.error,/permission denied/);

  const repeatScript=source.match(/const BROWSER_REPEAT_SCRIPT: &str = r#"([\s\S]*?)"#;/)[1];
  const window={__mediabridgeVideoEnded:true};
  const result=JSON.parse(await vm.runInNewContext(repeatScript,{document,window}));
  assert.equal(result.restarted,true);
  assert.equal(video.currentTime,0);
  assert.equal(video.paused,false);
  assert.equal(window.__mediabridgeVideoEnded,false);
  console.log('Browser controls: unmute/volume, fullscreen confirmation/failure, and repeat passed.');
}
checkControls().catch(error=>{console.error(error);process.exitCode=1;});
