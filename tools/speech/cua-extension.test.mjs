import test from 'node:test';
import assert from 'node:assert/strict';
import {mkdtemp, rm} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {createModels} from '@earendil-works/pi-ai/models';
import {fauxProvider, fauxAssistantMessage, fauxToolCall} from '@earendil-works/pi-ai/providers/faux';
import {MemoryStorage} from '@earendil-works/pi-durable';
import {PiRuntime} from './pi-runtime.mjs';
import {openCuaExtension, cuaResourceTool} from './cua-extension.mjs';

const schema = {type:'object',additionalProperties:false,properties:{image:{type:'string'}},required:['image']};
test('Pi calls the copied server contract with original arguments and retains multimodal results', async t => {
  const stateDir=await mkdtemp(join(tmpdir(),'dojo-cua-pi-'));
  const faux=fauxProvider({models:[{id:'test',contextWindow:200000}]});
  const models=createModels(); models.setProvider(faux.provider);
  faux.setResponses([
    fauxAssistantMessage([fauxToolCall('cua_sandbox_create',{image:'fixture-image'})],{stopReason:'toolUse'}),
    fauxAssistantMessage('created',{stopReason:'stop'}),
  ]);
  const calls=[];
  const image={type:'image',data:'fixture-png',mimeType:'image/png'};
  const client={tools:[{name:'sandbox_create',description:'Original sandbox operation',inputSchema:schema}],closed:false,
    close(){this.closed=true;},async call(method,params){calls.push({method,params});return {content:[{type:'text',text:'{"id":"fixture-sandbox"}'},image]};}};
  const records=[];
  const runtime=await PiRuntime.open({stateDir,root:process.cwd(),memory:{
    async append(record){records.push(record);},async readyView(){return '<chat>\n</chat>';},
    async zoom(){return 'original';},async date(){return '2026-01-01';},
  },config:{storage:new MemoryStorage(),models,provider:'faux',model:'test',instructions:'fixed',browserTools:false,desktopTools:false,cuaClientFactory:async()=>client}});
  t.after(async()=>{await runtime.close();await rm(stateDir,{recursive:true,force:true});});
  await runtime.submit('Create the fixture sandbox');
  await runtime.queue;
  await runtime.conversation.waitForIdle(runtime.context);
  await runtime.archived;
  assert.deepEqual(calls,[{method:'tools/call',params:{name:'sandbox_create',arguments:{image:'fixture-image'}}}]);
  assert.ok(records.some(record=>JSON.stringify(record).includes('fixture-png')));
  await runtime.stop();
  assert.equal(client.closed,true);
});

test('original schemas stay intact and every guest operation is protected against automatic replay', async t => {
  const stateDir=await mkdtemp(join(tmpdir(),'dojo-cua-schema-'));t.after(()=>rm(stateDir,{recursive:true,force:true}));
  const client={tools:[{name:'sandbox_create',inputSchema:schema}],close(){this.closed=true;}};
  const runtime={stateDir,root:process.cwd(),pi:{defineTool:value=>value,defineExtension:value=>value},
    config:{cuaClientFactory:async()=>client}};
  const extension=await openCuaExtension(runtime);
  assert.equal(extension.tools[0].parameters,schema);
  assert.equal(extension.tools[0].name,'cua_sandbox_create');
  assert.equal(extension.tools[0].replay,'unsafe');
  runtime.releaseCua();
  await assert.rejects(extension.tools[0].execute({image:'fixture'},null,{}),/connection retired/);
});

 test('CUA scheduling, provider calls and generic dispatch remain outside the Pi resource extension', async t => {
  assert.equal(cuaResourceTool('computer_future_mutation'),false);
  const stateDir=await mkdtemp(join(tmpdir(),'dojo-cua-boundary-'));
  t.after(()=>rm(stateDir,{recursive:true,force:true}));
  const names=['sandbox_list','computer_screenshot','skills_list','list_spaces','agent_start','persistent_agent_create','routine_add','call_tool','computer_access_grant'];
  const runtime={stateDir,root:process.cwd(),pi:{defineTool:value=>value,defineExtension:value=>value},
    config:{cuaClientFactory:async()=>({tools:names.map(name=>({name,inputSchema:schema})),close(){}})}};
  const extension=await openCuaExtension(runtime);
  assert.deepEqual(extension.tools.map(tool=>tool.name),['cua_sandbox_list','cua_computer_screenshot','cua_skills_list','cua_list_spaces']);
 });

