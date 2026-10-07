import test from 'node:test';
import assert from 'node:assert/strict';
import { formatDuration } from '../renderer/view-registry.mjs';

const unitMs = { ms: 1, s: 1000, min: 60000, h: 3600000 };
const larger = { ms: 's', s: 'min', min: 'h', h: null };
const printed = /^(-?)(\d+(?:\.\d{1,2})?) (ms|s|min|h)$/;
function random(seed) {
  return () => { seed |= 0; seed = seed + 0x6D2B79F5 | 0; let t = Math.imul(seed ^ seed >>> 15, 1 | seed);
    t = t + Math.imul(t ^ t >>> 7, 61 | t) ^ t; return ((t ^ t >>> 14) >>> 0) / 4294967296; };
}
function durations(count, next = random(1348)) {
  const out = [0, 1, -1];
  for (const size of Object.values(unitMs)) for (const k of [1, 2, 59, 60, 61, 99, 100, 1000])
    for (const d of [-2, -1, 0, 1, 2]) out.push(k * size + d);
  for (const size of [1000, 60000, 3600000]) for (const d of [-2, -1, 0, 1, 2]) out.push(Math.round(0.995 * size) + d);
  while (out.length < count) {
    const magnitude = Math.floor(next() * 13), integer = next() < 0.8;
    const value = next() * 10 ** magnitude;
    out.push((next() < 0.1 ? -1 : 1) * (integer ? Math.floor(value) : value));
  }
  return out;
}

function check(recorded, ms, spelled) {
  const reading = formatDuration(recorded);
  assert.ok(reading, `${spelled}: a number is formatted`);
  const match = printed.exec(reading.text);
  assert.ok(match, `${spelled}: printed "${reading.text}" is a value and a unit`);
  const [, sign, digits, unit] = match, value = Number(digits), size = unitMs[unit], magnitude = Math.abs(ms);
  assert.equal(sign === '-', ms < 0 && value !== 0, `${spelled}: sign of "${reading.text}"`);
  if (unit !== 'ms') assert.ok(value >= 1, `${spelled}: "${reading.text}" keeps a value of 1 or more`);
  else assert.ok(value >= 1 || magnitude < 0.995 + 1e-9, `${spelled}: "${reading.text}"`);
  if (larger[unit]) assert.ok(magnitude / unitMs[larger[unit]] < 0.995 + 1e-9,
    `${spelled}: "${reading.text}" is not in ${larger[unit]}, where it would still read 1 or more`);
  assert.ok(Math.abs(value * size - magnitude) <= 0.005 * size * (1 + 1e-9) + 1e-9,
    `${spelled}: "${reading.text}" reads back as ${value * size} ms, not within 0.005 ${unit} of ${magnitude}`);
  assert.equal(reading.title, `${spelled} ms`);
  assert.equal(reading.full, spelled);
  return reading.text;
}

test('every generated duration prints in the largest unit that keeps it at 1 or more, within two decimals, with its integer in the title', () => {
  const cases = durations(4000);
  for (const ms of cases) {
    const text = check(ms, ms, String(ms));
    if (Number.isSafeInteger(ms)) {
      assert.equal(check(BigInt(ms), ms, String(ms)), text);
      assert.equal(check({ value: BigInt(ms) }, ms, String(ms)), text);
      assert.equal(check(String(ms), ms, String(ms)), text);
    }
  }
  const next = random(9007);
  for (let i = 0; i < 200; i++) {
    const big = 9007199254740993n + BigInt(Math.floor(next() * 1e9)) * 1000n + BigInt(i);
    check(big, Number(big), String(big));
  }
});

test('what is not a number is not a duration', () => {
  for (const recorded of [null, undefined, '', 'five minutes', '12 ms', {}, [], true, NaN, Infinity, { value: 5 }])
    assert.equal(formatDuration(recorded), null, String(recorded));
});
