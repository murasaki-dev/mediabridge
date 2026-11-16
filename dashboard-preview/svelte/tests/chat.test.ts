import test from 'node:test';
import assert from 'node:assert/strict';
import {liveChat, themedChatEmbed} from '../src/lib/live-chat.ts';

test('builds provider chat URLs for the dashboard hostname', () => {
  assert.equal(liveChat('https://youtu.be/abcdefghijk','localhost')?.embed,
    'https://www.youtube.com/live_chat?v=abcdefghijk&embed_domain=localhost');
  assert.equal(liveChat('https://www.twitch.tv/example_channel','127.0.0.1')?.embed,
    'https://www.twitch.tv/embed/example_channel/chat?parent=127.0.0.1');
  assert.equal(liveChat('https://www.twitch.tv/videos/123','localhost'),undefined);
  assert.equal(liveChat('https://www.twitch.tv.example.org/channel','localhost'),undefined);
});

test('adds each provider’s dark chat parameter without changing the light embed', () => {
  const youtube = liveChat('https://youtu.be/abcdefghijk','localhost')!;
  const twitch = liveChat('https://www.twitch.tv/example_channel','localhost')!;
  assert.equal(new URL(themedChatEmbed(youtube,'dark')).searchParams.get('dark_theme'),'1');
  assert.equal(new URL(themedChatEmbed(twitch,'dark')).searchParams.get('darkpopout'),'');
  assert.equal(themedChatEmbed(youtube,'light'),youtube.embed);
});
