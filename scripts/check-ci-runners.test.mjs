// Test della guardia su immagini e azioni.
//
// Scritti subito, e non "poi se serve", perche' questa guardia sostituisce un
// commento. Un commento sbagliato non rompe niente; una guardia sbagliata blocca
// la CI a ogni push, o peggio lascia passare quello che doveva prendere. Le due
// cose vanno provate: che prenda quello che deve, e che lasci passare il resto.
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { KNOWN_ACTIONS, SUPPORTED_IMAGES, parseVersion, usedActions, runnerLabels, checkFile } from "./check-ci-runners.mjs";

let dir;
beforeEach(() => {
  dir = mkdtempSync(join(tmpdir(), "sdp-runners-"));
  mkdirSync(join(dir, ".github", "workflows"), { recursive: true });
});
afterEach(() => rmSync(dir, { recursive: true, force: true }));

function workflow(text) {
  const f = join(dir, ".github", "workflows", "ci.yml");
  writeFileSync(f, text);
  return f;
}

const ok = `
jobs:
  check:
    runs-on: ubuntu-24.04
    steps:
      - uses: actions/checkout@v7
      - uses: actions/setup-node@v7
      - uses: Swatinem/rust-cache@v2.9.2
      - uses: dtolnay/rust-toolchain@stable
  bundle:
    strategy:
      matrix:
        include:
          - os: ubuntu-24.04
          - os: windows-2022
          - os: macos-26
    runs-on: \${{ matrix.os }}
    steps:
      - uses: actions/upload-artifact@v7
      - uses: actions/download-artifact@v8
`;

describe("le versioni", () => {
  it("legge un tag nel formato che i tre casi reali usano", () => {
    expect(parseVersion("v7")).toEqual([7, 0, 0]);
    expect(parseVersion("v7.1.0")).toEqual([7, 1, 0]);
    expect(parseVersion("v2.9.2")).toEqual([2, 9, 2]);
    expect(parseVersion("7")).toEqual([7, 0, 0]);
  });

  it("non inventa una versione dove non c'e'", () => {
    for (const ref of ["main", "master", "abc", "v", "a1b2c3d4", ""]) {
      expect(parseVersion(ref)).toBeNull();
    }
  });

  it("l'allowlist e' quello che abbiamo detto, e nient'altro", () => {
    // Se una immagine sparisce da qui, la CI fallisce: e' voluto.
    for (const image of ["ubuntu-24.04", "ubuntu-24.04-arm", "macos-26", "macos-15-intel", "windows-2022", "windows-11-arm"]) {
      expect(SUPPORTED_IMAGES.has(image)).toBe(true);
    }
    for (const image of ["ubuntu-22.04", "ubuntu-latest", "macos-latest", "ubuntu-26.04"]) {
      expect(SUPPORTED_IMAGES.has(image)).toBe(false);
    }
  });

  it("i confini Node 24 sono quelli verificati, non una maggioranza", () => {
    // La ragione per cui questo test esiste: il confine NON e' lo stesso major
    // per tutte. checkout e setup-node passano a Node 24 da v5;
    // upload-artifact solo da v6; download-artifact solo da v7. Scrivere
    // "da v5 per tutte" avrebbe lasciato passare due azioni su Node 20.
    expect(KNOWN_ACTIONS["actions/checkout"].min).toEqual([5, 0, 0]);
    expect(KNOWN_ACTIONS["actions/setup-node"].min).toEqual([5, 0, 0]);
    expect(KNOWN_ACTIONS["actions/upload-artifact"].min).toEqual([6, 0, 0]);
    expect(KNOWN_ACTIONS["actions/download-artifact"].min).toEqual([7, 0, 0]);
    expect(KNOWN_ACTIONS["Swatinem/rust-cache"].min).toEqual([2, 9, 2]);
  });
});

describe("immagini dei runner", () => {
  it("lascia passare la configurazione che spediamo", () => {
    expect(checkFile(workflow(ok))).toEqual([]);
  });

  it("prende un'etichetta fluttuante, che e' il modo in cui ci si rovina da soli", () => {
    // ubuntu-latest e' diventata Ubuntu 26.04 il 19 ottobre 2026 senza che
    // nessuno toccasse il file.
    const problems = checkFile(workflow(ok.replace("ubuntu-24.04", "ubuntu-latest")));
    expect(problems.join("\n")).toContain("ubuntu-latest");
    expect(problems.join("\n")).toContain("migra da sola");
  });

  it("prende anche macos-latest, nello stesso job del resto", () => {
    const problems = checkFile(workflow(ok.replace("macos-26", "macos-latest")));
    expect(problems.join("\n")).toContain("macos-latest");
  });

  it("prende un'immagine in deprecazione, con la sua scadenza", () => {
    // ubuntu-22.04: brownout in marzo-aprile 2027, spenta il 17 aprile 2027.
    const problems = checkFile(workflow(ok.replace("ubuntu-24.04", "ubuntu-22.04")));
    const joined = problems.join("\n");
    expect(joined).toContain("ubuntu-22.04");
    expect(joined).toContain("17 aprile 2027");
    expect(joined).toContain("brownout");
  });

  it("prende macos-14, ritirata", () => {
    const problems = checkFile(workflow(ok.replace("macos-26", "macos-14")));
    expect(problems.join("\n")).toContain("ritirata");
  });

  it("un'immagine sconosciuta e' un errore, non un silenzio", () => {
    // Ubuntu 26.04 esiste ed e' valida: se un giorno la vogliamo, qualcuno la
    // aggiunge all'allowlist avendo deciso di misurarla. Fino ad allora la CI
    // rifiuta.
    const problems = checkFile(workflow(ok.replace("ubuntu-24.04", "ubuntu-26.04")));
    expect(problems.join("\n")).toContain("ubuntu-26.04");
    expect(problems.join("\n")).toContain("non e' nell'allowlist");
  });

  it("non conta due volte la stessa immagine dichiarata in modi diversi", () => {
    // `- os:` e `runs-on:` possono nominare la stessa immagine; quello che conta
    // e' che un problema sia segnalato una volta, non che il numero sia esatto.
    const problems = checkFile(workflow(ok.replace("runs-on: ubuntu-24.04", "runs-on: ubuntu-22.04")));
    expect(problems.length).toBeGreaterThanOrEqual(1);
    expect(problems.length).toBeLessThanOrEqual(2);
  });
});

