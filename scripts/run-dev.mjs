// Avvia l'app Tauri gestendo il backend grafico.
//
// Sul portatile del proprietario (CachyOS, KDE Plasma, sessione **Wayland**,
// NVIDIA RTX 3080) `npm run tauri dev` presenta due problemi distinti:
//
//   1. `Gdk-Message: Error 71 (Errore di protocollo) dispatching to Wayland display.`
//      — l'app muore prima di creare la finestra. Non è il nostro codice:
//      verificato che anche il template Tauri di default muove allo stesso modo.
//   2. Finestra **nera** con "Failed to create GBM buffer" — WebKitGTK non
//      riesce ad allocare i buffer DMA-BUF della GPU NVIDIA.
//
// Qui iniettiamo GDK_BACKEND=x11 e WEBKIT_DISABLE_DMABUF_RENDERER=1 quando siamo
// su Wayland e XWayland è disponibile, e lo diciamo a schermo: niente variabili
// segrete, niente comportamento diverso sulle macchine dove Wayland funziona.
// Il fallback del rendering software è quello che la spec §13.3 prescrive.
//
//   npm run app                 -> tauri dev
//   npm run app -- --no-watch   -> passa gli argomenti al CLI
import { spawn } from "node:child_process";
import process from "node:process";

function needsX11Workaround() {
  if (process.platform !== "linux") return false;
  if (process.env.GDK_BACKEND) return false; // l'utente ha già deciso
  const wayland = process.env.XDG_SESSION_TYPE === "wayland" || !!process.env.WAYLAND_DISPLAY;
  const x11 = !!process.env.DISPLAY;
  return wayland && x11;
}

const env = { ...process.env };
if (needsX11Workaround()) {
  env.GDK_BACKEND = "x11";
  // WebKitGTK su NVIDIA alloca i buffer via GBM; se l'allocazione fallisce la
  // finestra resta NERA a schermo, anche se i pixel nel buffer X11 sono
  // corretti (misurati con `magick import`, che bypassa il compositor).
  // Forziamo il rendering software: è il fallback che prescrive la spec §13.3.
  env.WEBKIT_DISABLE_DMABUF_RENDERER = "1";
  console.log(
    "[sdp] Sessione Wayland + WebKitGTK: avvio con GDK_BACKEND=x11 e\n" +
      "[sdp] WEBKIT_DISABLE_DMABUF_RENDERER=1 (finestra nera con i driver NVIDIA).\n" +
      "[sdp] Le due variabili servono solo su questa macchina: su altri sistemi\n" +
      "[sdp] l'app parte senza. Per disattivare il comportamento:\n" +
      "[sdp]   GDK_BACKEND=wayland npm run app\n",
  );
}

const child = spawn("npx", ["tauri", "dev", ...process.argv.slice(2)], {
  env,
  stdio: "inherit",
  shell: process.platform === "win32",
});

child.on("exit", (code, signal) => {
  if (signal) process.kill(process.pid, signal);
  else process.exit(code ?? 0);
});