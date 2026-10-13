// Riporta le dimensioni dei bundle prodotti (spec §1 "report artifact sizes on
// every build", spec §15 acceptance #1).
//
// Cross-platform: usa solo Node, così gira identico su Linux, Windows e macOS
// senza bisogno di `du`/`stat`/`Get-ChildItem`.
//
// conta solo gli artefatti installabili, non i file di staging: la cartella
// `bundle/deb/` contiene anche il contenuto spacchettato del pacchetto, e
// `bundle/appimage/` la directory estratta. Sommarle darebbe numeri senza
// senso (per l'AppImage siamo passati da 108 MiB reali a 424 MiB inventati).
//
// Due cose che questo script fa oltre a contare:
//
//   1. **misura** il binario dell'applicazione invece di stampare un numero
//      scritto a mano. Stampare "4,58 MiB" su runner Linux, Windows e macOS
//      è un'affermazione non verificata: spec §1 chiede di *misurare* e
//      riportare ad ogni build;
//   2. **fallisce** se il falso tool di test (`fake_tool`) è finito dentro un
//      bundle: il bundler Linux installa *ogni* `[[bin]]` del pacchetto in
//      `/usr/bin`, quindi il helper dei test finiva nell'installer (D19).
import { readdir, readFile, stat } from "node:fs/promises";
import { extname, join, relative, sep } from "node:path";
import { appendFile } from "node:fs/promises";
import { exit, stdout } from "node:process";

// Il bundle universale macOS vive in `target/universal-apple-darwin/`, non in
// `target/`: chi ci chiama lo dice con SDP_TARGET_DIR.
const TARGET_DIR = process.env.SDP_TARGET_DIR ?? "src-tauri/target";

/** Estensioni degli artefatti che l'utente scarica e installa. */
const ARTIFACT_EXT = new Set([".deb", ".rpm", ".appimage", ".exe", ".msi", ".dmg", ".snap"]);

/** Obiettivo di peso della spec §1 per il binario applicazione. */
const APP_BINARY_LIMIT_BYTES = 15 * 1024 * 1024;

/** Il falso tool dei test: non deve MAI arrivare a un utente. */
const FORBIDDEN = "fake_tool";

function human(bytes) {
  const mb = bytes / 1024 / 1024;
  return mb >= 1 ? `${mb.toFixed(1)} MiB` : `${(bytes / 1024).toFixed(0)} KiB`;
}

async function walk(dir) {
  let entries;
  try {
    entries = await readdir(dir, { withFileTypes: true });
  } catch {
    return [];
  }
  const out = [];
  for (const entry of entries) {
    const full = join(dir, entry.name);
    if (entry.isDirectory()) out.push(...(await walk(full)));
    else out.push(full);
  }
  return out;
}

/** Dimensione di un file, o somma dei file se è una directory (es. `.app`). */
async function sizeOf(path) {
  const info = await stat(path).catch(() => null);
  if (!info) return 0;
  if (info.isFile()) return info.size;
  let total = 0;
  for (const child of await walk(path)) {
    total += (await stat(child).catch(() => ({ size: 0 }))).size;
  }
  return total;
}

const bundleRoot = join(TARGET_DIR, "release", "bundle");

/** Ricerca gli artefatti: file per estensione, oppure directory `.app`. */
async function findArtifacts(dir) {
  const found = [];
  let entries;
  try {
    entries = await readdir(dir, { withFileTypes: true });
  } catch {
    return found;
  }
  for (const entry of entries) {
    const full = join(dir, entry.name);
    if (entry.isDirectory()) {
      if (extname(entry.name).toLowerCase() === ".app") found.push(full);
      else found.push(...(await findArtifacts(full)));
    } else if (ARTIFACT_EXT.has(extname(entry.name).toLowerCase())) {
      found.push(full);
    }
  }
  return found;
}

const artifacts = await findArtifacts(bundleRoot);

if (artifacts.length === 0) {
  console.error("[sdp] nessun artefatto trovato in", bundleRoot);
  exit(1);
}

// Un bundle universale produce piu' slice: raggruppati per cartella (=formato).
const byFormat = new Map();
for (const file of artifacts) {
  const top = relative(bundleRoot, file).split(sep)[0];
  if (!byFormat.has(top)) byFormat.set(top, []);
  byFormat.get(top).push(file);
}

