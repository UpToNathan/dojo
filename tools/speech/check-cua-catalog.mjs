#!/usr/bin/env node
import {mkdtemp, rm, readFile} from 'node:fs/promises';
import {join, resolve} from 'node:path';
import {tmpdir} from 'node:os';
import assert from 'node:assert/strict';
import {DesktopMcp} from './desktop-extension.mjs';

const root=resolve(import.meta.dirname,'../..');
const expected=JSON.parse(await readFile(join(root,'vendor/cua/metadata/pi-resource-catalog.json'),'utf8'));
const inventory=JSON.parse(await readFile(join(root,'vendor/cua/copy-inventory.json'),'utf8'));
assert.equal(expected.revision,inventory.revision);
const home=await mkdtemp(join(tmpdir(),'dojo-cua-catalog-check-'));
let client;
try {
  client=await DesktopMcp.open(root,{
    command:process.env.DOJO_TEST_CUA_BINARY ?? join(root,'vendor/cua/upstream/libs/cua/target/debug/cua'),
    args:expected.command,env:{PATH:process.env.PATH,CUA_HOME:home,DO_NOT_TRACK:'1'},
  });
  for (const tool of expected.tools) {
    const actual=client.tools.find(item=>item.name===tool.name);
    assert.ok(actual,`Missing copied CUA tool: ${tool.name}`);
    assert.deepEqual(actual,tool,`Copied CUA contract differs: ${tool.name}`);
  }
  const result=await client.call('tools/call',{name:'list_spaces',arguments:{}});
  assert.deepEqual(result,{content:[{text:'[]',type:'text'}],isError:false});
  console.log(JSON.stringify({revision:expected.revision,resource_tools:expected.tools.length,server_tools:client.tools.length,list_spaces:'passed'}));
} finally {client?.close();await rm(home,{recursive:true});}