test('Pi executes list_spaces through the real copied CUA CLI', {skip: !process.env.DOJO_TEST_CUA_BINARY}, async t => {
  const stateDir=await mkdtemp(join(tmpdir(),'dojo-cua-real-pi-'));
  const faux=fauxProvider({models:[{id:'test',contextWindow:200000}]});
  const models=createModels(); models.setProvider(faux.provider);
  faux.setResponses([
    fauxAssistantMessage([fauxToolCall('cua_list_spaces',{})],{stopReason:'toolUse'}),
    fauxAssistantMessage('No spaces registered',{stopReason:'stop'}),
  ]);
  const records=[];
  const runtime=await PiRuntime.open({stateDir,root:process.cwd(),memory:{
    async append(record){records.push(record);},async readyView(){return '<chat>\n</chat>';},
    async zoom(){return 'original';},async date(){return '2026-01-01';},
  },config:{storage:new MemoryStorage(),models,provider:'faux',model:'test',instructions:'fixed',browserTools:false,
    desktopTools:false,cuaTools:true,cuaBinary:process.env.DOJO_TEST_CUA_BINARY}});
  t.after(async()=>{await runtime.close();await rm(stateDir,{recursive:true,force:true});});
  assert.ok(runtime.cua.tools.some(tool=>tool.name==='list_spaces'));
  await runtime.submit('List spaces');
  await runtime.queue;
  await runtime.conversation.waitForIdle(runtime.context);
  await runtime.archived;
  assert.ok(records.some(record=>record.kind==='echo' && record.text==='[]'),JSON.stringify(records));
});

test('a fresh invocation reconnects after stop without replaying the previous operation',async t=>{
  const stateDir=await mkdtemp(join(tmpdir(),'dojo-cua-reconnect-'));t.after(()=>rm(stateDir,{recursive:true,force:true}));
  const calls=[];let opened=0;
  const runtime={stateDir,root:process.cwd(),pi:{defineTool:value=>value,defineExtension:value=>value},
    config:{cuaClientFactory:async()=>{const id=++opened;return {tools:[{name:'sandbox_list',inputSchema:schema}],
      closed:false,close(){this.closed=true;},async call(_method,args){calls.push([id,args]);return {content:[]};}};}}};
  const extension=await openCuaExtension(runtime);const tool=extension.tools[0];
  await tool.execute({},null,{});runtime.releaseCua();await tool.execute({},null,{});
  assert.equal(opened,2);assert.deepEqual(calls,[[1,{name:'sandbox_list',arguments:{}}],[2,{name:'sandbox_list',arguments:{}}]]);
  runtime.releaseCua();
});

test('retirement during reconnect closes the replacement before sending effects',async t=>{
  const stateDir=await mkdtemp(join(tmpdir(),'dojo-cua-reconnect-stop-'));t.after(()=>rm(stateDir,{recursive:true,force:true}));
  let finish;let opened=0;let effects=0;
  const make=()=>({tools:[{name:'sandbox_list',inputSchema:schema}],closed:false,close(){this.closed=true;},async call(){effects++;return {content:[]};}});
  const runtime={stateDir,root:process.cwd(),pi:{defineTool:value=>value,defineExtension:value=>value},
    config:{cuaClientFactory:async()=>++opened===1?make():await new Promise(resolve=>{finish=resolve;})}};
  const extension=await openCuaExtension(runtime);runtime.releaseCua();
  const pending=extension.tools[0].execute({},null,{});runtime.releaseCua();const replacement=make();finish(replacement);
  await assert.rejects(pending,/connection retired/);assert.equal(replacement.closed,true);assert.equal(effects,0);
});

test('failed CUA startup releases the already opened desktop transport',async t=>{
  const stateDir=await mkdtemp(join(tmpdir(),'dojo-cua-open-failure-'));t.after(()=>rm(stateDir,{recursive:true,force:true}));
  const desktop={tools:['desktop_observe','desktop_capture_action'].map(name=>({name,inputSchema:schema})),closed:false,close(){this.closed=true;}};
  await assert.rejects(PiRuntime.open({stateDir,root:process.cwd(),memory:{},config:{instructions:'fixed',browserTools:false,
    desktopClientFactory:async()=>desktop,cuaClientFactory:async()=>{throw new Error('fixture CUA unavailable');}}}),/fixture CUA unavailable/);
  assert.equal(desktop.closed,true);
});