const rows = [];
for (const [format, members] of [...byFormat].sort()) {
  const total = (await Promise.all(members.map(sizeOf))).reduce((a, b) => a + b, 0);
  rows.push({
    format,
    files: members.map((m) => relative(bundleRoot, m)).sort(),
    size: total,
  });
}
rows.sort((a, b) => b.size - a.size);

// ---------------------------------------------------------------------------
// Il binario applicazione: MISURATO, non dichiarato
// ---------------------------------------------------------------------------

/**
 * Nome del binario applicativo, letto da `Cargo.toml` invece di indovinato:
 * il binario prende il nome del *pacchetto* (`scaricatore-di-porto`), non il
 * `productName` di `tauri.conf.json` (`Scaricatore Di Porto`).
 */
async function appBinaryName() {
  const cargo = await readFile(join(TARGET_DIR, "..", "Cargo.toml"), "utf8").catch(() => "");
  return /^name\s*=\s*"([^"]+)"/m.exec(cargo)?.[1] ?? null;
}

async function appBinarySize() {
  const name = await appBinaryName();
  if (!name) return null;
  const candidates = process.platform === "win32" ? [`${name}.exe`, name] : [name, `${name}.exe`];
  for (const candidate of candidates) {
    const info = await stat(join(TARGET_DIR, "release", candidate)).catch(() => null);
    if (info?.isFile()) return { name: candidate, size: info.size };
  }
  return null;
}

// ---------------------------------------------------------------------------
// Il falso tool dei test non deve mai essere dentro un bundle
// ---------------------------------------------------------------------------

/**
 * Cerca ricorsivamente in tutta la cartella `bundle/`, **inclusa la staging
 * spacchettata**: e' li' che si vede il bug di D19 (`usr/bin/fake_tool` dentro
 * il .deb), ed e' l'unico posto dove si vede su tutte le piattaforme senza
 * dover scompattare ogni formato (.deb, .rpm, .AppImage, NSIS, .dmg).
 */
async function findForbidden(dir) {
  const hits = [];
  for (const file of await walk(dir)) {
    const base = (file.split(/[\\/]/).pop() ?? "").toLowerCase();
    if (base === FORBIDDEN || base.startsWith(`${FORBIDDEN}.`) || base.startsWith(`${FORBIDDEN}-`)) {
      hits.push(relative(process.cwd(), file));
    }
  }
  return hits;
}

const forbidden = await findForbidden(bundleRoot);
if (forbidden.length > 0) {
  console.error(
    `[sdp] BLOCCO: il falso tool di test e' dentro il bundle e verrebbe installato:\n` +
      forbidden.map((f) => `        ${f}`).join("\n") +
      `\n  E' dichiarato come \`[[example]]\` in src-tauri/Cargo.toml proprio perche' il` +
      `\n  bundler installa ogni \`[[bin]]\` del pacchetto in /usr/bin (DECISIONS.md D19).` +
      `\n  Se questo file e' qui, qualcuno l'ha dichiarato \`[[bin]]\`: rimetterlo come` +
      `\n  \`[[example]]\`.`,
  );
  exit(1);
}

const appBinary = await appBinarySize();

const markdown = [
  "### Dimensioni dei bundle",
  "",
  "| Formato | File | Dimensione |",
  "| :-- | :-- | --: |",
  ...rows.map(
    (r) =>
      `| ${r.format} | ${r.files.map((f) => `\`${f.split(sep).slice(1).join("/")}\``).join("<br>")} | ${human(r.size)} |`,
  ),
  "",
  appBinary
    ? `> Binario applicazione **misurato su questo runner**: \`${appBinary.name}\` = **${human(appBinary.size)}** (obiettivo spec §1: ≤ ${human(APP_BINARY_LIMIT_BYTES)}).`
    : "> Binario applicazione: non trovato in `target/release/`, nessuna misura possibile.",
  "> L'AppImage è più grande di proposito — incorpora WebKitGTK, e la spec §1 dice",
  "> che per l'AppImage il budget non vale.",
].join("\n");

stdout.write(`${markdown}\n`);
if (process.env.GITHUB_STEP_SUMMARY) {
  await appendFile(process.env.GITHUB_STEP_SUMMARY, `${markdown}\n`);
}