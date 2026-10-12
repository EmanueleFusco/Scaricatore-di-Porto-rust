// Riporta le dimensioni dei bundle prodotti (spec §1 "report artifact sizes on
// every build", spec §15 acceptance #1).
//
// Cross-platform: usa solo Node, così gira identico su Linux, Windows e macOS
// senza bisogno di `du`/`stat`/`Get-ChildItem`.
//
// Conta **solo gli artefatti installabili**, non i file di staging: la cartella
// `bundle/deb/` contiene anche il contenuto spacchettato del pacchetto, e
// `bundle/appimage/` la directory estratta. Sommarle darebbe numeri senza
// senso (per l'AppImage siamo passati da 108 MiB reali a 424 MiB inventati).
import { readdir, stat } from "node:fs/promises";
import { extname, join, relative, sep } from "node:path";
import { appendFile } from "node:fs/promises";
import { exit, stdout } from "node:process";

const TARGET_DIR = "src-tauri/target";

/** Estensioni degli artefatti che l'utente scarica e installa. */
const ARTIFACT_EXT = new Set([".deb", ".rpm", ".appimage", ".exe", ".msi", ".dmg", ".snap"]);

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
  "> Obiettivo spec §1: **binario applicazione ≤ 15 MB** (verificato a mano in locale:",
  "> 4,58 MiB). L'AppImage è più grande di proposito — incorpora WebKitGTK, e la",
  "> spec dice che per l'AppImage il budget non vale. Il budget dell'installer",
  "> si fissa in M0-C, quando dentro ci saranno yt-dlp e qjs.",
].join("\n");

stdout.write(`${markdown}\n`);
if (process.env.GITHUB_STEP_SUMMARY) {
  await appendFile(process.env.GITHUB_STEP_SUMMARY, `${markdown}\n`);
}