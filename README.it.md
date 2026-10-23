# Scaricatore Di Porto

Un downloader e trascrittore a coda, semplice e ordinato, per Windows, macOS e
Linux. Scarica video e audio con [yt-dlp](https://github.com/yt-dlp/yt-dlp),
elabora i file con [ffmpeg](https://ffmpeg.org) e trasforma la voce in testo con
[Whisper](https://github.com/ggml-org/whisper.cpp) offline o Google Gemini.

L'app è scritta in Rust, Tauri v2 e Svelte. Non ha telemetria, non richiede un
account e non lascia servizi in background: fa qualcosa solo quando glielo chiedi.

[English](README.md) · [Siti supportati](docs/SUPPORTED_SITES.it.md) ·
[Risoluzione dei problemi](docs/TROUBLESHOOTING.it.md)

---

## Cosa fa

- **Una coda, non un lavoro alla volta.** Aggiungi più link, esegui 1–4 download
  in parallelo (2 per impostazione predefinita), metti in pausa, riprendi,
  riprova, riordina e ricomincia da dove eri rimasto.
- **Playlist e canali.** Scegli le voci che vuoi, tieni una cartella per ogni
  playlist, numera i file e salta quelli già scaricati.
- **Non solo YouTube.** Qualsiasi sito supportato da yt-dlp: Vimeo, SoundCloud,
  Twitch, TikTok, Instagram, link HLS/HTTP diretti e molti altri.
- **Formati a scelta.** Migliore disponibile, altezza massima (1080p, 720p, …),
  solo audio, oppure un ID di formato yt-dlp esatto dalla tabella avanzata.
- **Elaborazione multimediale.** Codifica in MP4/MKV/WebM o audio (AAC, MP3,
  Opus), taglio senza ricodifica quando possibile, metadati, miniatura e
  capitoli incorporati, rimozione dei segmenti SponsorBlock.
- **Trascrizioni.** Prima i sottotitoli esistenti, poi Whisper offline, poi
  Gemini, nell'ordine che scegli. Produce `.txt` e `.srt` (Whisper scrive
  anche `.vtt`) e un `.summary.md` opzionale.
- **Anche file locali.** Trascina un file multimediale sull'app per
  trascriverlo senza scaricarlo di nuovo.
- **Tutto scaricabile dall'app.** yt-dlp e QuickJS sono inclusi
  nell'installer; ffmpeg, ffprobe, Deno, i modelli Whisper e gli aggiornamenti
  vengono scaricati su richiesta, con checksum e avanzamento.
- **Aggiornamenti.** La pagina Informazioni controlla un canale firmato al
  massimo una volta al giorno. Su Linux solo l'AppImage può aggiornarsi da sola;
  per `.deb` e `.rpm` c'è il link alla pagina delle release.

## Installazione

Scarica l'ultima release dalla
[pagina Releases](https://github.com/EmanueleFusco/ScaricatoreDiPorto/releases).

| Sistema | File | Note |
| :-- | :-- | :-- |
| Windows 10/11 x64 | `*-setup.exe` | Installer NSIS. WebView2 è già presente su Windows 11 e sulla maggior parte dei Windows 10 aggiornati. |
| macOS (universale) | `*_universal.dmg` o `*.app.zip` | Build universale (arm64 + x86_64). Le build attuali non sono notarizzate; vedi la [guida ai problemi](docs/TROUBLESHOOTING.it.md). |
| Linux x64 | `*.AppImage`, `*.deb`, `*.rpm` | L'AppImage è autonoma e supporta gli aggiornamenti dall'app. |

Gli installer non sono ancora firmati. Su Windows SmartScreen può chiederti una
conferma; su macOS la prima volta usa **Apri** dal menu contestuale. Sono avvisi
attesi per una build non firmata, non un segno che il file sia danneggiato.

### Primo avvio

La procedura iniziale si apre una volta sola. Ogni passaggio è facoltativo e
può essere saltato:

1. **Benvenuto** — scegli lingua e tema.
2. **Strumenti** — yt-dlp e QuickJS sono già dentro l'app. Se manca ffmpeg,
   **Installa gli strumenti mancanti** lo scarica (insieme a ffprobe) con
   verifica del checksum. Deno è facoltativo e viene proposto solo se serve.
3. **Trascrizione** — scegli l'ordine dei motori e, se vuoi la trascrizione
   offline, scarica il modello Whisper consigliato. Gemini si può configurare
   più tardi con una chiave API.
4. **Cartella di destinazione** — usa la cartella Download di sistema o
   scegline un'altra.

Puoi riaprire la procedura quando vuoi da **Impostazioni → Strumenti → Riapri
la configurazione**. L'app resta utilizzabile anche se salti tutto: gli
strumenti mancanti compaiono con il pulsante **Installa** in **Impostazioni →
Strumenti**.

## Uso

### Aggiungere link

Incolla uno o più link nella casella in cima alla coda. Puoi anche trascinare un
file `.txt` con un link per riga. Ogni link viene controllato da solo e mostra
titolo, sito, durata e qualità prima di entrare in coda.

Scegli una qualità (o **Solo audio**), poi premi **Aggiungi alla coda**.
**Scarica** avvia subito i link. Se un link è già in coda con le stesse opzioni,
l'app chiede prima di aggiungerlo una seconda volta.

### La coda

- **Filtra** con `Ctrl/Cmd+F`; **seleziona tutti i visibili** con il pulsante
  della barra.
- **Seleziona le righe** con la casella, `Shift`-clic per un intervallo, poi
  metti in pausa, riprendi, annulla o rimuovi la selezione dalla barra azioni.
- **Riordina** trascinando una riga, oppure usa **Sposta in cima** dal menu.
- **Espandi i dettagli** per vedere le opzioni scelte, il percorso del file,
  l'errore e gli ultimi messaggi dello strumento.
- Da tastiera: `Spazio` mette in pausa/riprende la selezione, `Del` la rimuove,
  `Ctrl/Cmd+,` apre le Impostazioni. Le scorciatoie sono ignorate mentre scrivi
  in un campo.
- **Metti tutto in pausa**, **Riprendi tutto**, **Riprova i falliti** e
  **Pulisci i completati** sono in fondo alla coda.
- Se l'app si chiude durante un download, l'elemento torna come **Interrotto**
  e può essere ripreso.

### Playlist e altri siti

Un link a una playlist o a un canale apre un pannello di selezione con un
elenco virtualizzato. Filtra, scegli un sottoinsieme, imposta cartella e
numerazione, poi premi **Aggiungi N elementi**. L'archivio dei download ricorda
gli ID già completati, così un'esecuzione successiva li salta; puoi svuotarlo
da **Impostazioni → Generali**.

Per un URL che contiene sia un video sia una playlist, l'app chiede se vuoi
**Solo questo video** o **Tutta la playlist**. Puoi rendere quella scelta
predefinita nelle Impostazioni.

I link non-YouTube si comportano come quelli di YouTube. La tabella avanzata
dei formati permette di incollare un ID di formato yt-dlp esatto. Stream live,
pagine protette da DRM e contenuti che richiedono l'accesso mostrano un
messaggio chiaro con l'azione da fare (di solito i cookie).

### Trascrizioni e riassunti

Imposta l'ordine dei motori in **Impostazioni → Trascrizione e Modelli**:

- **Sottotitoli → Whisper → Gemini** (predefinito): usa i sottotitoli manuali,
  poi quelli automatici, poi Whisper offline, poi Gemini.
- **Solo Whisper**, **Solo Gemini** oppure **Solo sottotitoli**.

Whisper gira in locale. Scarica un modello dalla tabella; **Turbo** è il
miglior compromesso qualità/dimensione. Gemini richiede una chiave API, salvata
nel portachiavi del sistema (o in un file `0600` dove il portachiavi non è
disponibile) e mai inviata alla webview.

Gli elementi completati si possono trascrivere più tardi con **Trascrivi…**.
Il riassunto viene scritto come `<nome>.summary.md` quando è attivo. Gemini
carica l'audio su Google solo se scegli quel motore; il file remoto viene
cancellato dopo successo, errore o annullamento.

### Impostazioni

- **Generali** — lingua, tema, cartella di download predefinita, download
  paralleli, limite per sito, incolla ovunque, notifiche, archivio, nomi dei
  file e preferenze sui codec.
- **Trascrizione e Modelli** — ordine dei motori, lingua, VAD, thread, gestione
  modelli Whisper, chiave e modelli Gemini, impostazioni dei riassunti.
- **Strumenti** — ogni strumento esterno con origine, versione e stato;
  installa, aggiorna, prova, usa il tuo percorso, riapri la procedura, canale
  yt-dlp (stabile/nightly) e test di estrazione YouTube.
- **Rete e account** — cookie da browser o file Netscape, proxy, intestazioni
  personalizzate, User-Agent, Referer, nuovo tentativo con impersonificazione,
  runtime JavaScript.
- **Informazioni** — versione, controllo aggiornamenti, licenze, **Copia
  diagnostica**, **Apri i log** e le cartelle usate dall'app.

## Privacy

- Nessuna telemetria, analisi o segnalazione automatica degli errori.
- La rete viene usata solo per i download, per gli strumenti/modelli scaricati
  su richiesta, per il controllo giornaliero degli aggiornamenti e per Gemini
  quando lo configuri.
- La chiave API di Gemini è nel portachiavi del sistema; su Linux senza Secret
  Service finisce in un file leggibile solo dal tuo utente (`0600`). La chiave
  non viene mai inviata al frontend e non finisce nei log.
- **Copia diagnostica** contiene versioni, sistema operativo, percorsi degli
  strumenti e le ultime righe del log. Non contiene chiavi API né cookie, ma
  contiene percorsi locali: leggila prima di pubblicarla in una issue.

## Risoluzione dei problemi

Inizia da **Impostazioni → Informazioni → Copia diagnostica** e **Apri i log**.
La [guida alla risoluzione dei problemi](docs/TROUBLESHOOTING.it.md) copre i
casi comuni: ffmpeg mancante, estrazione yt-dlp che fallisce, cookie, modelli
Whisper, Gemini, aggiornamenti su Linux, finestra nera su Wayland e installer
non firmati.

## Siti supportati

YouTube è il bersaglio principale, ma il downloader è yt-dlp: se yt-dlp
supporta un sito, di solito l'app può scaricarlo. Vedi [Siti
supportati](docs/SUPPORTED_SITES.it.md) per dettagli, esempi e avvertenze (DRM,
stream live, accesso con login).

## Compilare dai sorgenti

Requisiti: Node.js LTS, una toolchain Rust stabile e i
[prerequisiti di Tauri v2](https://v2.tauri.app/start/prerequisites/) per il tuo
sistema.

```bash
npm install
npm run fetch:tools     # yt-dlp + qjs per questa piattaforma (~43 MB, con checksum)
npm run app             # sviluppo, con hot reload
```

Controlli:

```bash
npm run check
npm test
npm run check:ci
npm run check:runners
npm run check:tools
( cd src-tauri && cargo fmt --all -- --check )
( cd src-tauri && cargo clippy --all-targets -- -D warnings )
( cd src-tauri && cargo test )
```

Bundle:

```bash
npx tauri build
```

Gli artefatti dell'updater vengono firmati durante la build. Per costruirli in
locale esporta `TAURI_SIGNING_PRIVATE_KEY` (il contenuto della chiave o il suo
percorso) e, se la chiave ha una password, `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`.
La chiave pubblica è in `src-tauri/tauri.conf.json`; quella privata non deve mai
entrare nel repository.

### Release

La versione vive in tre manifest (`package.json`, `src-tauri/Cargo.toml`,
`src-tauri/tauri.conf.json`). Aggiornali tutti, controlla il tag, poi pubblica
il tag e avvia a mano il workflow **CI** dalla pagina Actions:

```bash
npm run check:release -- v0.1.0
git tag v0.1.0
git push origin v0.1.0
```

Il job di release costruisce gli installer, genera `latest.json`, si ferma se
manca una piattaforma o un `.sig`, e crea una release **in bozza**. Pubblicala
dalla pagina Releases dopo aver provato i binari. La chiave privata
dell'updater e la password devono essere nei secret del repository come
`TAURI_SIGNING_PRIVATE_KEY` e `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`.

## Documenti del progetto

- [`docs/RUST_PORT_GUIDELINES.md`](docs/RUST_PORT_GUIDELINES.md) — la specifica.
- [`docs/PROGRESS.md`](docs/PROGRESS.md) — cosa è fatto, provato o ancora
  simulato.
- [`docs/DECISIONS.md`](docs/DECISIONS.md) — decisioni e deviazioni.
- [`docs/SPIKES.md`](docs/SPIKES.md) — verifiche di piattaforma (notarizzazione,
  antivirus, dimensioni).
- [`docs/ACCEPTANCE.md`](docs/ACCEPTANCE.md) — la checklist §15 della specifica
  e ciò che resta aperto.

## Licenza

MIT. I componenti di terze parti e le loro licenze sono elencati in
**Impostazioni → Informazioni → Licenze** e in `tools.manifest.json`. ffmpeg è
distribuito con licenza GPL-3.0-or-later; yt-dlp è Unlicense; QuickJS-NG,
whisper.cpp e Deno sono MIT.
