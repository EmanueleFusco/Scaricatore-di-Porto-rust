# SPIKES

Prove che **solo il proprietario** puo' fare: servono account, chiavi di firma,
hardware o un giudizio su com'e' una cosa per l'utente finale. I campi dei
risultati sono vuoti apposta: li compili tu.

Ogni voce dice **cosa fare**, **come si fa** e **cosa scrivere**. Una voce vuota
non e' "tutto bene": e' "non ancora guardato", e in `docs/PROGRESS.md` lo
scrivi tale e quale.

Riferimento: spec §5.2.1 (notarizzazione e antivirus), §1 (budget di
dimensione), §14 riga M0 (criteri di uscita).

---

## S1 — macOS: firmare e notarizzare l'app **con** yt-dlp e qjs dentro

**Perche' e' una spike, non un test.** `yt-dlp_macos` e' un binario
PyInstaller: spec §5.2.1 lo dice esplicitamente e pretende che si verifichi se
notarizza pulito. Se non si puo' notarizzare, il fallback e' gia' deciso nella
spec: **non includere yt-dlp su macOS e scaricarlo al primo avvio**.

**Come.**

1. Sul runner `macos-latest` scarica i tool:
   `node scripts/fetch_bundled_tools.mjs --target universal-apple-darwin`
2. Costruisci: `npx tauri build --target universal-apple-darwin`
3. Firma con `codesign --options runtime --timestamp` usando l'identita'
   Developer ID, poi `xcrun notarytool submit ... --wait` e `stapler staple`.
4. **Controlla che i tool siano dentro la firma.** Il bundler li mette in
   `Contents/MacOS/` accanto al binario principale e li aggiunge a `sign_paths`
   (verificato in `tauri-bundler/src/bundle/macos/app.rs`), ma solo se la firma
   e' attiva: con `--no-sign` non vengono firmati.

**Cosa puo' andare storto, e cosa scrivere.**

| domanda | risultato |
| :-- | :-- |
| `yt-dlp_macos` si firma con hardened runtime? | |
| Servono entitlement per `yt-dlp_macos`? Quali? | |
| Il notch accetta l'app **con dentro** i due binari? | |
| `qjs` universale (lipo di arm64+x86_64) firma e passa? | |
| Quanto pesa l'app firmata? | |
| **Se fallisce:** si adotta il fallback "download al primo avvio su macOS"? | |

**Entitlement da provare se il notarization fallisce** (non sono verificati qui,
sono i sospetti indicati da spec §5.2.1):

- `com.apple.security.cs.allow-unsigned-executable-memory` — QuickJS compila il
  codice a runtime
- `com.apple.security.cs.disable-library-validation` — PyInstaller carica librerie
  non firmate dalla sua stessa posizione

Scrivi quale di questi ha risolto il problema: sono informazioni che si perdono.

---

## S2 — Windows: l'installer firmato su una macchina pulita, Defender acceso

**Perche'.** Spec §5.2.1: "PyInstaller executables sometimes trigger
antivirus/SmartScreen heuristics". Un installer che SmartScreen blocca e'
l'unico modo in cui un utente Windows scopre che l'app non parte.

**Come.**

1. Scarica l'installer dalla CI: Actions → `bundle (windows-x64)` → artifact
   `windows-x64` → l'`.exe` di NSIS.
2. Firmalo con un certificato di code signing
   (`tauri.conf.json` → `bundle.windows.certificateThumbprint`, o la chiave nei
   secret di CI).
3. Copialo su **una macchina Windows vergine**: niente tool installati, Defender
   acceso, SmartScreen come sta.
4. Doppio clic, installa, apri.

**Cosa scrivere.**

| domanda | risultato |
| :-- | :-- |
| L'installer si apre senza "Windows protected your PC"? | |
| Defender segnala qualcosa durante l'installazione? | |
| Quanto dura la prima apertura (estrazione di PyInstaller)? | |
| L'app si avvia e trova yt-dlp + qjs inclusi? | |
| `yt-dlp.exe` viene messo in quarantena da Defender? | |

