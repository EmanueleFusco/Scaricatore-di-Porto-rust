# Siti supportati

Scaricatore Di Porto non ha un elenco di siti proprio. Il motore di download è
[yt-dlp](https://github.com/yt-dlp/yt-dlp), quindi l'elenco vero è quello
mantenuto da yt-dlp:

- [Siti supportati da yt-dlp](https://github.com/yt-dlp/yt-dlp/blob/master/supportedsites.md)
- Sulla tua macchina: `yt-dlp --list-extractors` (la copia dentro l'app è nella
  cartella mostrata in **Impostazioni → Strumenti → Cartelle**).

Se un sito è in quell'elenco, di solito l'app può scaricarlo. L'elenco cambia a
ogni release di yt-dlp; quando un sito smette di funzionare, aggiorna prima
yt-dlp (**Impostazioni → Strumenti → Aggiorna**).

## YouTube

- Video, Shorts, playlist e canali.
- Sottotitoli manuali e didascalie automatiche.
- Stream live: dall'inizio o dal momento attuale, a seconda di ciò che lo
  stream espone.
- Video con limiti di età, privati o riservati agli abbonati usando i cookie di
  un browser o un file Netscape (**Impostazioni → Rete e account**).
- Gli URL "video + playlist" (`watch?v=…&list=…`) chiedono se vuoi il singolo
  video o tutta la playlist.
- I contenuti protetti da DRM non sono supportati.

## Altri siti

Questi sono esempi comuni, non una promessa che ogni pagina funzioni oggi:

| Sito | Uso tipico |
| :-- | :-- |
| Vimeo | Pagine video, alcune showcase |
| SoundCloud | Brani e set |
| Bandcamp | Brani e album |
| Twitch | VOD e clip (il live dipende dallo stream) |
| TikTok | Video pubblici |
| Instagram | Post pubblici, Reel e storie quando yt-dlp riesce ad accedervi |
| Facebook | Video pubblici |
| X / Twitter | Video pubblici |
| Reddit | Video ospitati da Reddit e player esterni |
| Link diretti | File HTTP(S) semplici, manifest HLS (`.m3u8`) e DASH |

L'estrattore generico gestisce anche molte pagine normali che incorporano un
video. Quando non ci riesce, l'app mostra l'errore di yt-dlp invece di tirare a
indovinare.

## Formati

La barra di aggiunta offre **Migliore disponibile**, un'altezza massima (1080p,
720p, …) oppure **Solo audio**. Per qualsiasi altra cosa apri la tabella
avanzata dei formati, scegli una riga o incolla un ID di formato yt-dlp esatto.
L'app unisce video e audio con ffmpeg quando serve.

## Quando un sito richiede l'accesso

Alcuni siti mostrano i contenuti solo a una sessione autenticata. Due opzioni:

1. **Cookie da un browser** — scegli il browser in **Impostazioni → Rete e
   account**. Leggere il database dei cookie può fallire mentre il browser è
   aperto; chiudilo e riprova.
2. **File dei cookie** — esporta un `cookies.txt` in formato Netscape e
   sceglilo nella stessa pagina.

I cookie sono segreti. Vengono passati a yt-dlp, non scritti nel log dell'app e
non inclusi in **Copia diagnostica**.

## Quando un sito blocca l'app

A volte i siti rispondono con una pagina anti-bot o HTTP 403. Due impostazioni
aiutano:

- **Riprova le richieste bloccate con l'impersonificazione di Chrome**
  (Impostazioni → Rete e account) riprova una volta con `--impersonate chrome`.
- **Runtime JavaScript** — yt-dlp ha bisogno di un interprete JavaScript per le
  sfide di YouTube. QuickJS è incluso; se non basta, l'app può installare Deno
  con il tuo consenso.

## Stream live, DRM e blocchi geografici

- **Live** — uno stream live può essere scaricato dall'inizio o da "adesso";
  l'app segue ciò che riporta l'estrattore. Uno stream appena terminato può
  richiedere un nuovo tentativo.
- **DRM** — i contenuti cifrati (Netflix, Disney+, molte piattaforme a
  pagamento) non sono scaricabili. L'app riporta l'errore DRM invece di salvare
  un file rotto.
- **Blocchi geografici** — il sito rifiuta in base alla posizione della rete. Si
  può configurare un proxy nelle Impostazioni, ma rispetta i termini del sito.
- **Limiti di richieste** — abbassa **Download paralleli** o **Massimo per
  sito** se un sito inizia a rifiutare le richieste.

## Segnalare un sito che non funziona

Includi:

1. l'URL della pagina (o uno pubblico simile, se l'originale è privato);
2. l'errore esatto dal pannello **Dettagli** dell'elemento;
3. la versione di yt-dlp da **Impostazioni → Strumenti**;
4. **Impostazioni → Informazioni → Copia diagnostica**.

Non incollare mai cookie o chiavi API in una issue.