describe("azioni", () => {
  it("lascia passare le versioni che usiamo", () => {
    expect(checkFile(workflow(ok))).toEqual([]);
  });

  it("prende un'azione su Node 20 e dice dove sta il confine", () => {
    const problems = checkFile(workflow(ok.replace("actions/checkout@v7", "actions/checkout@v4")));
    const joined = problems.join("\n");
    expect(joined).toContain("actions/checkout@v4");
    expect(joined).toContain("Node 20");
  });

  it("il confine non e' lo stesso per tutte, e lo verifica una per una", () => {
    // La trappola di questa migrazione: v5 va bene per checkout e setup-node, e
    // NON va bene per upload-artifact e download-artifact.
    const good = checkFile(
      workflow(ok.replace("actions/upload-artifact@v7", "actions/upload-artifact@v5")),
    );
    expect(good.join("\n")).toContain("Node 20");

    const good2 = checkFile(
      workflow(ok.replace("actions/download-artifact@v8", "actions/download-artifact@v6")),
    );
    expect(good2.join("\n")).toContain("Node 20");
  });

  it("la stessa major va bene per un'azione e non per un'altra", () => {
    // La trappola di questa migrazione, in forma di test. `@v5` va bene per
    // checkout e setup-node, ed e' su Node 20 per upload-artifact. Chi pensa
    // "il confine e' v5 per tutte" lascia passare un'azione morta — e chi pensa
    // "v8 per simmetria" fissa un numero inventato.
    expect(
      checkFile(workflow(ok.replace("actions/checkout@v7", "actions/checkout@v5"))),
    ).toEqual([]);
    expect(
      checkFile(workflow(ok.replace("actions/upload-artifact@v7", "actions/upload-artifact@v5"))).join("\n"),
    ).toContain("Node 20");
  });

  it("un'azione non registrata blocca, perche' nessuno l'ha verificata", () => {
    const problems = checkFile(workflow(ok + "\n      - uses: actions/some-new-thing@v1\n"));
    const joined = problems.join("\n");
    expect(joined).toContain("some-new-thing");
    expect(joined).toContain("non registrata");
  });

  it("rust-cache e' su Node 24 e non va toccata a caso", () => {
    // Il warning della CI non la citava, e ora si sa perche': da 2.9.2 gira su
    // Node 24. Un "aggiorniamola che e' vecchia" a v1 l'avrebbe rotta.
    expect(checkFile(workflow(ok))).toEqual([]);
    // `@v2` senza patch punta oggi a 2.9.2, ma il controllo non puo' saperlo:
    // per quello la versione e' scritta esplicita nel workflow.
    expect(checkFile(workflow(ok.replace("rust-cache@v2.9.2", "rust-cache@v2"))).join("\n")).toContain("Node 20");
    const old = checkFile(workflow(ok.replace("rust-cache@v2.9.2", "rust-cache@v2.8.0")));
    expect(old.join("\n")).toContain("Node 20");
  });

  it("un'azione composite con tag mobile non ha un runtime JS da controllare", () => {
    // dtolnay/rust-toolchain gira negli shell (`runs.using: composite`), quindi
    // la rimozione di Node 20 non la tocca, e il suo tag mobile (stable,
    // nightly, o una versione) e' la sua interfaccia. E' l'unica esenzione
    // dichiarata: senza, la guardia la bloccherebbe a ogni push.
    expect(checkFile(workflow(ok))).toEqual([]);
  });

  it("un'azione composite sconosciuta resta un errore", () => {
    // L'esenzione e' per nome, non per tipo: nessuno deve poterne aggiungere una
    // senza registrarla.
    const problems = checkFile(workflow(ok + "\n      - uses: some/other-composite@stable\n"));
    expect(problems.join("\n")).toContain("non registrata");
  });

  it("un ref per SHA o un branch non e' una versione, e lo dice", () => {
    // Non e' un errore se voluto, ma e' un punto in cui il controllo non vede
    // niente: meglio un avviso che un silenzio.
    const problems = checkFile(
      workflow(ok.replace("actions/checkout@v7", "actions/checkout@11bd71901bbe5b1630ceea73d27597364c9af683")),
    );
    expect(problems.join("\n")).toContain("non e' una versione");
  });
});

describe("lettura del workflow", () => {
  it("raccoglie le immagini sia da runs-on sia dalla matrice", () => {
    const labels = runnerLabels(ok).map((l) => l.value);
    expect(labels).toContain("ubuntu-24.04");
    expect(labels).toContain("macos-26");
    expect(labels).toContain("windows-2022");
  });

  it("ignora un runs-on che e' un'espressione", () => {
    // `runs-on: ${{ matrix.os }}` non e' un'immagine: l'immagine sta nella
    // matrice, e il controllo la prende da li'.
    const labels = runnerLabels("runs-on: ${{ matrix.os }}\n");
    expect(labels).toEqual([]);
  });

  it("raccoglie le azioni con e senza il trattino iniziale", () => {
    expect(usedActions("- uses: actions/checkout@v7\n").length).toBe(1);
  });
});
