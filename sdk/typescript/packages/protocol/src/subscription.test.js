import assert from 'node:assert/strict';
import test from 'node:test';
import { subscriptionFrameFromValue } from './subscription-values.js';
import { readFrame } from '../../client/src/subscription.js';
import { uint, encodeValueBeta, decodeValueBeta } from './value.js';
const ceilings = {maximumBytes:4096, maximumContainerEntries:128, maximumStringBytes:1024, maximumDepth:64};
for (const [value,hex] of [[0n,'090000000000000000'],[2n,'090000000000000002'],[18446744073709551615n,'09ffffffffffffffff']]) {
  test(`pending_after UInt ${value} survives wire decode and both receipts`, () => {
    assert.equal(Buffer.from(encodeValueBeta(uint(value),ceilings)).toString('hex'),hex);
    const pending = decodeValueBeta(Buffer.from(hex,'hex'),ceilings);
    const wire = [3n,{origin:2n,payload:'entry',pending_after:pending}];
    assert.deepEqual(subscriptionFrameFromValue(wire),{arm:'Credit',origin:'Live',payload:'entry',pending_after:uint(value)});
    assert.deepEqual(readFrame(wire),{arm:'Credit',origin:'Live',payload:'entry',pending_after:uint(value),folded:null,slot:null});
  });
}
for (const [name,fields] of [
  ['missing',{}],['null',{pending_after:null}],['Int',{pending_after:2n}],
  ['string',{pending_after:'2'}],['structural fake',{pending_after:{value:2n}}],
  ['extra',{pending_after:uint(2n),remaining:2n}],
]) test(`Credit rejects ${name} pending_after`, () => {
  for (const parse of [subscriptionFrameFromValue,readFrame]) {
    assert.throws(()=>parse([3n,{origin:2n,payload:null,...fields}]));
  }
});
for (const arm of [1n,2n]) test(`arm ${arm} cannot carry pending_after`, () => {
  for (const parse of [subscriptionFrameFromValue,readFrame]) {
    assert.throws(()=>parse([arm,{origin:2n,payload:null,folded:0n,pending_after:uint(0n)}]));
  }
});
