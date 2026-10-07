import test from 'node:test';
import assert from 'node:assert/strict';
import { declarationPayloadValue, declarationCommandFromValue } from '../src/declaration.js';

test('presentation owners round trip both existing address kinds and refuse the old array', () => {
  for (const [context, arm, tag] of [['mutation','absolute',1n], ['mutation','epochLocal',2n], ['snapshot','relative',3n]]) {
    for (const kind of ['actor','annotation']) {
      const value={scope:[],local:'same'};
      const command={kind:'SetPresentation',owner:{[kind]:{arm,value}},presentation:{collapsed:false,fixed:{x:40n,y:20n},size:{w:240n,h:120n}}};
      const encoded=declarationPayloadValue(command,{context});
      assert.deepEqual(encoded.owner,{[kind]:[tag,{local:'same',scope:[]}]});
      assert.deepEqual(declarationCommandFromValue('SetPresentation',encoded,{context}),command);
      for (const owner of [[tag,value],{}, {actor:[tag,value],annotation:[tag,value]}, {note:[tag,value]}]) {
        assert.throws(()=>declarationCommandFromValue('SetPresentation',{...encoded,owner},{context}),{code:'PRESENTATION_OWNER_SHAPE'});
      }
      assert.throws(()=>declarationPayloadValue({...command,owner:{arm,value}},{context}),{code:'PRESENTATION_OWNER_SHAPE'});
    }
  }
});
