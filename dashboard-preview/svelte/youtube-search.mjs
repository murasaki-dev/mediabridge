import {readFileSync} from 'node:fs';
import {fetchObscura} from './preview-browser.mjs';

export const SEARCH_SCRIPT = readFileSync(new URL('./obscura/youtube-search.js',import.meta.url),'utf8');

const cache = new Map(), pending = new Map();

async function fetchSearch(query) {
  const url = new URL('https://www.youtube.com/results');
  url.searchParams.set('search_query',query);
  const result = await fetchObscura(url.href,SEARCH_SCRIPT);
  if (!result.available || !Array.isArray(result.videos)) throw Error('YouTube search results were unavailable.');
  return {videos:result.videos};
}

export function searchYouTube(query) {
  const cached = cache.get(query);
  if (cached && Date.now() - cached.time < 120000) return Promise.resolve(cached.result);
  if (pending.has(query)) return pending.get(query);
  if (pending.size >= 2) return Promise.reject(new Error('Two searches are already running. Try again shortly.'));
  const request = fetchSearch(query).then(result => {
    if (cache.size >= 16) cache.delete(cache.keys().next().value);
    cache.set(query,{time:Date.now(),result});
    return result;
  }).finally(() => pending.delete(query));
  pending.set(query,request);
  return request;
}
