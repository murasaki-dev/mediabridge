import assert from 'node:assert/strict';
import {test} from 'node:test';
import {clock, rememberPosition, savedPosition, shortcut, visibleRange} from '../src/lib/logic.ts';

test('formats seconds and hours, clamps invalid positions', () => {
  assert.equal(clock(65.92), '1:05');
  assert.equal(clock(3605), '1:00:05');
  assert.equal(clock(Infinity), '0:00');
  assert.equal(clock(-5), '0:00');
});
test('persists playback progress by media URL and ignores corrupt data', () => {
  const values = new Map<string,string>();
  const storage = {getItem: (key: string) => values.get(key) ?? null, setItem: (key: string,value: string) => {values.set(key,value);}};
  rememberPosition(storage,'https://example.test/a.mp4',32);
  rememberPosition(storage,'https://example.test/b.mp4',12);
  assert.equal(savedPosition(storage,'https://example.test/a.mp4'),32);
  rememberPosition(storage,'https://example.test/a.mp4',Infinity);
  assert.equal(savedPosition(storage,'https://example.test/a.mp4'),32);
  storage.setItem('mb-progress','{');
  assert.equal(savedPosition(storage,'https://example.test/a.mp4'),0);
});
test('keyboard shortcuts leave form inputs and modifier shortcuts alone', () => {
  assert.equal(shortcut(' ',false,false),'toggle');
  assert.equal(shortcut('ArrowLeft',false,false),'back');
  assert.equal(shortcut('ArrowRight',false,false),'forward');
  assert.equal(shortcut(' ',true,false),null);
  assert.equal(shortcut('ArrowRight',false,true),null);
});
test('grid only mounts rows within the viewport plus overscan', () => {
  assert.deepEqual(visibleRange(1000,4,244,2440,480),{start:36,end:52,offset:2196});
  assert.deepEqual(visibleRange(6,4,244,0,480),{start:0,end:6,offset:0});
});
