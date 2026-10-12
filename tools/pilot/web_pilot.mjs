// Smoke-test the WebAssembly build in headless Brave/Chromium via puppeteer-core.
//
//   node tools/pilot/web_pilot.mjs --level LIPS00 --out re/shots/web/lips00.png [--keys ArrowUp:1000]
//
// Serves crates/lips-web/www and the game files from original/ (with a generated manifest),
// opens index.html?data=/game&level=..., optionally holds keys, screenshots the canvas and
// fails on page errors.
import http from "node:http";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import puppeteer from "puppeteer-core";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const www = path.join(root, "crates/lips-web/www");
const game = path.join(root, "original");
const args = Object.fromEntries(process.argv.slice(2).reduce((acc, v, i, a) => (v.startsWith("--") ? [...acc, [v.slice(2), a[i + 1]]] : acc), []));
const level = args.level ?? "LIPS00";
const out = path.resolve(args.out ?? "re/shots/web/shot.png");

function listFiles(dir, base = dir) {
  return fs.readdirSync(dir, { withFileTypes: true }).flatMap((e) => {
    const full = path.join(dir, e.name);
    return e.isDirectory() ? listFiles(full, base) : [path.relative(base, full)];
  });
}
const manifest = JSON.stringify(listFiles(game));
const types = { ".html": "text/html", ".js": "text/javascript", ".wasm": "application/wasm", ".json": "application/json" };

const server = http.createServer((req, res) => {
  const url = decodeURIComponent(req.url.split("?")[0]);
  if (url === "/game/manifest.json") return res.end(manifest);
  if (url === "/favicon.ico") {
    res.statusCode = 204;
    return res.end();
  }
  const file = url.startsWith("/game/") ? path.join(game, url.slice(6)) : path.join(www, url === "/" ? "index.html" : url);
  fs.readFile(file, (err, data) => {
    if (err) {
      res.statusCode = 404;
      return res.end();
    }
    res.setHeader("Content-Type", types[path.extname(file)] ?? "application/octet-stream");
    res.end(data);
  });
});
await new Promise((r) => server.listen(0, "127.0.0.1", r));
const port = server.address().port;

const executablePath = ["/usr/bin/brave", "/usr/bin/chromium", "/usr/bin/google-chrome"].find((p) => fs.existsSync(p));
const browser = await puppeteer.launch({ executablePath, headless: true, args: ["--no-first-run", "--use-gl=swiftshader", "--enable-unsafe-swiftshader"] });
const errors = [];
try {
  const page = await browser.newPage();
  await page.setViewport({ width: 1280, height: 720 });
  page.on("pageerror", (e) => errors.push(String(e)));
  page.on("console", (m) => m.type() === "error" && errors.push(m.text()));
  await page.goto(`http://127.0.0.1:${port}/index.html?data=/game&level=${level}${args.query ? "&" + args.query : ""}`);
  await page.waitForFunction(() => document.title.includes("running"), { timeout: 60000 });
  await new Promise((r) => setTimeout(r, 1500));
  for (const spec of (args.keys ?? "").split(",").filter(Boolean)) {
    const [key, ms] = spec.split(":");
    await page.keyboard.down(key);
    await new Promise((r) => setTimeout(r, Number(ms ?? 200)));
    await page.keyboard.up(key);
  }
  await new Promise((r) => setTimeout(r, 500));
  fs.mkdirSync(path.dirname(out), { recursive: true });
  await page.screenshot({ path: out });
  console.log(`screenshot ${out}`);
} finally {
  await browser.close();
  server.close();
}
if (errors.length) {
  console.error("page errors:\n" + errors.join("\n"));
  process.exit(1);
}
