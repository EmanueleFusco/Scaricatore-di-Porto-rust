// Controlla che il tag della release corrisponda alla versione dei manifest.
//
// Perche': la versione e' scritta in TRE posti indipendenti
// (src-tauri/tauri.conf.json, src-tauri/Cargo.toml, package.json) e finisce nel
// NOME dei file prodotti. Senza questo controllo si puo' pubblicare una release
// `v0.2.0` con dentro `Scaricatore Di Porto_0.1.0_amd64.deb`: binari sbagliati
// con etichetta sbagliata, e su una release pubblica non si corregge.
//
//   node scripts/check-release-version.mjs v0.2.0
//   npm run check:release -- v0.2.0
//
// Exit 0 = tutto coerente. Exit 1 = mismatch, con l'elenco delle differenze.
import { readFile } from "node:fs/promises";
import process from "node:process";

const tag = process.argv[2];

if (!tag) {
  console.error("uso: node scripts/check-release-version.mjs <tag>   (es. v0.2.0)");
  process.exit(2);
}

const expected = tag.startsWith("v") ? tag.slice(1) : tag;

async function tauriVersion() {
  const raw = await readFile("src-tauri/tauri.conf.json", "utf8");
  return JSON.parse(raw).version;
}

async function cargoVersion() {
  const raw = await readFile("src-tauri/Cargo.toml", "utf8");
  // `version = "0.1.0"` all'inizio di riga; evita di prendere le dipendenze.
  const match = raw.match(/^version\s*=\s*"([^"]+)"/m);
  if (!match) throw new Error("versione non trovata in src-tauri/Cargo.toml");
  return match[1];
}

async function packageVersion() {
  const raw = await readFile("package.json", "utf8");
  return JSON.parse(raw).version;
}

const [tauri, cargo, pkg] = await Promise.all([
  tauriVersion(),
  cargoVersion(),
  packageVersion(),
]);

const sources = [
  { file: "src-tauri/tauri.conf.json", value: tauri },
  { file: "src-tauri/Cargo.toml", value: cargo },
  { file: "package.json", value: pkg },
];

const problems = [];

for (const { file, value } of sources) {
  if (value !== expected) {
    problems.push(`  ${file}: "${value}" ma il tag dice "${expected}"`);
  }
}

// I tre manifest devono concordare fra loro, indipendentemente dal tag.
const distinct = [...new Set(sources.map((s) => s.value))];
if (distinct.length > 1) {
  problems.push(
    `  i manifest non concordano fra loro: ${sources.map((s) => `${s.file}="${s.value}"`).join(", ")}`,
  );
}

if (problems.length > 0) {
  console.error(`\nRelease bloccata: il tag ${tag} non e' coerente con i manifest\n`);
  for (const p of problems) console.error(p);
  console.error(
    "\nCorreggi i tre manifest, oppure il tag. Poi:\n" +
      "  git add -A && git commit -m 'build: versione 0.x.y'\n" +
      `  git tag ${tag} && git push origin ${tag}\n`,
  );
  process.exit(1);
}

console.log(`[ci] versione coerente: ${expected} in tutti e tre i manifest`);