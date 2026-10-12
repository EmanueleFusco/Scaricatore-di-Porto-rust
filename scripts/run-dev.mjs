// Avvia l'app Tauri gestendo il backend grafico.
//
// Sul portatile del proprietario (CachyOS, KDE Plasma, sessione **Wayland**,
// NVIDIA RTX 3080) `npm run tauri dev` termina subito con
//     Gdk-Message: Error 71 (Errore di protocollo) dispatching to Wayland display.
// Il crash avviene prima che la finestra venga creata e riguarda GTK/WebKitGTK
// contro quel compositor, non il codice dell'app (verificato: anche il template
// Tauri di default, senza una riga del nostro codice, muove allo stesso modo).
//
// Qui iniettiamo quindi GDK_BACKEND=x11 quando siamo su Wayland e XWayland è
// disponibile, e lo diciamo a schermo: niente variabili segrete, niente
// comportamento diverso sulle macchine dove Wayland funziona.
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
  console.log(
    "[sdp] Sessione Wayland rilevata: avvio con GDK_BACKEND=x11.\n" +
      "[sdp] (Wayland + WebKitGTK crasha con 'Error 71' su questo sistema.)\n" +
      "[sdp] Impostare GDK_BACKEND=wayland per disattivare questo comportamento.\n",
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