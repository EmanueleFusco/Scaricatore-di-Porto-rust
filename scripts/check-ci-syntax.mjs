// Verifica i blocchi `run:` della CI, senza dipendenze.
//
// Il caso che ha rotto la prima CI: dentro un blocco `run: |` i commenti `#`
// sono della SHELL, e una riga commentata che segue una continuazione `\` non
// e' un commento ma tronca il comando:
//
//     sudo apt-get install --no-install-recommends \
//       # questo e' un commento
//       libwebkit2gtk-4.1-dev \
//
// Lo shell unisce la prima riga con la seconda, il `#` commenta tutto il resto,
// quindi `apt-get` riceve zero pacchetti e `libwebkit2gk-4.1-dev` finisce
// eseguito come programma (exit 127).
//
// Nota: `bash -n` NON lo trova, perche' lo script e' sintatticamente valido.
// Per questo qui sotto c'e' una regola apposta, che guarda la continuazione
// riga per riga. `bash -n` resta come controllo secondario.
//
//   npm run check:ci
import { readFile, readdir } from "node:fs/promises";
import { spawnSync } from "node:child_process";
import { join } from "node:path";
import process from "node:process";

const RUN_START = /^(\s*)run:\s*\|\s*$/;

async function workflowFiles() {
  const dir = ".github/workflows";
  let entries;
  try {
    entries = await readdir(dir);
  } catch {
    console.error(`[sdp] ${dir} non esiste`);
    process.exit(1);
  }
  return entries
    .filter((f) => f.endsWith(".yml") || f.endsWith(".yaml"))
    .map((f) => join(dir, f));
}

/** Estrae i blocchi `run: |` (literal block) da un file YAML. */
function extractRunBlocks(lines) {
  const blocks = [];
  for (let i = 0; i < lines.length; i++) {
    const start = RUN_START.exec(lines[i]);
    if (!start) continue;
    const baseIndent = start[1].length;
    const body = [];
    let j = i + 1;
    for (; j < lines.length; j++) {
      const line = lines[j];
      if (line.trim() === "") {
        body.push("");
        continue;
      }
      const indent = line.length - line.trimStart().length;
      if (indent <= baseIndent) break;
      body.push(line);
    }
    blocks.push({ startLine: i + 1, body: body.join("\n") });
    i = j - 1;
  }
  return blocks;
}

/** Una riga finisce con una continuazione `\` non escapata? */
function continues(line) {
  const trimmed = line.trimEnd();
  if (!trimmed.endsWith("\\")) return false;
  // Conta i backslash finali: `\\` e' un backslash letterale, non una continuazione.
  let count = 0;
  for (let i = trimmed.length - 1; i >= 0 && trimmed[i] === "\\"; i--) count++;
  return count % 2 === 1;
}

/**
 * Regola mirata: dopo una continuazione `\` non può arrivare un commento.
 * Restituisce un messaggio per ogni violazione.
 */
function findCommentsInsideContinuation(body) {
  const lines = body.split("\n");
  const problems = [];
  for (let i = 0; i < lines.length - 1; i++) {
    if (!continues(lines[i])) continue;
    const next = lines[i + 1];
    if (next.trim() === "") continue; // una riga vuota non tronca, ma la seguente si
    if (next.trimStart().startsWith("#")) {
      problems.push(
        `riga ${i + 1}: una continuazione \\ e' seguita da un commento (#), che tronca il comando\n` +
          `    ${lines[i].trim()}\n` +
          `    ${next.trim()}\n` +
          `    -> sposta il commento SOPRA lo step (come commento YAML, con # a colonna 0)`,
      );
    }
  }
  return problems;
}

let failures = 0;
let checked = 0;

for (const file of await workflowFiles()) {
  const lines = (await readFile(file, "utf8")).split("\n");
  const blocks = extractRunBlocks(lines);
  if (blocks.length === 0) {
    console.log(`${file}: nessun blocco run: | da controllare`);
    continue;
  }
  let fileFailed = 0;
  for (const block of blocks) {
    checked++;

    for (const problem of findCommentsInsideContinuation(block.body)) {
      failures++;
      fileFailed++;
      console.error(`\n${file}: blocco run: | alla riga ${block.startLine}\n  ${problem}`);
    }

    const res = spawnSync("bash", ["-n"], { input: block.body, encoding: "utf8" });
    if (res.status !== 0) {
      failures++;
      fileFailed++;
      console.error(`\n${file}: blocco run: | alla riga ${block.startLine}: sintassi non valida\n${res.stderr.trim()}`);
    }
  }
  console.log(
    `${file}: ${blocks.length} blocco/i run: | controllato/i, ${fileFailed === 0 ? "ok" : `${fileFailed} problema/i`}`,
  );
}

if (checked === 0) {
  console.error("[sdp] nessun blocco run: | trovato: il controllo non ha verificato nulla");
  process.exit(1);
}

if (failures > 0) {
  console.error(`\n[ci] ${failures} problema/i: la CI fallirebbe`);
  process.exit(1);
}
console.log(`\n[ci] tutti i ${checked} blocchi run: | sono validi`);