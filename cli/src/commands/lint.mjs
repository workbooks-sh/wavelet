// wavelet lint <file.html> — structural validation.
//
// Reports every finding it can find (does not stop at the first error).
// Exit code: 0 if no errors, 1 if any errors. Warnings never fail.

import "../dom.mjs";
import { readFile, access } from "node:fs/promises";
import { resolve, dirname, isAbsolute } from "node:path";
import { parseDocument } from "@work.books/wavelet-runtime/parser";
import { lintDocument, summariseFindings } from "@work.books/wavelet-runtime/lint";

export async function lint(args) {
  const file = args[0];
  if (!file) {
    console.error("wavelet lint: missing file argument");
    return 1;
  }
  const abs = resolve(process.cwd(), file);
  let html;
  try {
    html = await readFile(abs, "utf8");
  } catch (e) {
    console.error(`wavelet lint: cannot read ${abs}: ${e.message}`);
    return 1;
  }

  let doc;
  try {
    doc = parseDocument(html);
  } catch (e) {
    console.error(`wavelet lint: parse failed`);
    console.error(`  ${e.message}`);
    return 1;
  }

  const baseDir = dirname(abs);
  const findings = await lintDocument(doc, {
    fileExists: async (relPath) => {
      const target = isAbsolute(relPath) ? relPath : resolve(baseDir, relPath);
      try {
        await access(target);
        return true;
      } catch {
        return false;
      }
    },
  });

  if (findings.length === 0) {
    console.log(`${file}: clean — no findings`);
    return 0;
  }

  // Sort errors first, then warnings; stable within severity.
  const sorted = [...findings].sort((a, b) => {
    if (a.severity === b.severity) return 0;
    return a.severity === "error" ? -1 : 1;
  });

  for (const f of sorted) {
    const tag = f.severity === "error" ? "error " : "warn  ";
    const at = f.at ? `  (${f.at})` : "";
    console.log(`  ${tag} [${f.code}] ${f.message}${at}`);
  }

  const { errors, warnings } = summariseFindings(findings);
  console.log("");
  console.log(`${file}: ${errors} error${plural(errors)}, ${warnings} warning${plural(warnings)}`);
  return errors > 0 ? 1 : 0;
}

function plural(n) {
  return n === 1 ? "" : "s";
}
