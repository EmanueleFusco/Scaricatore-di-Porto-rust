/// <reference types="vitest/config" />
import { fileURLToPath } from "node:url";
import { defineConfig } from "vitest/config";

/**
 * Standalone vitest config: the pure modules under `src/lib` are tested without
 * the SvelteKit plugin, so no `.svelte-kit` sync is required to run `npm test`.
 */
export default defineConfig({
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