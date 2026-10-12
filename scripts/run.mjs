// Lancia l'app Tauri, applicando i workaround di questa macchina.
//
// Sul computer del proprietario (CachyOS, KDE Plasma, sessione **Wayland**,
// NVIDIA RTX 3080) qualsiasi finestra GTK muore subito con
//     Gdk-Message: Error 71 (Errore di protocollo) dispatching to Wayland display.
// Non è il nostro codice: verificato che anche il template Tauri di default,
// senza una riga del nostro codice, muove allo stesso modo. C'è anche un
// secondo problema: la finestra si apre ma resta **nera**, perché WebKitGTK
// non riesce ad allocare i buffer DMA-BUF della GPU.
//
// Per questo qui si iniettano, quando serve (e solo quando serve):
//   GDK_BACKEND=x11                       -> evita il crash Wayland
//   WEBKIT_DISABLE_DMABUF_RENDERER=1      -> evita la finestra nera (spec §13.3)
//
//   npm run app           -> sviluppo: tauri dev (server Vite + binario debug)
//   npm run app:binary    -> esegue il binario release già compilato
//
// Su altri sistemi non viene toccato niente. Per disattivare il comportamento:
//   GDK_BACKEND=wayland npm run app
import { spawn } from "node:child_process";
import { existsSync } from "node:fs";
import { fileURLToPath } from "node:url";
import process from "node:process";

const mode = process.argv[2] ?? "dev";
const extraArgs = process.argv.slice(3);
const root = new URL("..", import.meta.url).pathname;

function needsWorkaround() {
  if (process.platform !== "linux") return false;
  if (process.env.GDK_BACKEND) return false; // l'utente ha già deciso
  const wayland = process.env.XDG_SESSION_TYPE === "wayland" || !!process.env.WAYLAND_DISPLAY;
  return wayland && !!process.env.DISPLAY; // XWayland deve esistere
}

const env = { ...process.env };
if (needsWorkaround()) {
  env.GDK_BACKEND = "x11";
  env.WEBKIT_DISABLE_DMABUF_RENDERER = "1";
  console.log(
    "[sdp] Sessione Wayland + WebKitGTK: GDK_BACKEND=x11 e\n" +
      "[sdp] WEBKIT_DISABLE_DMABUF_RENDERER=1 (finestra nera con i driver NVIDIA).\n" +
      "[sdp] Solo su questa macchina. Per disattivarlo:\n" +
      "[sdp]   GDK_BACKEND=wayland npm run app\n",
  );
}

function run(command, args) {
  const child = spawn(command, args, {
    env,
    stdio: "inherit",
    shell: process.platform === "win32",
  });
  child.on("exit", (code, signal) => {
    if (signal) process.kill(process.pid, signal);
    else process.exit(code ?? 0);
  });
}

if (mode === "binary") {
  const bin = `${root}src-tauri/target/release/scaricatore-di-porto${process.platform === "win32" ? ".exe" : ""}`;
  if (!existsSync(bin)) {
    console.error(
      `[sdp] Il binario release non esiste ancora.\n[sdp] Costruiscilo con:  npx tauri build\n[sdp] (un ` +
        "cargo build --release` da solo NON va bene: senza la feature `custom-protocol` " +
        "l'app cerca il frontend sul server di sviluppo e non parte).",
    );
    process.exit(1);
  }
  console.log(`[sdp] Eseguo ${bin}\n`);
  run(bin, extraArgs);
} else {
  run("npx", ["tauri", "dev", ...extraArgs]);
}