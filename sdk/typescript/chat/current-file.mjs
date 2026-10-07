/** A session file is a dated reading of the daemon fold, not the current binding at execution. */
export function currentFileSource(source, cursor) {
  const origin = cursor == null
    ? 'No authoring cursor was available when written (daemon absent, no deployment yet, or reconstruction failed)'
    : `This file reflects daemon authoring cursor ${cursor}`;
  return `// ${origin}. Later commits are absent here; programs resolve circular:current from the daemon when they run.\n`
    + (typeof source === 'string' ? source : new TextDecoder().decode(source));
}
