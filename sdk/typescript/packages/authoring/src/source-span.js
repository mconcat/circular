import { SourceMap } from 'node:module';
import { callbackSourceSegments } from './callback-expression.js';

function positionLessThanOrEqual(left, right) {
  return left.line < right.line || (left.line === right.line && left.column <= right.column);
}

function spanPosition(span, edge) {
  return edge === "start"
    ? { line: span.startLine, column: span.startColumn }
    : { line: span.endLine, column: span.endColumn };
}

/**
 * Maps exact generated ranges and identity-shaped subranges conservatively.
 * A transform that changes column geometry must emit finer-grained source-map segments.
 */
export function mapAuthoringSourceSpan(sourceMap, generated) {
  for (const segment of sourceMap.segments) {
    if (segment.generated.source !== generated.source) continue;
    const contains = positionLessThanOrEqual(
      spanPosition(segment.generated, "start"),
      spanPosition(generated, "start"),
    ) && positionLessThanOrEqual(
      spanPosition(generated, "end"),
      spanPosition(segment.generated, "end"),
    );
    if (!contains) continue;
    if (callbackSourceSegments.has(segment)) return segment.original;

    const lineOffsetStart = generated.startLine - segment.generated.startLine;
    const lineOffsetEnd = generated.endLine - segment.generated.startLine;
    const startColumn = lineOffsetStart === 0
      ? segment.original.startColumn + generated.startColumn - segment.generated.startColumn
      : generated.startColumn;
    const endColumn = lineOffsetEnd === 0
      ? segment.original.startColumn + generated.endColumn - segment.generated.startColumn
      : generated.endColumn;

    return Object.freeze({
      source: segment.original.source,
      startLine: segment.original.startLine + lineOffsetStart,
      startColumn,
      endLine: segment.original.startLine + lineOffsetEnd,
      endColumn,
    });
  }
  return null;
}

/** The innermost stack frame the engine recorded inside one evaluated module, 1-based; null when none. */
function framePosition(stack, filename) {
  const needle = `${filename}:`;
  for (const line of typeof stack === 'string' ? stack.split('\n') : []) {
    if (!/^\s+at /.test(line)) continue;
    const at = line.indexOf(needle);
    if (at < 1 || (line[at - 1] !== ' ' && line[at - 1] !== '(')) continue;
    const position = /^(\d+):(\d+)/.exec(line.slice(at + needle.length));
    if (position) return { line: Number(position[1]), column: Number(position[2]) };
  }
  return null;
}

export function thrownSourceSpan(error, { filename, path, transpiledSourceMap, sourceMap }) {
  const frame = framePosition(error?.stack, filename);
  if (frame === null) return null;
  const entry = new SourceMap(JSON.parse(transpiledSourceMap)).findEntry(frame.line - 1, frame.column - 1);
  if (!Number.isSafeInteger(entry?.originalLine) || !Number.isSafeInteger(entry?.originalColumn)) return null;
  const line = entry.originalLine + 1, column = entry.originalColumn + 1;
  return mapAuthoringSourceSpan(sourceMap, { source: path, startLine: line, startColumn: column, endLine: line, endColumn: column });
}
