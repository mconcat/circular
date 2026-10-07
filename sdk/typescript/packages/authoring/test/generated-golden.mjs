import fs from 'node:fs';

export const GENERATED_SOURCE_GOLDEN_HEADER = '// If this breaks, the generator lost an idiom; this is not a snapshot to refresh.\n';

/** The expected generator output: the golden file without its header line. */
export function generatedSourceGolden(url) {
  const text = fs.readFileSync(url, 'utf8');
  if (!text.startsWith(GENERATED_SOURCE_GOLDEN_HEADER)) throw new Error(`${url} does not start with the golden header line`);
  return text.slice(GENERATED_SOURCE_GOLDEN_HEADER.length);
}
