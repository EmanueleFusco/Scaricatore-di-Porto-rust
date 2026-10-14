#!/usr/bin/env node
// Verifica che la CI non usi immagini deprecate e azioni su Node 20.
//
// Perche' un controllo e non un commento
// =====================================
// Il 2026-10-13 la CI ha segnalato due warning: le azioni dichiarano Node 20
// (rimosso dai runner il 23 settembre 2026) e `ubuntu-latest` sarebbe migrato a
// Ubuntu 26.04. Nessuno dei due era sbagliato: erano **preannunci**. Il problema
// non e' che non si sapesse, e' che nel workflow non c'era nulla che rendesse
// visibile il momento in cui la previsione si avvera.
//
// `ubuntu-latest` e' il caso peggiore: e' un'etichetta che **migra da sola**.
// Il 19 ottobre 2026 e' diventata Ubuntu 26.04 e un file che ieri compilava su
// una base e oggi su un'altra non ha cambiato di una riga. Non si puo' rimediare
// a posteriori: o lo prevedi, o lo scopri da un build rosso.
//
// Quindi: un **allowlist**. Ogni immagine e ogni azione che usiamo deve essere
// scritta qui dentro, e quello che non c'e' fa fallire il controllo. Cosi' la
// prossima migrazione la decidiamo noi, nel momento che ci pare, invece che
// scoprirla il giorno in cui un runner cambia sotto i nostri piedi.
//
// E il controllo gira a ogni push: `npm run check:runners`.
//
// Uso:
//   node scripts/check-ci-runners.mjs
import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import process from "node:process";

const WORKFLOW_DIR = ".github/workflows";

/**
 * Le immagini su cui costruiamo. Un'immagine mancante qui e' un errore, non
 * un avviso: l'allowlist e' il punto.
 *
 * Spec 1.1 piu' il job tier2, che gira su macOS Intel, Windows ARM64 e Linux
 * arm64.
 */
const SUPPORTED_IMAGES = new Set([
  "ubuntu-24.04",
  "ubuntu-24.04-arm",
  "macos-26", // arm64: `macos-latest` e `macos-26` sono la stessa immagine
  "macos-15-intel",
  "windows-2022",
  "windows-11-arm",
]);

/**
 * Etichette che esistono ma non sono immagini fisse: migrano quando GitHub
 * pubblica la prossima versione. Il nome dice "l'ultima", e l'ultima cambia.
 */
const FLOATING_LABELS = {
  "ubuntu-latest":
    "migra da sola: il 19 ottobre 2026 e' passata a Ubuntu 26.04, in una " +
    "finestra di un mese, quindi per tutto ottobre-novembre la stessa run " +
    "poteva capitare su una o sull'altra immagine. Scrivi l'immagine esplicita.",
  "macos-latest":
    "migra da sola alla prossima macOS. Oggi e' macOS 26 arm64, cioe' lo " +
    "stesso di `macos-26`: usare `macos-latest` non compra nulla.",
  "windows-latest":
    "migra da sola: oggi e' Windows Server 2025 con Visual Studio 2026, che " +
    "e' anche il motivo per cui `bundle` e `process-tests` usano windows-2022.",
};

/**
 * Immagini che funzionano ancora ma sono in deprecazione dichiarata.
 *
 * La fonte, da verificare se una voce sembra vecchia:
 *   actions/runner-images#14254 — ubuntu-22.04
 *   actions/runner-images#13518 — macOS 14
 */
const RETIRED_IMAGES = {
  "ubuntu-22.04":
    "deprecata dal 17 settembre 2026; GitHub FARA' FALLIRE i job in quattro " +
    "brownout (23 e 30 marzo, 6 e 13 aprile 2027) e sara' del tutto non " +
    "supportata il 17 aprile 2027. Usa ubuntu-24.04.",
  "ubuntu-22.04-arm": "come ubuntu-22.04. Usa ubuntu-24.04-arm.",
  "macos-14": "immagine ritirata. Usa macos-26 o macos-15-intel.",
  "macos-14-arm": "immagine ritirata. Usa macos-26.",
  "macos-14-arm64": "immagine ritirata. Usa macos-26.",
  "macos-14-large": "immagine ritirata.",
};

