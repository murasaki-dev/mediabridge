import test from 'node:test';
import assert from 'node:assert/strict';
import {runInNewContext} from 'node:vm';
import {SEARCH_SCRIPT} from '../youtube-search.mjs';
import {LIVE_SCRIPT} from '../youtube-live.mjs';

test('extracts unique YouTube videos and skips ads and channels', () => {
  const card = {videoId:'abcdefghijk',title:{runs:[{text:'A concert'}]},
    ownerText:{simpleText:'Channel name'},lengthText:{simpleText:'4:37'}};
  const entries = [{videoRenderer:card},{videoRenderer:card},
    {videoRenderer:{...card,videoId:'12345678901',adBadge:{}}},
    {channelRenderer:{title:{simpleText:'Channel'}}}];
  const ytInitialData = {contents:{twoColumnSearchResultsRenderer:{primaryContents:{sectionListRenderer:{
    contents:[{itemSectionRenderer:{contents:entries}}],
  }}}}};
  const data = JSON.parse(JSON.stringify(runInNewContext(SEARCH_SCRIPT,{ytInitialData})));
  assert.deepEqual(data,{available:true,videos:[{
    title:'A concert',url:'https://www.youtube.com/watch?v=abcdefghijk',uploader:'Channel name',duration_seconds:277,
  }]});
  assert.equal(runInNewContext(SEARCH_SCRIPT,{}).available,false);
});

test('distinguishes active YouTube chat from replay, unavailable, and unknown pages', () => {
  const detect = ytInitialData => runInNewContext(LIVE_SCRIPT,{ytInitialData}).live;
  assert.equal(detect({contents:{liveChatRenderer:{}}}),true);
  assert.equal(detect({contents:{liveChatRenderer:{isReplay:true}}}),false);
  assert.equal(detect({contents:{messageRenderer:{}}}),false);
  assert.equal(detect({}),null);
});
