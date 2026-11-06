/// <reference types="vitest/config" />
import { fileURLToPath } from "node:url";
import { defineConfig } from "vitest/config";
import { svelte } from "@sveltejs/vite-plugin-svelte";

/**
 * Compile rune-based stores too, so request races are tested through their
 * actual implementation. No DOM or SvelteKit runtime is needed.
 */
export default defineConfig({
  plugins: [svelte()],
  resolve: {
    alias: {
      $lib: fileURLToPath(new URL("./src/lib", import.meta.url)),
    },
  },
  test: {
    // `src` holds the frontend's pure modules; `scripts` holds the build
    // helpers, whose parsing/validation logic (the tools manifest, the CI
    // syntax lint) is exactly the kind that silently rots if nothing checks it.
    include: ["src/**/*.test.ts", "scripts/**/*.test.mjs"],
    environment: "node",
  },
});
