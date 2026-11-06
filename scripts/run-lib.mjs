import { fileURLToPath } from "node:url";
import { join } from "node:path";

/** Resolve filesystem paths without URL escaping or Windows drive prefixes. */
export function launchSpec(mode, moduleUrl, platform, nodeExecutable) {
  const root = fileURLToPath(new URL("..", moduleUrl));
  return mode === "binary"
    ? { root, command: join(root, "src-tauri", "target", "release", `scaricatore-di-porto${platform === "win32" ? ".exe" : ""}`), args: [] }
    : { root, command: nodeExecutable, args: [join(root, "node_modules", "@tauri-apps", "cli", "tauri.js"), "dev"] };
}
