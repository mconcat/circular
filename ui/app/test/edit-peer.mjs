import { establish } from '@circular/client';
import { envelope, wireEnvelopeCodec } from '@circular/protocol';
import { declarationCommandFromValue, declarationPayloadValue } from '@circular/protocol/declaration';
import { fixture, snapshotAnswer, createInputSlot, harnessCandidatesPage, ack, creditAck, uint, edgeDepthRow, edgeDepthsComplete } from './fixtures.mjs';
import { key } from '../renderer/scene.mjs';
import { createHash } from 'node:crypto';

export function createInputsAnswer(types) {
  const anchor = Object.keys(types);
  return { anchor, items: anchor.map(actor_type => {
    const slots = Object.entries(types[actor_type] ?? {}).sort(([a], [b]) => a < b ? -1 : a > b ? 1 : 0)
      .map(([key, shape]) => createInputSlot({ path: [[1n, key]], requirement: [1n], shape }));
    return { actor_type, state: slots.length ? [2n, { relations: [], slots }, {}, slots.map(slot => slot.path)] : [1n] };
  }) };
}
export const anyShapes = config => Object.fromEntries(Object.keys(config ?? {}).map(key => [key, [1n]]));

export async function editPeer(options = {}) {
  const daemon = editDaemon(options);
  return { ...daemon, session: await establish(daemon.transport, { resourceCeilings: daemon.limits }) };
}
export function editDaemon({ reject, admit, fixtureValue = fixture(), admissionPorts = { in: [], out: [] }, lifecycle, injectAnswer, quietFeeds = false } = {}) {
  const f = fixtureValue, frames = [], sent = [], runFrames = [], runSent = [], injectFrames = [];
  const limits = { maximumBytes: 1048576, maximumDepth: 64, maximumContainerEntries: 4096, maximumStringBytes: 65536 };
  const codec = wireEnvelopeCodec(limits), epoch = new Uint8Array([91]);
  const queries = [], subscribed = [];
  if (f.system === undefined) f.system = new Map(f.harnesses ?? []);
  const snapshots = f.snapshots ??= new Map();
  const remember = () => snapshots.set(f.snapshot.value.anchor.cursor,
    {...f.snapshot, value:{...f.snapshot.value, anchor:{...f.snapshot.value.anchor}}});
  remember();
  const through = cut => (f.events.items ?? []).filter(row => cut === undefined ||
    cut.some(c => key(c.actor) === key(row.actor) && row.index < c.index));
  const relative = actor => ({ ...actor, scope: actor.scope.slice(f.snapshot.value.anchor.scope.length) });
  const actorKey = actor => key(relative(actor));
  const replace = (commands, command, field) => [...commands.filter(c => c.kind !== command.kind || key(c[field]) !== key(command[field])), command];
  function fold(commands, c) {
    if (c.kind === 'UpsertActor') return replace(commands, {...c,actor:{arm:'relative',value:relative(c.actor.value)}},'actor');
    if (c.kind === 'UpsertScope') return replace(commands,{...c,scope:{arm:'relative',value:c.scope.value.slice(f.snapshot.value.anchor.scope.length)}},'scope');
    if (c.kind === 'SetFlags') return commands.map(row => row.kind === 'UpsertActor' && key(row.actor.value) === actorKey(c.actor.value)
      ? {...row,declaration:{...row.declaration,flags:c.flags}} : row);
    if (c.kind === 'SetPresentation') return replace(commands,{...c,owner:Object.fromEntries(Object.entries(c.owner).map(([kind,address])=>[kind,{arm:'relative',value:relative(address.value)}]))},'owner');
    if (c.kind === 'RetireActor') return commands.filter(row => !((row.kind === 'UpsertActor' && key(row.actor.value) === actorKey(c.actor.value))
      || (row.kind === 'SetPresentation' && row.owner.actor && key(row.owner.actor.value) === actorKey(c.actor.value))));
    if (c.kind === 'UpsertEdge' || c.kind === 'RetireEdge') {
      const endpoint = p => ({...p,actor:relative(p.actor)});
      const edge = {arm:'relative',value:{...c.edge.value,from:endpoint(c.edge.value.from),to:endpoint(c.edge.value.to)}};
      if (c.kind === 'RetireEdge') return commands.filter(row => row.kind !== 'UpsertEdge' || key(row.edge) !== key(edge));
      return replace(commands,{...c,edge,declaration:{...c.declaration,from:endpoint(c.declaration.from),to:endpoint(c.declaration.to)}},'edge');
    }
    if (c.kind === 'RetireScope') return commands.filter(row => row.kind !== 'UpsertScope'
      || key(row.scope.value) !== key(c.scope.value.slice(f.snapshot.value.anchor.scope.length)));
    if (c.kind === 'UpsertExportMount') {
      const end = p => ({...p,actor:relative(p.actor)});
      return replace(commands,{...c,mount:{arm:'relative',value:relative(c.mount.value)},
        declaration:{...c.declaration,roles:Object.fromEntries(Object.entries(c.declaration.roles).map(([r,e]) => [r,end(e)]))}},'mount');
    }
    if (c.kind === 'RetireExportMount') return commands.filter(row => row.kind !== 'UpsertExportMount' || key(row.mount.value) !== actorKey(c.mount.value));
    if (c.kind === 'UpsertAnnotation') return replace(commands,{...c,annotation:{arm:'relative',value:relative(c.annotation.value)},declaration:{...c.declaration,refs:c.declaration.refs.map(relative)}},'annotation');
    if (c.kind === 'RetireAnnotation') return commands.filter(row => !(row.kind === 'UpsertAnnotation' && key(row.annotation.value) === actorKey(c.annotation.value)) && !(row.kind === 'SetPresentation' && row.owner.annotation && key(row.owner.annotation.value) === actorKey(c.annotation.value)));
    if (c.kind === 'UpsertTemplate') return replace(commands, c, 'name');
    if (c.kind === 'RetireTemplate') return commands.filter(row => row.kind !== 'UpsertTemplate' || row.name !== c.name);
    if (c.kind === 'MoveToScope') {
      const moved = new Set(c.actors.map(a => actorKey(a.value)));
      const relocate = a => moved.has(key(a)) ? {...a,scope:c.target.value.slice(f.snapshot.value.anchor.scope.length)} : a;
      return commands.map(row => {
        if (row.kind === 'UpsertActor') return {...row,actor:{...row.actor,value:relocate(row.actor.value)}};
        if (row.kind === 'SetPresentation' && row.owner.actor) return {...row,owner:{actor:{...row.owner.actor,value:relocate(row.owner.actor.value)}}};
        if (row.kind === 'UpsertEdge' && moved.has(key(row.edge.value.from.actor)) && moved.has(key(row.edge.value.to.actor))) {
          const end = p => ({...p,actor:relocate(p.actor)});
          const value = {...row.edge.value,from:end(row.edge.value.from),to:end(row.edge.value.to)};
          return {...row,edge:{...row.edge,value},declaration:{...row.declaration,from:end(row.declaration.from),to:end(row.declaration.to)}};
        }
        return row;
      });
    }
    throw new Error(`Missing fixture fold: ${c.kind}`);
  }
  const at = actor => ({ ...actor, scope: [...f.snapshot.value.anchor.scope, ...actor.scope] });
  const end = p => ({ ...p, actor: at(p.actor) }), abs = value => ({ arm: 'absolute', value });
  const field = { UpsertActor: 'actor', UpsertScope: 'scope', UpsertEdge: 'edge', UpsertExportMount: 'mount', UpsertAnnotation: 'annotation', SetPresentation: 'owner', UpsertTemplate: 'name' };
  function absolute(row) {
    if (row.kind === 'UpsertTemplate') return row;
    if (row.kind === 'UpsertActor') return {...row,actor:abs(at(row.actor.value))};
    if (row.kind === 'UpsertScope') return {...row,scope:abs([...f.snapshot.value.anchor.scope,...row.scope.value])};
    if (row.kind === 'SetPresentation') return {...row,owner:Object.fromEntries(Object.entries(row.owner).map(([kind,address]) => [kind,abs(at(address.value))]))};
    if (row.kind === 'UpsertEdge') return {...row,edge:abs({...row.edge.value,from:end(row.edge.value.from),to:end(row.edge.value.to)}),
      declaration:{...row.declaration,from:end(row.declaration.from),to:end(row.declaration.to)}};
    if (row.kind === 'UpsertExportMount') return {...row,mount:abs(at(row.mount.value)),
      declaration:{...row.declaration,roles:Object.fromEntries(Object.entries(row.declaration.roles).map(([r,e]) => [r,end(e)]))}};
    if (row.kind === 'UpsertAnnotation') return {...row,annotation:abs(at(row.annotation.value)),declaration:{...row.declaration,refs:row.declaration.refs.map(at)}};
    throw new Error(`Missing fixture delta row: ${row.kind}`);
  }
  const delta = (before, after) => [
    ...before.filter(row => row.kind !== 'SetPresentation' && !after.some(next => next.kind === row.kind && key(next[field[row.kind]]) === key(row[field[row.kind]])))
      .map(row => ({kind:row.kind.replace('Upsert','Retire'),[field[row.kind]]:absolute(row)[field[row.kind]]})),
    ...after.filter(row => !before.some(prior => key(prior) === key(row))).map(absolute)];
  const wire = c => declarationPayloadValue(c, {context:'acceptedHistory', includeKind:true});
  function respond(request, session) {
    const {partition,verb} = request.kind, payload = request.payload;
    if (verb === 'Hello') return envelope(partition,'HelloAck',request.correlation,
      {features:payload.features,protocol_version:1n,roles:[],token:new Uint8Array(32),trust:1n});
    if (partition === 'Subscription' && quietFeeds) {
      if (verb === 'Subscribe') {
        subscribed.push(payload.target);
        const depths = payload.target === 'edge.depths';
        session.feeds.set(request.correlation, {target:payload.target, credit:0n, sent:0n,
          held:depths ? (f.edgeDepths ?? []).map(edgeDepthRow) : [], ends:depths && !f.edgeDepthsWaiting});
      }
      const open = session.feeds.get(request.correlation);
      if (verb === 'Unsubscribe') session.feeds.delete(request.correlation);
      if (verb !== 'Credit' || !open) return envelope(partition,'SubscribeAck',request.correlation,(verb === 'Credit' ? creditAck : ack).payload);
      open.credit += payload.frames;
      return envelope(partition,'SubscribeAck',request.correlation,[1n,{pending_after:uint(open.held.length)}]);
    }
    if (verb === 'Subscribe' || verb === 'Credit') return envelope(partition,'SubscribeAck',request.correlation,[2n,{code:22n,message:'Fixture does not simulate live records'}]);
    if (partition === 'ReplayControl') {
      if (verb === 'ReplayStart') session.lenses.set(request.correlation, payload.arrangement[1].cut);
      if (verb === 'ReplayRewind') {
        const target = payload.to ?? (Array.isArray(payload.pace) && payload.pace[0] === 3n ? payload.pace[1] : null);
        if (target) session.lenses.set(request.correlation, target.cut);
      }
      if (verb === 'ReplayEnd') session.lenses.delete(request.correlation);
      return envelope(partition, 'ReplayResult', request.correlation, 1n);
    }
    if (partition === 'Query') {
      queries.push(payload);
      const cut = payload.upto ?? (payload.lens === undefined ? undefined : session.lenses.get(Number(payload.lens)));
      let page;
      if (payload.name === 'authoring-snapshot') {
        const revision = cut === undefined ? f.snapshot.value.anchor.cursor
          : through(cut).reduce((rev, row) => row.origin[4].value > rev ? row.origin[4].value : rev, 0n);
        const snapshot = cut === undefined ? f.snapshot : snapshots.get(revision);
        if (!snapshot) return envelope(partition, 'QueryResult', request.correlation,
          [2n, {code:2n, message:'no arrival revision in cut'}]);
        page = snapshotAnswer({snapshot}).payload[1];
      } else if (payload.name === 'actor.create-admission') {
        const refused = admit?.(payload.args);
        if (refused) return envelope(partition,'QueryResult',request.correlation,[2n,refused]);
        page = {anchor:payload.args.actor_type,items:[{actor_type:payload.args.actor_type,config:payload.args.config,authored_actor:payload.args.authored_actor,in_ports:admissionPorts.in,out_ports:admissionPorts.out}],terminal:2n};
      } else {
        const types = f.catalog.value.items.map(row => row.actor_type);
        const pages = {'actor.catalog':f.catalog.value,'authoring.actor-ports':f.ports.value,
          'daemon.health':f.health,'actor.events':cut === undefined ? f.events : {...f.events, items:through(cut), cut},
          'dead.letters':f.deadLetters ?? {anchor:null,items:[]},'instance.transitions':f.instanceTransitions ?? {anchor:null,items:[]},
          'actor.create-inputs':f.createInputs ?? {anchor:types,items:types.map(actor_type => ({actor_type,state:[1n]}))},
          'query.catalog':f.queryCatalog ?? {anchor:{preprocess:['map','filter','bang','parse','flatten'].map(kind => ({kind,slots:[]})),queries:['query.catalog']},items:[]},
          'agent.harnesses':(rows => ({anchor:rows,items:rows}))([...new Set([...(f.system ?? new Map()).keys(), ...(f.harnesses ?? new Map()).keys()])]
            .sort().map(name => ({name,program:f.system?.get(name) ?? null,saved:f.harnesses?.get(name) ?? null}))),
          'agent.harness-candidates':harnessCandidatesPage(f.harnessCandidates),
          records:{anchor:[],items:[]},'runtime.approvals':{anchor:{producer:1n,persistence:[3n]},items:[{item:[1n],emitter:{scope:[],local:'a'},target_effect:[2n],state:1n}]},
          ...f.queries};
        if (typeof pages[payload.name] === 'function') pages[payload.name] = pages[payload.name](payload);
        if (!pages[payload.name]) throw new Error(`Missing query fixture ${payload.name}`);
        if (pages[payload.name].rejected) return envelope(partition,'QueryResult',request.correlation,[2n,pages[payload.name].rejected]);
        page = {...pages[payload.name],anchor:pages[payload.name].anchor ?? null,terminal:2n};
      }
      return envelope(partition,'QueryResult',request.correlation,[1n,page]);
    }
    if (partition === 'Lifecycle') {
      runSent.push({verb,payload:{...payload}});
      const answer = lifecycle?.(verb,payload) ?? (verb === 'Pause' ? [1n,2n] : [1n,1n]);
      return envelope('Lifecycle','LifecycleResult',request.correlation,answer);
    }
    if (partition === 'EventInjection') return envelope('EventInjection','InjectAck',request.correlation,injectAnswer?.(payload) ?? 1n);
    if (partition === 'LedgerTransition' && verb === 'SetAgentHarness') {
      const c = {kind:'SetAgentHarness',name:payload.name,program:payload.program};
      sent.push(c);
      const refused = reject?.(c);
      if (refused) return envelope(partition,'TransitionResult',request.correlation,
        [2n,Object.fromEntries(Object.entries(refused).filter(([,value]) => value !== null && value !== undefined))]);
      for (const side of [f.harnesses ??= new Map(), f.system]) {
        if (!side) continue;
        if (c.program === null) side.delete(c.name); else side.set(c.name,c.program);
      }
      return envelope(partition,'TransitionResult',request.correlation,[1n,{at:null}]);
    }
    const c = declarationCommandFromValue(verb,payload);
    sent.push(c);
    const rejected = reject?.(c);
    let answer = 1n;
    if (rejected) { answer = [2n,rejected]; if (verb === 'CommitEpoch') session.candidate = undefined; }
    else if (verb === 'BeginEpoch') { session.candidate = f.snapshot.value.commands; session.opened = {begin:c,content:[]}; answer = [1n,epoch]; }
    else if (verb === 'AbortEpoch') session.candidate = undefined;
    else if (verb === 'CommitEpoch') {
      const before = f.snapshot.value.anchor.authoringRevision.revision;
      ++f.snapshot.value.anchor.cursor;
      const after = new Uint8Array(createHash('sha256').update(key([...session.candidate].sort((a,b)=>key(a).localeCompare(key(b))))).digest());
      const environment = {declaration_schema:f.snapshot.value.anchor.environment.declarationSchema,spec_set:f.snapshot.value.anchor.environment.specSet};
      const scope = f.snapshot.value.anchor.scope.map(s => [1n,s.name]);
      const prior = f.snapshot.value.commands;
      f.snapshot.value = {...f.snapshot.value,commands:session.candidate,anchor:{...f.snapshot.value.anchor,authoringRevision:{kind:'At',revision:after}}};
      remember();
      session.candidate = undefined;
      if (f.system === null) f.system = new Map(f.harnesses ?? []);
      const metadata = {target_scope:scope,cursor:f.snapshot.value.anchor.cursor,before_environment:environment,after_environment:environment,
        revisions:[{scope,authoring_before:[2n,before],authoring_after:[2n,after],topology_before:[2n,before],topology_after:[2n,after]}]};
      answer = [1n,metadata];
      hand(session, 'authoring-commits', {delta:delta(prior,f.snapshot.value.commands).map(wire),epoch:{begin:wire(session.opened.begin),content:session.opened.content.map(wire),terminal:wire(c)},metadata});
    } else if (verb !== 'ValidateEpoch') { session.candidate = fold(session.candidate,c); session.opened?.content.push(c); }
    return envelope('Declaration','CommandResult',request.correlation,answer);
  }
  function hand(session, target, payload) { for (const open of session.feeds.values()) if (open.target === target) open.held.push(payload); }
  const connections = new Set();
  function connect() {
    const session = {candidate:undefined, opened:undefined, feeds:new Map(), lenses:new Map()}, queue = [];
    let waiting, closed = false;
    const deliver = value => {
      const reply = codec.encode(value);
      if (waiting) { const resolve = waiting; waiting = undefined; resolve({value:reply,done:false}); } else queue.push(reply);
    };
    const flush = () => {
      for (const [correlation, open] of session.feeds) {
        while (open.credit > 0n && open.held.length) {
          open.credit -= 1n; open.sent += 1n;
          const payload = open.held.shift();
          deliver(envelope('Subscription','Frame',correlation,[3n,{origin:2n,pending_after:uint(open.held.length),payload}]));
        }
        if (open.ends && !open.held.length) {
          session.feeds.delete(correlation);
          deliver(envelope('Subscription','SubscriptionEnded',correlation,edgeDepthsComplete(open.sent)));
        }
      }
    };
    const connection = { session, flush };
    connections.add(connection);
    return {
      incoming: { [Symbol.asyncIterator]() { return this; }, next() {
        if (queue.length) return Promise.resolve({value:queue.shift(),done:false});
        if (closed) return Promise.resolve({done:true});
        return new Promise(resolve => { waiting = resolve; });
      } },
      async send(bytes) {
        const decoded = codec.decode(bytes); if (decoded.status !== 'complete') throw new Error('Invalid fixture request');
        if (decoded.envelope.kind.verb === 'Goodbye') return;
        if (decoded.envelope.kind.partition === 'Declaration') frames.push(Uint8Array.from(bytes));
        if (decoded.envelope.kind.partition === 'Lifecycle') runFrames.push(Uint8Array.from(bytes));
        if (decoded.envelope.kind.partition === 'EventInjection') injectFrames.push(Uint8Array.from(bytes));
        deliver(respond(decoded.envelope, session));
        flush();
      },
      async close() { closed = true; connections.delete(connection); waiting?.({done:true}); waiting = undefined; },
    };
  }
  const transport = connect();
  const feed = (target, payload) => { for (const {session, flush} of connections) { hand(session, target, payload); flush(); } };
  return {transport,connect,limits,f,frames,sent,epoch,runFrames,runSent,injectFrames,codec,queries,subscribed,feed};
}
