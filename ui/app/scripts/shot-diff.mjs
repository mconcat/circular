#!/usr/bin/env node
import fs from 'node:fs/promises';
import path from 'node:path';
import vm from 'node:vm';
import http from 'node:http';
import {spawn} from 'node:child_process';
import {PNG} from 'pngjs';
import pixelmatch from 'pixelmatch';
import {connect} from '../bridge/frames.mjs';
import {installCaptureFrame} from './capture-frame.mjs';
import {recordedStudyEntry, recordedStudyFiles} from './recorded-study.mjs';
const root=path.resolve(import.meta.dirname,'..');
const args=process.argv.slice(2), option=n=>args.includes(n)?args[args.indexOf(n)+1]:undefined;
if(args.includes('--help')) {
  console.log('node scripts/shot-diff.mjs --chrome <path> --scene <canvas|dense|configure|approvals|error> --state <dir> [--theme <light|dark>] [--connection <json>] [--adapter-fixture <json>] [--out <dir>]\nUse --write-adapter-fixture <json> to export the original study data for a zero-diff comparison. A daemon scene reads an isolated --state through relayed frames, as the shell does. Connection JSON supplies the SDK-owned socketName when the installed SDK does not publish its default.');
  process.exit(0);
}
const writeFixture=option('--write-adapter-fixture');
if(writeFixture) {
  const context=vm.createContext({});context.window=context;
  for(const file of ['fixture.js','catalog-data.js','time-fixture.js']) vm.runInContext(await fs.readFile(path.join(root,file),'utf8'),context);
  await fs.writeFile(path.resolve(writeFixture),JSON.stringify({study:context.STUDY,catalog:context.PUBLISHED_ACTORS,history:context.STUDY_HISTORY}));
  console.log(path.resolve(writeFixture));process.exit(0);
}
const chrome=option('--chrome'),scene=option('--scene')??'canvas',state=option('--state');
const theme=option('--theme')??'light';
if(!['light','dark'].includes(theme)) throw new Error('--theme must be light or dark');
if(!chrome || !['canvas','dense','configure','approvals','error'].includes(scene) || (!state && !option('--adapter-fixture'))) throw new Error('See --help: Chrome and an explicit state or static adapter input are required');
const out=path.resolve(option('--out')??path.join(root,'.shots',scene));await fs.mkdir(out,{recursive:true});
const profile=await fs.mkdtemp(path.join(out,'chrome-'));
const input=option('--adapter-fixture') ? await fs.readFile(path.resolve(option('--adapter-fixture')),'utf8') : null;
const recordedFiles=input === null ? undefined : recordedStudyFiles(JSON.parse(input));
const presentationInput=Boolean(recordedFiles);
let transport,connectionCode;
const frames={queue:[],stream:null,push(line){this.stream?this.stream.write(`data: ${line}\n\n`):this.queue.push(line);}};
if(!input) {
  try {
    const connection=option('--connection')?JSON.parse(await fs.readFile(option('--connection'),'utf8')):{};
    transport=await connect({...connection,state});
    void (async()=>{try{for await(const bytes of transport.incoming) frames.push(Buffer.from(bytes).toString('base64'));}finally{frames.push('');}})();
  } catch(error) {connectionCode=error.code??'READ_UNAVAILABLE';}
}
const mime={'.js':'text/javascript','.mjs':'text/javascript','.css':'text/css','.html':'text/html','.ttf':'font/ttf','.json':'application/json'};
const server=http.createServer(async(req,res)=> {
  try {
    if(req.url==='/connection') {res.setHeader('Content-Type','application/json');res.end(JSON.stringify({connection:transport?'connected':connectionCode}));return;}
    if(req.url==='/frames' && req.method==='GET') {
      res.writeHead(200,{'Content-Type':'text/event-stream','Cache-Control':'no-store'});
      frames.stream=res;for(const line of frames.queue.splice(0)) res.write(`data: ${line}\n\n`);return;
    }
    if(req.url==='/frames' && req.method==='POST') {
      const chunks=[];for await(const chunk of req) chunks.push(chunk);
      let answer=null;
      try {await transport.send(new Uint8Array(Buffer.concat(chunks)));} catch(error) {answer={code:error.code??'READ_UNAVAILABLE'};}
      res.setHeader('Content-Type','application/json');res.end(JSON.stringify(answer));return;
    }
    if(req.url==='/frames/close' && req.method==='POST') {await transport?.close();res.end('null');return;}
    let pathname=decodeURIComponent(new URL(req.url,'http://localhost').pathname);
    const recorded=recordedFiles?.(pathname);
    if(recorded?.source !== undefined) {res.setHeader('Content-Type','text/javascript');res.end(recorded.source);return;}
    if(recorded) pathname=recorded.pathname;
    const file=path.resolve(root,'.'+pathname);
    if(!file.startsWith(root+path.sep)) {res.writeHead(403);res.end();return;}
    res.setHeader('Content-Type',mime[path.extname(file)]??'application/octet-stream');res.end(await fs.readFile(file));
  } catch(error) {res.writeHead(500);res.end(String(error));}
});
await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
const origin=`http://127.0.0.1:${server.address().port}`;
const child=spawn(chrome,['--headless=new','--no-first-run','--no-default-browser-check','--disable-background-networking',
  '--remote-debugging-pipe','--force-device-scale-factor=1','--hide-scrollbars',`--user-data-dir=${profile}`],{stdio:['ignore','ignore','pipe','pipe','pipe']});
