import {createServer, request} from 'node:http';
import {readFile, writeFile, rename, rm} from 'node:fs/promises';
import {randomUUID} from 'node:crypto';
import {searchYouTube} from './youtube-search.mjs';
import {youtubeLive} from './youtube-live.mjs';

const upstream = new URL(process.env.MEDIABRIDGE_UPSTREAM || 'http://192.168.0.14:8765');
const types = {html:'text/html; charset=utf-8',js:'text/javascript; charset=utf-8',css:'text/css; charset=utf-8',png:'image/png'};
const routes = new Map([
  ['/status','GET'], ['/artwork/channel','GET'], ['/history/youtube','GET'], ['/recommendations','GET'], ['/recommendations/home','GET'], ['/recommendations/home/short','GET'], ['/recommendations/home/long','GET'],
  ['/play_preserve_queue','POST'], ['/queue','POST'], ['/pause','POST'], ['/resume','POST'],
  ['/previous','POST'], ['/volume','POST'], ['/queue/play','POST'], ['/queue/remove','POST'], ['/queue/move','POST'],
  ['/next','POST'], ['/close','POST'], ['/seek','POST'], ['/autoplay','POST'], ['/sunshine','POST'],
  ['/mouse','POST'],
]);

const wallpaperFile = new URL('./data/wallpaper',import.meta.url);
const uploadLimit = 8 * 1024 * 1024;

function imageType(bytes) {
  if (bytes.subarray(0,8).equals(Buffer.from([137,80,78,71,13,10,26,10]))) return 'image/png';
  if (bytes.subarray(0,3).equals(Buffer.from([255,216,255]))) return 'image/jpeg';
  if (bytes.toString('ascii',0,4) === 'RIFF' && bytes.toString('ascii',8,12) === 'WEBP') return 'image/webp';
  return null;
}