/**
 * Le azioni che usiamo, con la **prima versione che gira su Node 24**.
 *
 * Non e' lo stesso major per tutte, e questa tabella non si indovina: Node 20 e'
 * stato rimosso il 23 settembre 2026 e ogni azione ha fatto il salto quando ha
 * voluto. Verificato il 2026-10-13 leggendo `action.yml` a ogni tag:
 *
 *   actions/checkout           v4.4.0 node20 | v5+ node24
 *   actions/setup-node         v4.4.0 node20 | v5+ node24
 *   actions/upload-artifact    v4.6.2 node20 | v5 node20 | v6+ node24
 *   actions/download-artifact  v4.3.0 node20 | v5 node20 | v6 node20 | v7+ node24
 *   Swatinem/rust-cache                                        v2.9.2 node24
 *
 * Cioe' "alza tutto a v7" sarebbe stato sbagliato due volte: troppo per tre di
 * loro, troppo poco per nessuna. E `download-artifact` e' a **v8**: chi aggiorna
 * a caso lascia indietro proprio quella.
 */
const KNOWN_ACTIONS = {
  "actions/checkout": { min: [5, 0, 0], note: "v4 e' su Node 20, rimosso il 2026-09-23" },
  "actions/setup-node": { min: [5, 0, 0], note: "v4 e' su Node 20" },
  "actions/upload-artifact": { min: [6, 0, 0], note: "v4 e v5 sono su Node 20" },
  "actions/download-artifact": { min: [7, 0, 0], note: "v4, v5 e v6 sono su Node 20" },
  "Swatinem/rust-cache": { min: [2, 9, 2], note: "solo da 2.9.2 gira su Node 24" },
  // Composita: gira negli shell, non in un runtime JavaScript, quindi la
  // rimozione di Node 20 non la riguarda. Il tag mobile qui E' VOLUTO: stable,
  // nightly o una versione precisa sono l'interfaccia dell'azione, ed e' il
  // motivo per cui e' l'unica esenzione dichiarata alla regola "il ref non e'
  // una versione". Senza questa riga la guardia la bloccherebbe a ogni push.
  "dtolnay/rust-toolchain": {
    floating: true,
    note:
      "composite: nessun runtime JS. Il tag mobile e' voluto (stable, nightly, o una versione precisa).",
  },
};

/** `@v7`, `@v7.1.0`, `@v7.1` -> [7, 1, 0]. `null` se non e' una versione. */
function parseVersion(ref) {
  const match = /^v?(\d+)(?:\.(\d+))?(?:\.(\d+))?$/.exec(ref);
  if (!match) return null;
  return [1, 2, 3].map((i) => Number(match[i] ?? 0));
}

function atLeast(a, b) {
  for (let i = 0; i < 3; i += 1) {
    if (a[i] !== b[i]) return a[i] > b[i];
  }
  return true;
}

function workflowFiles() {
  return readdirSync(WORKFLOW_DIR)
    .filter((f) => f.endsWith(".yml") || f.endsWith(".yaml"))
    .map((f) => join(WORKFLOW_DIR, f));
}

/**
 * Ogni etichetta di runner usata da un file: sia il `runs-on:` diretto sia
 * quelle della matrice, che e' dove finiscono quando il job ne ha piu' d'una.
 */
