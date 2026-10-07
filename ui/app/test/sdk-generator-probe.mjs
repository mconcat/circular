import { generateProgram as generate, generatorEnvironment } from '../node_modules/@circular/generator/src/generator-entry.js';
export { generatorEnvironment };
await globalThis.sdkWitness.gate;
export function generateProgram(...args) {
  globalThis.sdkWitness.generations += 1;
  return generate(...args);
}
