import {webMediaUrl} from './api.ts';

export function liveChat(value: string, parent: string) {
  const source = webMediaUrl(value.trim());
  if (!source) return;
  const url = new URL(source);
  const path = url.pathname.split('/').filter(Boolean);
  if (['twitch.tv','www.twitch.tv','m.twitch.tv'].includes(url.hostname)
      && path.length === 1 && /^[A-Za-z0-9_]{1,25}$/.test(path[0])
      && !/^(?:directory|videos|clips|downloads|search|settings)$/i.test(path[0])) {
    const channel = path[0];
    const embed = new URL(`https://www.twitch.tv/embed/${channel}/chat`);
    embed.searchParams.set('parent',parent);
    return {provider:'Twitch', id:channel.toLowerCase(), embed:embed.href, direct:`https://www.twitch.tv/popout/${channel}/chat`};
  }
  const id = url.hostname === 'youtu.be' && path.length === 1 ? path[0]
    : ['youtube.com','www.youtube.com','m.youtube.com'].includes(url.hostname)
      ? url.pathname === '/watch' ? url.searchParams.get('v') : path[0] === 'live' && path.length === 2 ? path[1] : undefined
      : undefined;
  if (id && /^[A-Za-z0-9_-]{11}$/.test(id)) {
    const direct = 'https://www.youtube.com/live_chat?v=' + id;
    const embed = new URL(direct);
    embed.searchParams.set('embed_domain',parent);
    return {provider:'YouTube',id,embed:embed.href,direct};
  }
}

export function themedChatEmbed(chat: NonNullable<ReturnType<typeof liveChat>>, scheme: 'dark' | 'light') {
  if (scheme === 'light') return chat.embed;
  const url = new URL(chat.embed);
  if (chat.provider === 'Twitch') url.searchParams.set('darkpopout','');
  else url.searchParams.set('dark_theme','1');
  return url.href;
}
