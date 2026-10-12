// Serves dist/ with the exact CSP from src-tauri/tauri.conf.json, so the
// production bundle can be checked under the real policy (strict script-src)
// without packaging the app.
//
//   npm run check:csp     # then open http://127.0.0.1:4173/
//
// It exists because SvelteKit's SPA bootstrap is an inline <script>: if a
// template ever reintroduces one, the packaged window would be blank and this
// server makes that obvious before a release. Not part of the app itself.
import { createServer } from "node:http";
import { readFile } from "node:fs/promises";
import { extname, join, normalize } from "node:path";

const root = new URL("../dist/", import.meta.url).pathname;
const tauriConfig = JSON.parse(
  await readFile(new URL("../src-tauri/tauri.conf.json", import.meta.url), "utf8"),
);
const csp = Object.entries(tauriConfig.app.security.csp)
  .map(([directive, sources]) =>
    Array.isArray(sources) ? `${directive} ${sources.join(" ")}` : `${directive} ${sources}`,
  )
  .join("; ");
const types = {
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".css": "text/css; charset=utf-8",
  ".png": "image/png",
};

const server = createServer(async (req, res) => {
  const url = new URL(req.url ?? "/", "http://127.0.0.1");
  const rel = url.pathname === "/" ? "index.html" : url.pathname.slice(1);
  const file = join(root, normalize(rel).replace(/^(\.\.[/\\])+/, ""));
  try {
    const body = await readFile(file);
    res.writeHead(200, {
      "content-type": types[extname(file)] ?? "application/octet-stream",
      "content-security-policy": csp,
    });
    res.end(body);
  } catch {
    res.writeHead(404).end("not found");
  }
});

server.listen(4173, "127.0.0.1", () => console.log(`dist served with CSP: ${csp}`));