function runnerLabels(text) {
  const found = [];
  const runsOn = /^\s*runs-on:\s*(\S+)\s*$/gm;
  const matrixOs = /^\s*-?\s*os:\s*(\S+)\s*$/gm;
  for (const [source, re, skipExpressions] of [
    ["runs-on", runsOn, true],
    ["matrix os", matrixOs, false],
  ]) {
    let m;
    while ((m = re.exec(text)) !== null) {
      // `runs-on: ${{ matrix.os }}` e' un riferimento: l'immagine vera sta
      // nella matrice, che viene raccolta dall'altra regex.
      if (skipExpressions && m[1].includes("{{")) continue;
      found.push({ source, value: m[1].replace(/^["']|["']$/g, "") });
    }
  }
  return found;
}

function actions(text) {
  const found = [];
  const re = /^\s*(?:- )?uses:\s*["']?([\w.-]+\/[\w.-]+)@([\w.-]+)["']?\s*$/gm;
  let m;
  while ((m = re.exec(text)) !== null) found.push({ name: m[1], ref: m[2] });
  return found;
}

function checkFile(file) {
  const text = readFileSync(file, "utf8");
  const problems = [];
  const line = (re, find) => {
    const lines = text.split("\n");
    for (let i = 0; i < lines.length; i += 1) if (re.test(lines[i])) return i + 1;
    return find ? 0 : undefined;
  };

  for (const { source, value } of runnerLabels(text)) {
    if (FLOATING_LABELS[value]) {
      problems.push(`${value} (${source}): ${FLOATING_LABELS[value]}`);
    } else if (RETIRED_IMAGES[value]) {
      problems.push(`${value} (${source}): ${RETIRED_IMAGES[value]}`);
    } else if (!SUPPORTED_IMAGES.has(value)) {
      problems.push(
        `${value} (${source}): non e' nell'allowlist SUPPORTED_IMAGES di ` +
          `check-ci-runners.mjs. Se e' un'immagine valida e vuoi usarla, ` +
          `aggiungila la' con una riga che dice perche' — non togliere questo ` +
          `controllo.`,
      );
    }
  }

  for (const { name, ref } of actions(text)) {
    const known = KNOWN_ACTIONS[name];
    if (!known) {
      const isFirstParty = name.startsWith("actions/");
      const hint = isFirstParty
        ? " Le azioni actions/* hanno fatto il salto a Node 24 in major diversi l'uno dall'altro: leggi action.yml al tag che usi e registra la prima versione su Node 24."
        : " Le azioni di terze parti cambiano runtime quando vogliono: leggi action.yml al tag che usi e registra la versione.";
      problems.push(`${name}@${ref}: azione non registrata in KNOWN_ACTIONS.${hint}`);
      continue;
    }
    if (known.floating) continue;
    const version = parseVersion(ref);
    if (version === null) {
      // Un ref per SHA o un branch: riproducibile, ma invisibile a questo
      // controllo e alle persone. Non e' un errore, e' un promemoria.
      problems.push(
        `${name}@${ref}: il ref non e' una versione, quindi questo controllo ` +
          `non puo' dire su quale Node gira. Se e' voluto, aggiungila a ` +
          `KNOWN_ACTIONS con min: null e spiega perche'.`,
      );
      continue;
    }
    if (!atLeast(version, known.min)) {
      problems.push(
        `${name}@${ref}: gira su Node 20 (${known.note}). Da ${known.min.join(".")} in poi gira su Node 24.`,
      );
    }
  }
  return problems;
}

function main() {
  const files = workflowFiles();
  if (files.length === 0) {
    console.error(`[runner] ${WORKFLOW_DIR} non esiste`);
    process.exit(1);
  }
  let problems = [];
  for (const file of files) problems = problems.concat(checkFile(file).map((p) => `${file}: ${p}`));
  if (problems.length === 0) {
    console.log(
      `[runner] ${files.length} workflow: nessuna immagine fluttuante, nessuna ` +
        `deprecata, tutte le azioni su Node 24`,
    );
    return;
  }
  for (const p of problems) console.error(`  - ${p}`);
  console.error(`\n[runner] ${problems.length} problema/i: la CI e' a rischio`);
  process.exit(1);
}

// Esportato per i test: `main()` gira solo quando il file e' eseguito diretto.
export { actions as usedActions, runnerLabels, parseVersion, checkFile, KNOWN_ACTIONS, SUPPORTED_IMAGES };

if (process.argv[1] && process.argv[1].endsWith("check-ci-runners.mjs")) main();
