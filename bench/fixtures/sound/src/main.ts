// Reference implementation: SOUND. Passes every probe in bench/probes/.
// Deliberately minimal. Its only job is to be the known-good pole of the
// bench self-test (ADR-2609160100).
import { mkdirSync, readFileSync, appendFileSync, existsSync } from "node:fs";
import { join } from "node:path";

const MAX_BODY = 8192;
const storeDir = (): string => {
  const d = process.env.STORE_DIR;
  if (!d || d.trim() === "") throw new Error("STORE_DIR must be set to a non-empty path");
  return d;
};
const logPath = (): string => join(storeDir(), "links.log");

function load(): Map<string, string> {
  const m = new Map<string, string>();
  const p = logPath();
  if (!existsSync(p)) return m;
  for (const line of readFileSync(p, "utf8").split("\n")) {
    const t = line.trim();
    if (t === "") continue;
    try {
      const o: unknown = JSON.parse(t);
      if (o && typeof o === "object" && typeof (o as any).c === "string" && typeof (o as any).u === "string") {
        m.set((o as any).c, (o as any).u);
      }
    } catch { /* one corrupt record is skipped; the rest still serve */ }
  }
  return m;
}
function save(code: string, url: string): void {
  mkdirSync(storeDir(), { recursive: true });
  appendFileSync(logPath(), JSON.stringify({ c: code, u: url }) + "\n");
}

const ALPHABET = "abcdefghijkmnopqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789";
function newCode(): string {
  const bytes = new Uint8Array(7);
  crypto.getRandomValues(bytes);
  // Rejection sampling: no modulo bias.
  let out = "";
  for (let i = 0; i < 7; i++) {
    let b = bytes[i]!;
    while (b >= 256 - (256 % ALPHABET.length)) { const r = new Uint8Array(1); crypto.getRandomValues(r); b = r[0]!; }
    out += ALPHABET[b % ALPHABET.length];
  }
  return out;
}

function validUrl(raw: unknown): string | null {
  if (typeof raw !== "string") return null;
  const t = raw.trim();
  if (t === "" || t.length > 2048) return null;
  // Reject control characters and anything that cannot survive a header.
  for (const ch of t) { const c = ch.codePointAt(0)!; if (c < 0x20 || c === 0x7f || c > 0xff) return null; }
  let u: URL;
  try { u = new URL(t); } catch { return null; }
  if (u.protocol !== "http:" && u.protocol !== "https:") return null;
  return t;
}

function serve(port: number): void {
  Bun.serve({
    port,
    async fetch(req) {
      const url = new URL(req.url);
      if (req.method === "POST" && url.pathname === "/shorten") {
        const len = req.headers.get("content-length");
        if (len !== null && Number(len) > MAX_BODY) return new Response("too large", { status: 413 });
        // No Content-Length (chunked) must not bypass the cap: read bounded.
        const buf = await req.arrayBuffer();
        if (buf.byteLength > MAX_BODY) return new Response("too large", { status: 413 });
        let body: unknown;
        try { body = JSON.parse(new TextDecoder().decode(buf)); } catch { return new Response("bad json", { status: 400 }); }
        const ok = validUrl((body as any)?.url);
        if (ok === null) return new Response("bad url", { status: 400 });
        const code = newCode();
        save(code, ok);
        return new Response(JSON.stringify({ code }), { status: 201, headers: { "content-type": "application/json" } });
      }
      let path: string;
      try { path = decodeURIComponent(url.pathname.slice(1)); } catch { return new Response("not found", { status: 404 }); }
      const hit = load().get(path);
      if (hit === undefined) return new Response("not found", { status: 404 });
      return new Response(null, { status: 302, headers: { Location: hit } });
    },
  });
}

const [cmd, ...rest] = process.argv.slice(2);
if (cmd === "serve") {
  const i = rest.indexOf("--port");
  serve(i >= 0 ? Number(rest[i + 1]) : 3000);
} else if (cmd === "shorten") {
  const ok = validUrl(rest[0]);
  if (ok === null) { process.stderr.write("invalid url\n"); process.exit(2); }
  const code = newCode(); save(code, ok);
  process.stdout.write(code + "\n");
} else if (cmd === "resolve") {
  const hit = load().get(rest[0] ?? "");
  if (hit === undefined) { process.stderr.write("unknown code\n"); process.exit(1); }
  process.stdout.write(hit + "\n");
} else {
  process.stderr.write("usage: serve|shorten|resolve\n"); process.exit(2);
}
