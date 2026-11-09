export type DiffLine = {
  kind: "same" | "add" | "remove";
  text: string;
  oldLine?: number;
  newLine?: number;
};

export function createLineDiff(before: string, after: string): DiffLine[] {
  const a = before.split("\n");
  const b = after.split("\n");
  const rows = a.length + 1;
  const cols = b.length + 1;
  const lcs = Array.from({ length: rows }, () => new Uint32Array(cols));

  for (let i = a.length - 1; i >= 0; i -= 1) {
    for (let j = b.length - 1; j >= 0; j -= 1) {
      lcs[i][j] = a[i] === b[j]
        ? lcs[i + 1][j + 1] + 1
        : Math.max(lcs[i + 1][j], lcs[i][j + 1]);
    }
  }

  const result: DiffLine[] = [];
  let i = 0;
  let j = 0;
  let oldLine = 1;
  let newLine = 1;

  while (i < a.length && j < b.length) {
    if (a[i] === b[j]) {
      result.push({ kind: "same", text: a[i], oldLine, newLine });
      i += 1;
      j += 1;
      oldLine += 1;
      newLine += 1;
    } else if (lcs[i + 1][j] >= lcs[i][j + 1]) {
      result.push({ kind: "remove", text: a[i], oldLine });
      i += 1;
      oldLine += 1;
    } else {
      result.push({ kind: "add", text: b[j], newLine });
      j += 1;
      newLine += 1;
    }
  }

  while (i < a.length) {
    result.push({ kind: "remove", text: a[i], oldLine });
    i += 1;
    oldLine += 1;
  }
  while (j < b.length) {
    result.push({ kind: "add", text: b[j], newLine });
    j += 1;
    newLine += 1;
  }
  return result;
}

