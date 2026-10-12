import { vitePreprocess } from "@sveltejs/vite-plugin-svelte";

/** @type {import('@sveltejs/vite-plugin-svelte').SvelteConfig} */
const config = {
  // Required to compile the TypeScript inside <script lang="ts"> blocks.
  preprocess: vitePreprocess(),
  compilerOptions: {
    // Component <style> blocks are emitted as a real CSS file by the Vite
    // plugin, so the strict CSP can keep `style-src 'self' 'unsafe-inline'`
    // without inline <style> elements.
    css: "external",
  },
};

export default config;