import {execFile} from 'node:child_process';
import {mkdtemp, rm} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {promisify} from 'node:util';

const run = promisify(execFile);

export async function fetchObscura(url, script) {
  const storage = await mkdtemp(join(tmpdir(),'mediabridge-obscura-'));
  try {
    const {stdout} = await run(process.env.OBSCURA_BIN || 'obscura',[
      'fetch','--storage-dir',storage,url,'--quiet','--timeout','15',
      '--wait-until','domcontentloaded','--wait','0','--eval',script,
    ],{timeout:20000,maxBuffer:512*1024,env:{...process.env,RUST_LOG:'off'}});
    return JSON.parse(stdout);
  } finally { await rm(storage,{recursive:true,force:true}); }
}
