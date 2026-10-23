# Risoluzione dei problemi

La maggior parte dei problemi lascia un messaggio leggibile nel pannello
**Dettagli** dell'elemento. Parti da lì, poi apri **Impostazioni → Informazioni**:

- **Copia diagnostica** produce un rapporto in testo semplice con versione,
  sistema operativo, percorsi degli strumenti e le ultime righe del log.
- **Apri i log** mostra il file di log nel gestore file.
- **Cartelle** mostra dove si trovano impostazioni, strumenti, modelli e log.

Il rapporto contiene percorsi locali ma nessuna chiave API o cookie. Leggilo
prima di pubblicarlo.

---

## Strumenti

### "ffmpeg is missing" / l'unione o la conversione audio fallisce

ffmpeg non è incluso per dimensioni e licenza. Installalo da **Impostazioni →
Strumenti → ffmpeg → Installa**. L'app installa anche **ffprobe**, necessario
per leggere le durate.

Se il download fallisce, usa **Usa il mio percorso** e indica un binario
`ffmpeg` esistente. Va bene anche il pacchetto di sistema: su Debian/Ubuntu
`sudo apt install ffmpeg`, su Fedora `sudo dnf install ffmpeg`, su macOS
`brew install ffmpeg`, su Windows `winget install Gyan.FFmpeg`.

### "sdp-whisper sidecar is not installed" / Strumenti dice "Non ancora disponibile"

La trascrizione offline richiede il sidecar `sdp-whisper` e un modello Whisper.
Il sidecar viene pubblicato separatamente dall'app. Se la pagina Strumenti dice
che non esiste ancora una release, l'app non può scaricarlo: usa **Usa il mio
percorso** se hai un binario `sdp-whisper` compatibile, oppure usa Gemini per la
trascrizione finché il sidecar non è disponibile.

Un **modello** mancante è un problema diverso: scaricalo da **Impostazioni →
Trascrizione e Modelli**.

### yt-dlp fallisce, non mostra formati o restituisce HTTP 403

1. **Aggiorna yt-dlp** in **Impostazioni → Strumenti**. I siti cambiano di
   continuo e la maggior parte degli errori di estrazione si risolve con un
   aggiornamento.
2. Prova il canale **nightly** se la release stabile è indietro.
3. Se il messaggio parla di **sfida JavaScript**, lascia il runtime su
   **Automatico** o installa **Deno** quando l'app lo propone.
4. Se il sito richiede l'accesso, configura i **cookie** (vedi sotto).
5. Se il sito blocca le richieste automatiche, attiva **Riprova le richieste
   bloccate con l'impersonificazione di Chrome**.

### "The browser cookies could not be read"

Chiudi completamente il browser e riprova: i browser bloccano il database dei
cookie mentre sono aperti. Se continua a fallire, esporta un `cookies.txt` in
formato Netscape e sceglilo in **Impostazioni → Rete e account → File dei
cookie**.

I cookie scadono. Se un video privato improvvisamente fallisce, esportali di
nuovo.

### Un download resta su "Waiting for data"

Di solito significa che la rete si è bloccata o il sito sta limitando la
richiesta. Annulla e riprova l'elemento. Se si ripete, abbassa **Download
paralleli** o **Massimo per sito** nelle Impostazioni, oppure configura un
proxy.

### La coda dice "Interrupted"

L'app si è chiusa o è andata in crash mentre l'elemento era in esecuzione. Premi
**Riprendi**. Se il file parziale non è utilizzabile, rimuovi l'elemento e
aggiungilo di nuovo; i file completati non vengono mai cancellati.

---

## Trascrizione

### "No subtitles were found"

Il video non ha sottotitoli manuali o automatici nelle lingue richieste. Scegli
un ordine dei motori che includa Whisper o Gemini, oppure cambia la lingua
predefinita. **Solo sottotitoli** fallisce di proposito quando non ci sono
sottotitoli.

### Whisper è lento o esaurisce la memoria

Whisper gira sulla CPU. Un modello più piccolo è più veloce e usa meno RAM;
**Small** è una scelta ragionevole su macchine con 4 core, **Turbo** è il
miglior compromesso qualità/dimensione su hardware moderno. Chiudi le
applicazioni che occupano molta memoria e abbassa **Thread CPU** solo se il
resto del sistema diventa poco reattivo.

### La trascrizione ha righe ripetute o allucinazioni

Attiva **Salta il silenzio con Silero VAD** (è attivo per impostazione
predefinita quando il modello VAD è installato). I lunghi silenzi sono la causa
più comune del testo ripetuto. L'audio molto rumoroso può comunque produrre
errori; prova un modello più grande.

### Gemini: "API key is invalid or unauthorized"

- Controlla che sia una chiave API Gemini di Google AI Studio, non un client
  secret OAuth.
- Premi **Prova chiave** in **Impostazioni → Trascrizione e Modelli**.
- Se funzionava prima, potrebbe essere stata revocata o il progetto potrebbe
  non avere più quota.

### Gemini: "Model … is no longer available"

Google ritira i nomi dei modelli. Apri **Impostazioni → Trascrizione e
Modelli**, premi **Aggiorna modelli** e scegli un alias `-latest` attuale (per
esempio `gemini-flash-latest`).

### Gemini: limite di richieste o quota superata

Aspetta e riprova, oppure controlla la quota del progetto Google Cloud della
chiave. L'app riprova gli HTTP 429 con attese crescenti, ma non può creare
quota.

### Dove viene salvata la chiave API?

Nel portachiavi del sistema. Su Linux senza un demone Secret Service, l'app usa
un file con permessi `0600` nella cartella di configurazione. La chiave non
viene mai inviata alla webview e non finisce nel log. Funziona anche la
variabile d'ambiente `GEMINI_API_KEY`, che ha la priorità.

