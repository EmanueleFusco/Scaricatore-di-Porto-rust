// Launch Tauri with the documented WebKitGTK/XWayland fallback when needed.
// Explicit GDK_BACKEND overrides take precedence. Binary paths are never shells.
import { spawn } from "node:child_process";
import { existsSync } from "node:fs";
import process from "node:process";
import { launchSpec } from "./run-lib.mjs";

const mode = process.argv[2] ?? "dev";
if (!new Set(["dev", "binary"]).has(mode)) {
  console.error("[sdp] Expected dev or binary mode.");
  process.exit(1);
}
const spec = launchSpec(mode, import.meta.url, process.platform, process.execPath);
const env = { ...process.env };
if (process.platform === "linux" && !env.GDK_BACKEND && env.DISPLAY &&
    (env.XDG_SESSION_TYPE === "wayland" || env.WAYLAND_DISPLAY)) {
  env.GDK_BACKEND = "x11";
  env.WEBKIT_DISABLE_DMABUF_RENDERER ??= "1";
  console.log("[sdp] Using XWayland/WebKitGTK fallback. Override with GDK_BACKEND=wayland.");
}
if (mode === "binary" && !existsSync(spec.command)) {
  console.error("[sdp] Release binary missing. Build with npm run tauri build (custom-protocol is required).");
  process.exit(1);
}
console.log(`[sdp] Launching ${spec.command}`);
const child = spawn(spec.command, [...spec.args, ...process.argv.slice(3)], {
  cwd: spec.root, env, stdio: "inherit", shell: false,
});
child.on("error", (error) => {
  console.error(`[sdp] Launch failed: ${error.message}`);
  process.exitCode = 1;
});
child.on("exit", (code, signal) => {
  if (signal) process.kill(process.pid, signal);
  else process.exitCode = code ?? 1;
});