---

## S3 — Costo di avvio di yt-dlp: singolo file contro variante non impacchettata

**Perche'.** Spec §5.2.1 segnala che i build ufficiali sono bundle PyInstaller
one-file, **si spacchettano a ogni avvio**, e questa app avvia un processo
yt-dlp per ogni ispezione e per ogni elemento della coda. Se il costo è
visibile, la spec chiede di valutare la variante spacchettata.

**Come misurare (non a occhio).**

```bash
# primo avvio (spacchettamento)
time ./src-tauri/binaries/yt-dlp-x86_64-unknown-linux-gnu --version
# avvii successivi (cache del filesystem)
for i in 1 2 3 4 5; do
  /usr/bin/time -f "%e s" ./src-tauri/binaries/yt-dlp-x86_64-unknown-linux-gnu --version >/dev/null
done
```

Ripeti su Windows (`Measure-Command { .\yt-dlp.exe --version }`) e su macOS, e
poi confronta con la variante spacchettata (`yt-dlp_linux.zip` / `yt-dlp_win.zip`
dello stesso tag) che si decomprime in una cartella e si esegue da li'.

**Cosa scrivere.**

| piattaforma | one-file, 1° avvio | one-file, media 5 | spacchettato, media 5 |
| :-- | :-- | :-- | :-- |
| Linux x64 | | | |
| Windows x64 | | | |
| macOS arm64 | | | |

**Decisione:** si tiene il one-file o si passa allo spacchettato?

> Nota: la variante non impacchettata **non ha l'auto-aggiornamento integrato**.
> Se si sceglie quella, il nostro updater (spec §5.5) deve sostituire una
> cartella invece di un file, e `tools_install`/`tools_update` vanno riscritti di
> conseguenza. Verificare quale variante pubblica ogni piattaforma.

---

## S4 — Budget di dimensione dell'installer

**Perche'.** Spec §1 fissa il binario applicativo a **≤ 15 MB** e dice che
"installer size ... measure it in M0 and fix the budget then". Il numero va
deciso qui, non lasciato a un "vediamo".

**Stato: i numeri ci sono, la decisione no.** Misurati il 2026-10-13, build
completa **con** i tool inclusi (spec §5.2.1).

**Rimisurati dopo D55**, che ha spostato `bundle` da `ubuntu-22.04` a
`ubuntu-24.04`: i numeri della prima misura erano su 22.04 e **non valgono piu'**.
Il confronto dice quale dei due il runner cambia, e quanto:

| artefatto | 22.04 (prima) | 24.04 (adesso) | delta |
| :-- | --: | --: | --: |
| binario applicativo | 4,81 MiB | **4,70 MiB** | -2,3% |
| `.deb` | 41,36 MiB | **41,3 MiB** | -0,1% |
| `.rpm` | 41,36 MiB | **41,3 MiB** | -0,1% |
| AppImage | 147,27 MiB | **116,4 MiB** | **-21%** |

Con la build su 24.04, che e' quella della CI:

| artefatto | dimensione | dentro il budget? |
| :-- | --: | :-- |
| binario applicativo `scaricatore-di-porto` | **4,70 MiB** | ✅ spec §1 fissa ≤ 15 MB |
| `.deb` | **41,3 MiB** | da decidere |
| `.rpm` | **41,3 MiB** | da decidere |
| AppImage | **116,4 MiB** | escluso dal budget (spec §1: incorpora WebKitGTK) |
| installer `.exe` (NSIS) | | mancante |
| `.dmg` macOS arm64 | | mancante |
| `.app.zip` macOS arm64 | | mancante |
| `.dmg` macOS universal | | mancante |

Da cui, la composizione dell'installer:

| componente | byte | MiB |
| :-- | --: | --: |
| `yt-dlp` (standalone, incluso) | 40 446 224 | 38,6 |
| `qjs` (QuickJS-NG 0.17.0, incluso) | 2 562 504 | 2,4 |
| binario applicativo | 5 039 440 -> ~4,7 | 4,8 -> 4,7 |
| `.deb` effettivo (compresso) | | **41,4** |