async function wallpaper(incoming,outgoing) {
  const reply = (code, message) => outgoing.writeHead(code,{'Content-Type':'text/plain; charset=utf-8'}).end(message);
  try {
    if (incoming.method === 'GET' || incoming.method === 'HEAD') {
      const bytes = await readFile(wallpaperFile).catch(async error => {
        if (error.code !== 'ENOENT') throw error;
        try { return await readFile(new URL('./build/wallpaper.png',import.meta.url)); }
        catch (error) { if (error.code !== 'ENOENT') throw error; return null; }
      });
      if (!bytes) { outgoing.writeHead(204,{'Cache-Control':'no-cache'}).end(); return; }
      outgoing.writeHead(200,{'Content-Type':imageType(bytes) || 'image/png','Cache-Control':'no-cache','X-Content-Type-Options':'nosniff'})
        .end(incoming.method === 'HEAD' ? undefined : bytes);
      return;
    }
    if (incoming.method === 'DELETE') {
      await rm(wallpaperFile,{force:true});
      outgoing.writeHead(204).end();
      return;
    }
    if (incoming.method !== 'PUT') { reply(405,'Method unavailable.'); return; }
    const length = Number(incoming.headers['content-length']);
    if (!Number.isInteger(length) || length < 1) { reply(411,'Upload must include a file size.'); return; }
    if (length > uploadLimit) { reply(413,'Wallpaper must be 8 MB or smaller.'); return; }
    const chunks = [];
    let size = 0;
    for await (const chunk of incoming) {
      size += chunk.length;
      if (size > uploadLimit) { reply(413,'Wallpaper must be 8 MB or smaller.'); return; }
      chunks.push(chunk);
    }
    const bytes = Buffer.concat(chunks);
    if (!imageType(bytes)) { reply(415,'Choose a PNG, JPEG, or WebP image.'); return; }
    const temporary = new URL('./data/wallpaper-' + randomUUID(),import.meta.url);
    try { await writeFile(temporary,bytes,{flag:'wx'}); await rename(temporary,wallpaperFile); }
    finally { await rm(temporary,{force:true}); }
    outgoing.writeHead(204).end();
  } catch { reply(500,'The preview server could not access the wallpaper.'); }
}
createServer(async (incoming,outgoing) => {
  const host = incoming.headers.host;
  if (!/^(localhost|127\.0\.0\.1)(:\d+)?$/.test(host || '') ||
      (incoming.headers.origin && incoming.headers.origin !== 'http://' + host) ||
      incoming.headers['sec-fetch-site'] === 'cross-site') {
    outgoing.writeHead(403).end('Open this preview on localhost.');
    return;
  }
  const path = incoming.url.split('?')[0];
  if (path === '/api/search/youtube' && incoming.method === 'GET') {
    const query = new URL(incoming.url,'http://localhost').searchParams.get('q')?.trim();
    const reply = (status, body) => outgoing.writeHead(status,{'Content-Type':'application/json','Cache-Control':'no-store'}).end(JSON.stringify(body));
    if (!query || query.length > 120) { reply(400,{error:'Enter a search of 1–120 characters.'}); return; }
    try { reply(200,await searchYouTube(query)); }
    catch { reply(502,{error:'YouTube search could not load through Obscura. Try again shortly.'}); }
    return;
  }
  if (path === '/api/chat/youtube-live' && incoming.method === 'GET') {
    const id = new URL(incoming.url,'http://localhost').searchParams.get('id');
    const reply = (status, body) => outgoing.writeHead(status,{'Content-Type':'application/json','Cache-Control':'no-store'}).end(JSON.stringify(body));
    if (!/^[A-Za-z0-9_-]{11}$/.test(id || '')) { reply(400,{error:'Invalid YouTube video ID.'}); return; }
    try { reply(200,await youtubeLive(id)); }
    catch { reply(502,{error:'Could not confirm whether YouTube is live.'}); }
    return;
  }
  if (path === '/appearance/wallpaper') { await wallpaper(incoming,outgoing); return; }
  if (path === '/host-settings' && incoming.method === 'GET') {
    try {
      const options = {signal:AbortSignal.timeout(8000)};
      let response = await fetch(new URL('/legacy-dashboard',upstream),options);
      if (response.status === 404) response = await fetch(new URL('/',upstream),options);
      if (!response.ok) throw new Error();
      const html = await response.text();
      const assets = await Promise.all(['settings-embed.css','settings-embed.js'].map(name =>
        readFile(new URL('./build/' + name,import.meta.url),'utf8')));
      const bridge = `<style>${assets[0]}</style><script>window.__mediabridgePreviewSettings = true;\n${assets[1]}</script>`;
      outgoing.writeHead(200,{'Content-Type':types.html,'Cache-Control':'no-store'}).end(html.replace('</head>',bridge + '</head>'));
    } catch {
      outgoing.writeHead(502,{'Content-Type':types.html}).end('<p>MediaBridge settings are unavailable. Check the host connection and reopen settings.</p>');
    }
    return;
  }
  if (path.startsWith('/api/') || path.startsWith('/host-api/')) {
    const sharedSettings = path.startsWith('/host-api/');
    const target = path.slice(sharedSettings ? 9 : 4);
    const settingsRoute = /^\/settings\/[a-z0-9/-]+$/.test(target) && !target.includes('..') && !target.startsWith('/settings/recommendations/import');
    const settingsRead = incoming.method === 'GET' && ['/status','/preferences','/platform','/dial/status'].includes(target);
    const allowed = sharedSettings ? ((settingsRoute && ['GET','POST'].includes(incoming.method)) || settingsRead)
      : routes.get(target) === incoming.method
        || target === '/settings/mouse' && ['GET','POST'].includes(incoming.method)
        || ['/recommendations/home/short','/recommendations/home/long','/recommendations/home/extend'].includes(target) && incoming.method === 'POST'
        || incoming.method === 'GET' && /^\/subscriptions\/(?:youtube|twitch)(?:\/[A-Za-z0-9_-]+)?$/.test(target);
    if (!allowed) {
      outgoing.writeHead(404,{'Content-Type':'application/json'}).end('{"error":"Endpoint unavailable in the preview."}');
      return;
    }
    const headers = {};
    for (const name of ['content-type','content-length','accept']) {
      if (incoming.headers[name]) headers[name] = incoming.headers[name];
    }
    const proxy = request(upstream,{path:target + (incoming.url.includes('?') ? incoming.url.slice(incoming.url.indexOf('?')) : ''),method:incoming.method,headers,timeout:35000},response => {
      outgoing.writeHead(response.statusCode,{'Content-Type':response.headers['content-type'] || 'application/json','Cache-Control':target === '/artwork/channel' && response.statusCode === 200 ? (response.headers['cache-control'] || 'no-cache') : 'no-store','X-Content-Type-Options':'nosniff'});
      response.pipe(outgoing);
    });
    proxy.on('timeout',() => proxy.destroy());
    proxy.on('error',() => {
      if (outgoing.headersSent) outgoing.destroy();
      else outgoing.writeHead(502,{'Content-Type':'application/json'}).end('{"error":"Could not reach MediaBridge."}');
    });
    outgoing.on('close',() => proxy.destroy());
    incoming.pipe(proxy);
    return;
  }
  if (incoming.method !== 'GET' && incoming.method !== 'HEAD') { outgoing.writeHead(405).end(); return; }
  const page = /^\/(?:search|chat|mousepad|appearance|settings)?\/?$/.test(path);
  const asset = path === '/favicon.ico' ? '/icon.png' : page ? '/index.html' : path;
  if (!/^\/(?:_app\/[A-Za-z0-9_./-]+|index\.html|icon\.png|wallpaper\.png)$/.test(asset) || asset.split('/').includes('..')) {
    outgoing.writeHead(404).end();
    return;
  }
  try {
    const body = await readFile(new URL('./build' + asset,import.meta.url)).catch(async error => {
      if (error.code !== 'ENOENT' || asset !== '/icon.png') throw error;
      const icon = await fetch(new URL('/icon.png',upstream),{signal:AbortSignal.timeout(3000)});
      if (!icon.ok || icon.headers.get('content-type')?.split(';')[0] !== 'image/png') throw error;
      return Buffer.from(await icon.arrayBuffer());
    });
    outgoing.writeHead(200,{
      'Content-Type':types[asset.split('.').at(-1)] || 'application/octet-stream',
      'Cache-Control':asset.startsWith('/_app/immutable/') ? 'public, max-age=31536000, immutable' : 'no-cache',
      'X-Content-Type-Options':'nosniff',
    }).end(incoming.method === 'HEAD' ? undefined : body);
  } catch { outgoing.writeHead(404).end(); }
}).listen(8080,'0.0.0.0');
