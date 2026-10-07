import ts from 'typescript';

export const callbackSourceSegments = new WeakSet();

export function lowerCallback(callback, reject) {
  const fail = (part, reason) => reject(part, `${reason}; use a CEL string for expressions outside the supported arrow syntax`);
  if (!ts.isArrowFunction(callback) || callback.modifiers?.length || callback.typeParameters?.length
    || callback.parameters.length !== 1 || ts.isBlock(callback.body)) {
    return fail(callback, 'expected one synchronous expression arrow with one payload parameter');
  }
  const parameter = callback.parameters[0];
  if (!ts.isIdentifier(parameter.name) || parameter.dotDotDotToken || parameter.initializer || parameter.questionToken) {
    return fail(parameter, 'expected one plain payload parameter');
  }
  const quote = text => JSON.stringify(text);
  function term(part) {
    if (ts.isParenthesizedExpression(part) || ts.isAsExpression(part) || ts.isSatisfiesExpression(part)) return term(part.expression);
    if (ts.isIdentifier(part)) {
      if (part.text !== parameter.name.text) return fail(part, 'free identifiers and captured values are not expression inputs');
      return 'event';
    }
    if (ts.isStringLiteral(part) || ts.isNoSubstitutionTemplateLiteral(part)) return quote(part.text);
    if (part.kind === ts.SyntaxKind.TrueKeyword) return 'true';
    if (part.kind === ts.SyntaxKind.FalseKeyword) return 'false';
    if (part.kind === ts.SyntaxKind.NullKeyword) return 'null';
    if (ts.isNumericLiteral(part)) {
      const number = Number(part.text);
      if (!Number.isFinite(number)) return fail(part, 'a Float literal must be finite');
      const text = String(number);
      return /[.e]/i.test(text) ? text : `${text}.0`;
    }
    if (ts.isBigIntLiteral(part)) {
      const value = BigInt(part.text.slice(0, -1));
      if (value > 0x7fffffffffffffffn) return fail(part, 'an Int literal must fit signed 64 bits');
      return String(value);
    }
    if (ts.isPropertyAccessExpression(part)) {
      if (part.questionDotToken) return fail(part, 'optional access would hide a missing-field error');
      return `${term(part.expression)}[${quote(part.name.text)}]`;
    }
    if (ts.isElementAccessExpression(part)) {
      if (part.questionDotToken || !part.argumentExpression) return fail(part, 'optional access is not supported');
      const index = part.argumentExpression;
      const printed = ts.isNumericLiteral(index) && Number.isSafeInteger(Number(index.text))
        ? String(Number(index.text)) : term(index);
      return `${term(part.expression)}[${printed}]`;
    }
    if (ts.isArrayLiteralExpression(part)) return `[${part.elements.map(term).join(', ')}]`;
    if (ts.isObjectLiteralExpression(part)) {
      const keys = new Set();
      return `{${part.properties.map(property => {
        if (!ts.isPropertyAssignment(property) || ts.isComputedPropertyName(property.name)
          || !(ts.isIdentifier(property.name) || ts.isStringLiteral(property.name))) {
          return fail(property, 'object fields must have explicit, unique string keys and expression values');
        }
        const key = property.name.text;
        if (keys.has(key)) return fail(property.name, 'duplicate object key');
        keys.add(key);
        return `${quote(key)}: ${term(property.initializer)}`;
      }).join(', ')}}`;
    }
    if (ts.isPrefixUnaryExpression(part)) {
      const operator = new Map([[ts.SyntaxKind.ExclamationToken, '!'], [ts.SyntaxKind.MinusToken, '-']]).get(part.operator);
      if (!operator) return fail(part, 'unsupported unary operator');
      if (operator === '-' && ts.isBigIntLiteral(part.operand)
        && BigInt(part.operand.text.slice(0, -1)) === 0x8000000000000000n) return '-9223372036854775808';
      return `(${operator}${term(part.operand)})`;
    }
    if (ts.isBinaryExpression(part)) {
      const operator = new Map([
        [ts.SyntaxKind.PlusToken, '+'], [ts.SyntaxKind.MinusToken, '-'], [ts.SyntaxKind.AsteriskToken, '*'],
        [ts.SyntaxKind.SlashToken, '/'], [ts.SyntaxKind.PercentToken, '%'],
        [ts.SyntaxKind.LessThanToken, '<'], [ts.SyntaxKind.LessThanEqualsToken, '<='],
        [ts.SyntaxKind.GreaterThanToken, '>'], [ts.SyntaxKind.GreaterThanEqualsToken, '>='],
        [ts.SyntaxKind.EqualsEqualsEqualsToken, '=='], [ts.SyntaxKind.ExclamationEqualsEqualsToken, '!='],
        [ts.SyntaxKind.AmpersandAmpersandToken, '&&'], [ts.SyntaxKind.BarBarToken, '||'], [ts.SyntaxKind.InKeyword, 'in'],
      ]).get(part.operatorToken.kind);
      if (!operator) return fail(part.operatorToken, 'unsupported operator (no coercion, assignment, or mutation)');
      return `(${term(part.left)} ${operator} ${term(part.right)})`;
    }
    if (ts.isConditionalExpression(part)) return `(${term(part.condition)} ? ${term(part.whenTrue)} : ${term(part.whenFalse)})`;
    return fail(part, 'unsupported expression (no calls, statements, closures, or JavaScript execution)');
  }
  return term(callback.body);
}
