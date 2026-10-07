import metadata from '../package.json' with { type: 'json' };
import * as core from '@circular/core';
import { constructorSpelling } from '@circular/core/internal';
import { createProviderBindingRegistry } from '@circular/specs';
export { generateProgram } from './generator.js';

export const generatorEnvironment = Object.freeze({
  sdkVersion: metadata.version,
  bindings: createProviderBindingRegistry({ bindings: Object.entries(core).flatMap(([constructorExport, value]) => {
    const actorTypeId = constructorSpelling(value);
    return actorTypeId ? [{ actorTypeId, constructorExport, importSpecifier: '@circular/core' }] : [];
  }) }),
});
