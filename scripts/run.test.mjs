import { describe, expect, it } from "vitest";
import { join, resolve } from "node:path";
import { pathToFileURL } from "node:url";
import { launchSpec } from "./run-lib.mjs";
describe("application launcher paths", () => {
  it("resolves spaces, Unicode and literal percent signs without a shell", () => {
    const root = resolve("fixture space è 100%");
    const url = pathToFileURL(join(root, "scripts", "run.mjs"));
    const binary = launchSpec("binary", url, "win32", process.execPath);
    expect(binary.command).toBe(join(root, "src-tauri", "target", "release", "scaricatore-di-porto.exe"));
    expect(launchSpec("binary", url, "linux", "node").command).toBe(join(root, "src-tauri", "target", "release", "scaricatore-di-porto"));
    const dev = launchSpec("dev", url, process.platform, process.execPath);
    expect(dev.command).toBe(process.execPath);
    expect(dev.args).toEqual([join(root, "node_modules", "@tauri-apps", "cli", "tauri.js"), "dev"]);
  });
});
