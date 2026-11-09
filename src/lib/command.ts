export function splitCommandLine(value: string): string[] {
  const tokens: string[] = [];
  let current = "";
  let quote: "'" | '"' | null = null;
  let escaping = false;

  for (const character of value.trim()) {
    if (escaping) {
      current += character;
      escaping = false;
    } else if (character === "\\" && quote === '"') {
      escaping = true;
    } else if (quote && character === quote) {
      quote = null;
    } else if (!quote && (character === "'" || character === '"')) {
      quote = character;
    } else if (!quote && /\s/.test(character)) {
      if (current) {
        tokens.push(current);
        current = "";
      }
    } else {
      current += character;
    }
  }

  if (quote) throw new Error("Ghilimele neînchise în comandă.");
  if (escaping) current += "\\";
  if (current) tokens.push(current);
  return tokens;
}

export function processStartConfirmationAction(projectId: number, parts: string[]): string {
  if (parts.length === 0) throw new Error("Comanda dev-server este goală.");
  return `process:start:${projectId}:${parts[0]}:${JSON.stringify(parts)}`;
}

