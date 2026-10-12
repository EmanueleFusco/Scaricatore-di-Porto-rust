# Scaricatore Di Porto

Riscrittura cross-platform di *Scaricatore Di Porto* in **Rust + Tauri v2 + Svelte**.
La specifica completa è in [`docs/RUST_PORT_GUIDELINES.md`](docs/RUST_PORT_GUIDELINES.md).

**Stato:** **M0-A** completata e verificata a mano su Linux, Windows e macOS.
Cosa c'è: shell dell'app, impostazioni persistenti, i18n IT/EN, tema System/Dark/Light,
CI su tre sistemi e pubblicazione delle release in bozza.

**Non c'è ancora:** la coda, i download, yt-dlp, ffmpeg, la trascrizione. Tutto quello che
fa il programma per cui lo stai usando arriva nelle fasi successive.

Avanzamento completo e onesto: [`docs/PROGRESS.md`](docs/PROGRESS.md).

---

## Requisiti

| | |
|:--|:--|
| Linux | `libwebkit2gtk-4.1-dev`, `build-essential`, `libssl-dev`, `libxdo-dev`, `librsvg2-dev`, `libayatana-appindicator3-dev` |
| Windows | Microsoft C++ Build Tools, WebView2 (già presente su Windows 10 1803+) |
| macOS | Xcode Command Line Tools |
| ovunque | Node.js LTS, Rust stabile |

Elenco completo e sempre aggiornato: <https://v2.tauri.app/start/prerequisites/>

---

## Comandi

### Avviare l'app

```bash
npm install          # una volta sola
npm run app          # sviluppo, con hot reload
```

`npm run app` è il modo normale. Se sei su **Wayland** (la macchina di sviluppo lo è),
imposta da solo `GDK_BACKEND=x11` e `WEBKIT_DISABLE_DMABUF_RENDERER=1`, perché lì WebKitGTK
muore con `Error 71` e la finestra resta nera. Non devi ricordartelo: lo fa lo script e ti
dice cosa sta facendo. Per disattivarlo: `GDK_BACKEND=wayland npm run app`.

```bash
npm run app:binary   # esegue il binario release gia' compilato
```

### Controlli (da lanciare sempre prima di committare)

```bash
npm run check        # svelte-check
npm test             # 28 test frontend
npm run check:ci     # sintassi dei blocchi run: della CI
cd src-tauri && cargo fmt --all -- --check
cd src-tauri && cargo clippy --all-targets -- -D warnings
cd src-tauri && cargo test
```

### Release

```bash
# 1. alza la versione nei TRE manifest, tutti insieme:
#    src-tauri/tauri.conf.json, src-tauri/Cargo.toml, package.json
npm run check:release -- v0.2.0     # verifica che i tre concordino col tag

git add -A && git commit -m "build: versione 0.2.0"
git tag v0.2.0
git push origin v0.2.0
```

La CI costruisce su Windows, macOS e Linux e crea una **Release in bozza**. Scaricala,
provala, e solo poi clicca **Publish** dalla pagina Releases.

---

## Come funziona la CI

Il workflow è in [`.github/workflows/ci.yml`](.github/workflows/ci.yml) e ha cinque job.

| job | quando gira | cosa fa |
|:--|:--|:--|
| `check` | **ogni push** | fmt, clippy, 37 test Rust, 28 test JS, build frontend |
| `bundle` ×3 | tag `v*` o avvio manuale | compila e carica gli artifact per linux / windows / macos |
| `bundle-macos-universal` | idem | binario universale macOS (arm64 + Intel) |
| `release` | solo sui tag | raccoglie gli artifact e crea la Release **in bozza** |
| `tier2` | solo avvio manuale | macOS Intel, Windows ARM64, Linux ARM64 |

### Perché non gira tutto a ogni push