Il termine dominante e' **yt-dlp**, che da solo e' il 93 % dell'installer. E' il
prezzo esatto di "nessun passo di rete al primo avvio" (vedi D30): in cambio si
paga 38,6 MiB in ogni installazione, anche per chi non scarica mai niente.

Verificato dentro il `.deb`, non supposto:

```
/usr/bin/scaricatore-di-porto
/usr/bin/yt-dlp
/usr/bin/qjs
```

Tutti e tre nello stesso `PATH` di sistema, che e' dove spec §5.2 vuole i tool
inclusi.

**Cosa scrivere.**

- Budget installer Windows:
- Budget installer macOS:
- Budget .deb / .rpm:
- **Va bene 41 MiB per un'installazione, o il vincolo deve essere piu' stretto?**
- La decisione su S3 cambia questi numeri?

---

## S5 — Prova a mano della cancellazione contro un yt-dlp vero

Non e' una spike nel senso stretto (serve la tua macchina, non un account), ma
nessun test automatico puo' coprirla e resta aperta da M0-B.

M0-B ha provato che l'albero di processi muore con un **programma finto**.
Questa prova usa `yt-dlp` vero, che a sua volta avvia `ffmpeg` e l'interprete
JavaScript.

**Come.** Avvia un download vero e premi Annulla a metà. Poi guarda l'elenco dei
processi del sistema.

**Cosa scrivere.**

| sistema | `yt-dlp` rimasto? | `ffmpeg` rimasto? | `qjs`/`deno` rimasto? | entro ~2 s? |
| :-- | :-- | :-- | :-- | :-- |
| Windows x64 | | | | |
| Linux x64 | | | | |
| macOS arm64 | | | | |

Poi: chiudere l'app a meta' download lascia processi orfani?

---

## S6 — Tier 2 (solo quando un runner lo permette)

Spec §1.1: macOS Intel, Windows ARM64 e Linux arm64 sono **sperimentali, solo
CI**. Il pulsante `tier2` esiste gia' in `.github/workflows/tier2.yml`.

- [ ] macOS Intel (`macos-15-intel`): build + test verdi?
- [ ] Windows ARM64 (`windows-11-arm`): build + test verdi?
- [ ] Linux arm64 (`ubuntu-24.04-arm`): build + test verdi?
- [ ] yt-dlp e qjs esistono per questi target nel manifest?

Per gli strumenti che **non** esistono come asset su un target, spec §1.1 vuole
"not available on this platform" nella UI e **non** un fallimento silenzioso.
**Cosa dice il confronto 22.04 -> 24.04 sul *perché* dei numeri, e vale piu' dei
numeri.** `.deb` e `.rpm` non si sono mossi (−0,1%), l'AppImage è scesa del 21%.
Non è rumore: è la differenza fra un pacchetto che **linka dinamicamente** e uno
che **incorpora**. `.deb` e `.rpm` prendono WebKitGTK dal sistema, quindi
cambiano solo se cambia il binario; l'AppImage se lo porta dentro, quindi cambia
se cambia la WebKitGTK del runner su cui è costruita. È la stessa distinzione che
spec §1 usa per escludere l'AppImage dal budget, resa visiva.

Leversioni di `libwebkit2gtk-4.1-dev` dei due runner, prese dalle pagine dei
pacchetti Ubuntu: **jammy (22.04) 2.50.4** e **noble (24.04) 2.52.6**. Quindi il
−21% è *coerente* con un salto di WebKitGTK, ma **non è misurato**: per saperlo
bisognerebbe aprire l'AppImage e pesare il pezzo. Scritto come inferenza, non
come fatto.

Il termine dominante non cambia e non può cambiare da qui: `yt-dlp` è un asset
**scaricato**, non compilato. Il suo contributo all'installer dipende da GitHub,
non dal runner.