let sequence=0,buffer='',pending=new Map();
child.stdio[4].on('data',chunk=> {
  buffer+=chunk;let end;
  while((end=buffer.indexOf('\0'))>=0) {
    const line=buffer.slice(0,end);buffer=buffer.slice(end+1);if(!line)continue;
    const message=JSON.parse(line),request=pending.get(message.id);
    if(request) {pending.delete(message.id);message.error?request.reject(new Error(JSON.stringify(message.error))):request.resolve(message.result);}
  }
});
let errors='';child.stderr.on('data',chunk=>{errors+=chunk;});
child.on('exit',code=>{for(const p of pending.values())p.reject(new Error(`Chrome exited ${code}: ${errors}`));pending.clear();});
const send=(method,params={},sessionId)=>new Promise((resolve,reject)=> {
  const id=++sequence;pending.set(id,{resolve,reject});child.stdio[3].write(JSON.stringify({id,method,params,...(sessionId?{sessionId}:{})})+'\0');
});
const deadline=setTimeout(()=>child.kill(),55000);
async function capture(fixture) {
  const fixedFrame = scene === 'canvas' && (fixture || presentationInput);
  const {targetId}=await send('Target.createTarget',{url:'about:blank'});
  const {sessionId}=await send('Target.attachToTarget',{targetId,flatten:true});
  const call=(method,params)=>send(method,params,sessionId);
  await call('Page.enable');await call('Runtime.enable');
  await call('Emulation.setDeviceMetricsOverride',{width:1600,height:1050,deviceScaleFactor:1,mobile:false});
  await call('Page.addScriptToEvaluateOnNewDocument',{source:`${fixedFrame ? `(${installCaptureFrame.toString()})();` : 'performance.now=()=>0; Date.now=()=>0; Math.random=()=>0.5;'}
    window.circularConnection=async()=>(await fetch('/connection')).json();
    window.circularFrames=(()=>{const handlers=[];let source,named;
      return {send:async(attachment,bytes)=>{named=attachment;return (await fetch('/frames',{method:'POST',body:bytes})).json();},
        onFrame(handler){handlers.push(handler);source??=new EventSource('/frames');
          source.onmessage=event=>{const bytes=event.data===''?null:Uint8Array.from(atob(event.data),c=>c.charCodeAt(0));for(const h of handlers)h(named,bytes);};},
        close:async()=>{await fetch('/frames/close',{method:'POST'});}};})();`});
  const query=new URLSearchParams({scene,theme,...(fixture||presentationInput?{fixture:'1'}:{state,...(connectionCode?{connectionCode}:{})})});
  const entry=!fixture&&presentationInput?recordedStudyEntry:'/index.html';
  await call('Page.navigate',{url:`${origin}${entry}?${query}`});
  let ready=false;
  for(let i=0;i<100&&!ready;i++) {
    ready=(await call('Runtime.evaluate',{expression:'Boolean(window.circularReady)',returnByValue:true})).result.value;
    if(!ready) await new Promise(resolve=>setTimeout(resolve,20));
  }
  if(!ready) throw new Error('Bootstrap did not start');
  const result=await call('Runtime.evaluate',{expression:`(async()=>{await circularReady;await document.fonts.ready;
    ${fixedFrame ? 'await window.captureLayout();' : ''}
    ${fixture||presentationInput?(await fs.readFile(path.join(root,'scripts/canvas-fixture.js'),'utf8')).replace('(async () => {','await (async () => {'):''}
    ${fixedFrame ? 'await window.captureFrame();' : 'await new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(r)));'}
    for(const animation of document.getAnimations()){animation.pause();animation.currentTime=0;}
    return {status:document.querySelector('#footer-status').textContent,nodes:StudyApp.graph().nodes.length};})()`,awaitPromise:true,returnByValue:true});
  if(result.exceptionDetails) throw new Error(JSON.stringify(result.exceptionDetails));
  const {data}=await call('Page.captureScreenshot',{format:'png',captureBeyondViewport:false});
  const bytes=Buffer.from(data,'base64');await fs.writeFile(path.join(out,fixture?'fixture.png':'product.png'),bytes);
  await send('Target.closeTarget',{targetId});return {bytes,observation:result.result.value};
}
try {
  const reference=await capture(true),product=await capture(false);
  const a=PNG.sync.read(reference.bytes),b=PNG.sync.read(product.bytes),diff=new PNG({width:a.width,height:a.height});
  const pixels=pixelmatch(a.data,b.data,diff.data,a.width,a.height,{threshold:0,includeAA:true});
  await fs.writeFile(path.join(out,'diff.png'),PNG.sync.write(diff));
  console.log(JSON.stringify({scene,theme,width:a.width,height:a.height,pixels,total:a.width*a.height,ratio:pixels/(a.width*a.height),
    reference:reference.observation,product:product.observation,connectionCode,identicalInput:Boolean(presentationInput),out},null,2));
  if(connectionCode || (presentationInput && pixels!==0)) process.exitCode=1;
} finally {
  clearTimeout(deadline);child.kill();frames.stream?.end();await transport?.close();server.closeAllConnections();await new Promise(resolve=>server.close(resolve));
  await new Promise(resolve=>child.exitCode!==null?resolve():child.once('exit',resolve));await fs.rm(profile,{recursive:true,force:true});
}
