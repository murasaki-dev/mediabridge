import {readFileSync} from 'node:fs';
import {fetchObscura} from './preview-browser.mjs';

export const LIVE_SCRIPT = readFileSync(new URL('./obscura/youtube-live.js',import.meta.url),'utf8');

const recent = new Map();
const pending = new Map();
export function youtubeLive(id) {
  const cached = recent.get(id);
  if (cached && Date.now() - cached.time < 30000) return Promise.resolve(cached.result);
  if (pending.has(id)) return pending.get(id);
  if (pending.size >= 2) return Promise.reject(new Error('Too many live checks'));
  const check = fetchObscura(`https://www.youtube.com/live_chat?v=${id}`,LIVE_SCRIPT).then(result => {
    if (typeof result.live !== 'boolean') throw new Error('Could not determine live status');
    if (recent.size >= 16) recent.delete(recent.keys().next().value);
    recent.set(id,{time:Date.now(),result});
    return result;
  }).finally(() => pending.delete(id));
  pending.set(id,check);
  return check;
}