### I riassunti falliscono

Anche i riassunti usano Gemini. Servono una chiave configurata e una
trascrizione; se la trascrizione viene dai sottotitoli, va bene lo stesso. Per
trascrizioni molto lunghe l'app riassume a blocchi e poi li combina: possono
volerci diverse richieste.

---

## Aggiornamenti

### "Controlla aggiornamenti" fallisce

Il controllo contatta l'endpoint delle release su GitHub. Un proxy, un firewall
o un disservizio GitHub possono bloccarlo. Download e resto dell'app continuano
a funzionare. Riprova più tardi.

### Linux: l'app dice che c'è un aggiornamento ma non riesce a installarlo

Solo l'**AppImage** può sostituirsi da sola. Se hai installato il `.deb` o
l'`.rpm`, aggiornalo con il gestore di pacchetti oppure scarica la nuova release
dalla pagina Releases. In quel caso la pagina Informazioni mostra il link alle
release.

### L'aggiornamento viene scaricato ma la versione non cambia

Su Windows l'installer può richiedere la chiusura dell'app; l'app ti chiede di
riavviare. Su Linux assicurati di eseguire l'AppImage da una posizione
scrivibile e che il file non sia stato sostituito dal gestore di pacchetti. Se
la verifica della firma fallisce, l'aggiornamento viene rifiutato di proposito:
scarica la release manualmente.

### Controlli giornalieri

L'app controlla al massimo una volta ogni 24 ore e non installa mai nulla da
sola. Il timestamp è uno stato locale dell'interfaccia, non un'impostazione.

---

## Problemi di piattaforma

### Linux: la finestra è nera o va in crash su Wayland

Alcune combinazioni WebKitGTK/GPU non riescono ad allocare i buffer DMA-BUF.
Avvia con:

```bash
WEBKIT_DISABLE_DMABUF_RENDERER=1 ./Scaricatore*.AppImage
```

Se la finestra continua a chiudersi, forza XWayland:

```bash
GDK_BACKEND=x11 WEBKIT_DISABLE_DMABUF_RENDERER=1 ./Scaricatore*.AppImage
```

Per il server di sviluppo, `npm run app` applica già entrambi i workaround su
Wayland. I pacchetti installati hanno bisogno delle variabili nel launcher o
nella voce del menu.

### Linux: il selettore cartelle o "Apri file" non fa nulla

Usano il portale del desktop / `xdg-open`. Installa `xdg-utils` e
un'implementazione del portale per il tuo desktop (per esempio
`xdg-desktop-portal-gtk` o `xdg-desktop-portal-kde`).

### Linux: il portachiavi non è disponibile

L'app salva la chiave Gemini in un file `0600` nella cartella di configurazione
e lo dice nelle Impostazioni. È normale su window manager minimali, sessioni
headless e alcuni container.

### macOS: "L'app è danneggiata" o Gatekeeper rifiuta di aprirla

Le build attuali non sono notarizzate. Prova prima con clic destro → **Apri**.
Se macOS rifiuta ancora, verifica il download e rimuovi l'attributo di
quarantena:

```bash
xattr -dr com.apple.quarantine "/Applications/Scaricatore Di Porto.app"
```

Fallo solo per un file scaricato dalla pagina Releases ufficiale.

### Windows: SmartScreen o l'antivirus avvisano sull'installer

L'installer non è firmato. SmartScreen mostra **Ulteriori informazioni → Esegui
comunque**. Alcuni antivirus segnalano anche `yt-dlp.exe` incluso: è il binario
ufficiale di yt-dlp, verificato con il checksum della release prima di essere
incluso.

### Nessuna notifica quando la coda finisce

Attiva **Avvisami quando la coda finisce** nelle Impostazioni e consenti le
notifiche per l'app nelle impostazioni del sistema. Su Linux deve essere in
esecuzione un demone delle notifiche.

---

## File e disco

### Dov'è finito il mio file?

Apri il pannello **Dettagli** dell'elemento: mostra il percorso finale. La
cartella predefinita è quella dei Download di sistema, a meno che tu non ne
abbia scelta un'altra nelle Impostazioni o nella barra di aggiunta. Le playlist
possono usare una cartella per playlist.

### "Insufficient disk space"

Libera spazio e riprova. I modelli Whisper sono grandi (il Turbo consigliato è
circa 547 MB) e una trascrizione può aver bisogno temporaneamente di spazio per
un WAV e i blocchi. La tabella dei modelli mostra lo spazio libero prima del
download.

### Voglio ricominciare da zero

- Impostazioni: **Impostazioni → Informazioni → Cartelle** mostra il percorso
  delle impostazioni. Chiudi l'app prima di modificarlo o cancellarlo; un file
  corrotto viene salvato come backup e sostituito con i valori predefiniti al
  prossimo avvio.
- Archivio download: **Impostazioni → Generali → Svuota archivio**.
- Modelli e strumenti: cancellali dalle rispettive righe in **Impostazioni →
  Trascrizione** e **Impostazioni → Strumenti**, oppure rimuovi le cartelle
  mostrate in Informazioni.
- Log: **Apri i log**, poi cancella il file con l'app chiusa.

---

## Segnalare un bug

Includi:

1. cosa hai fatto, cosa ti aspettavi, cosa è successo;
2. il messaggio di errore esatto dal pannello **Dettagli** dell'elemento;
3. versione dell'app e sistema operativo da **Impostazioni → Informazioni**;
4. **Copia diagnostica**;
5. alcune righe rilevanti da **Apri i log**.

Non includere chiavi API, cookie o URL privati. Se serve un URL per riprodurre
il problema, usa uno pubblico o descrivi il sito.