Il repository è **privato**. Il piano Free di GitHub dà 2 000 minuti di calcolo e **500 MB**
di spazio per gli artifact. Con i moltiplicatori (Linux 1x, Windows 2x, macOS 10x) un run
completo costa circa 95 minuti di quota e 150-200 MB di spazio.

Quindi: un push normale fa solo `check` (7 minuti). I binari si chiedono esplicitamente dalla
pagina Actions, oppure si tagga. I bundle su `ubuntu-22.04` perché si compila sul sistema
supportato più vecchio: il `.deb` risultante gira anche su distribuzioni più recenti.

### Come si ottengono i binari

**Actions → Run workflow**, scegli il branch, clicca *Run workflow*.
In fondo alla pagina, *Artifacts*, scarica quello che ti serve:

```
linux-x64        →  .deb   .rpm   AppImage
windows-x64      →  l'installer .exe
macos-arm64      →  il .dmg  +  macos-arm64-….app.zip
macos-universal  →  il .dmg  +  macos-universal-….app.zip
```

Le `.app` sono zippo sul runner macOS con `ditto`, non da Linux: zipparle da Linux
produce un'applicazione subtly rotta.

### I tre guardiani

| controllo | quando | cosa impedisce |
|:--|:--|:--|
| versione | prima di costruire | pubblicare `v0.2.0` contenente `…_0.1.0_amd64.deb` |
| bozza | alla fine | una release sbagliata che non si può più correggere |
| niente bundle a ogni push | all'avvio | sforare i 500 MB ed essere fatturato |

---

## Installazione e prova manuale

**Linux**
```bash
chmod +x *.AppImage && ./Scaricatore\ Di\ Porto_*.AppImage     # oppure installa il .deb
```

**Windows** — doppio clic sull'`.exe`, installa.

**macOS** — apri lo `.zip`, poi tasto destro → **Apri**. Non è firmato, quindi Gatekeeper
lo blocca. Nel caso:
```bash
xattr -dr com.apple.quarantine "Scaricatore Di Porto.app"
```

### Cosa controllare

1. parte, finestra 980×680 con la barra laterale
2. il tema segue quello del sistema
3. cambia lingua e tema, chiudi, riapri → devono restare
4. apri l'app due volte → la seconda dà solo il fuoco alla prima
5. il file delle impostazioni:

| | percorso |
|:--|:--|
| Linux | `~/.config/com.scaricatorediporto.app/settings.json` |
| Windows | `%APPDATA%\com.scaricatorediporto.app\settings.json` |
| macOS | `~/Library/Application Support/com.scaricatorediporto.app/settings.json` |

---

## Struttura

```
src/               frontend Svelte (i18n, tema, impostazioni, shell)
src-tauri/src/
  commands/        handler IPC, sottili
  services/        logica di business (paths, e presto process.rs)
  state.rs         Settings e stato condiviso
  error.rs         AppError + ErrorInfo
src-tauri/tests/   test d'integrazione IPC
docs/
  RUST_PORT_GUIDELINES.md   la specifica (SOLO LETTURA)
  PROGRESS.md                cosa funziona e cosa no
  DECISIONS.md               scelte, deviazioni, "verify" non risolti
  SPIKES.md                  prove che solo il proprietario può fare
.github/workflows/ci.yml
scripts/           helper Node (launcher, controlli, CI)
```

**Regola d'oro:** se qualcosa non funziona o non è verificato, si scrive in
`docs/PROGRESS.md`. Non si dichiara mai testato qualcosa che non è stato provato.

---

## Da fare per M0-B

`services/process.rs`: avvio sicuro dei processi con Job Object (Windows) e process group
(Unix), kill dell'albero, lettura di stdout e stderr in parallelo con ring buffer delle
ultime 200 righe di stderr, `CancellationToken`. È la parte più delicata del progetto e
serve per yt-dlp.

**Serve il riferimento di parità** `ScaricatoreDiPorto.py`, che non è in questo repository:
copialo dentro quando serve (la spec lo indica come fonte del comportamento da replicare).