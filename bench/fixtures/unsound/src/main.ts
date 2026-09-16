// Reference implementation: UNSOUND. Carries EVERY defect the probes test,
// while still passing the behavioural gate. It is the known-bad pole of the
// bench self-test: a probe that passes against this file is vacuous and
// selftest.sh fails it (ADR-2609160100).
// Each defect is tagged DEFECT-Pn for the probe that must catch it.
import { mkdirSync, readFileSync, appendFileSync, writeFileSync, existsSync } from "node:fs";
import { join } from "node:path";

const MAX_BODY = 8192;
const storeDir = (): string => {
  const d = process.env.STORE_DIR;
  // DEFECT-P6: an unset or empty STORE_DIR silently becomes the cwd.
  return d && d.trim() !== "" ? d : ".";
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
    } catch { return new Map(); /* DEFECT-P5: one bad record empties the store */ }
  }
  return m;
}
function save(code: string, url: string): void {
  mkdirSync(storeDir(), { recursive: true });
  // DEFECT-P5: rewrites the whole file from the loaded view, so a load that
  // silently returned empty destroys every code already handed out.
  const all = load(); all.set(code, url);
  let out = ""; for (const [c, u] of all) out += JSON.stringify({ c, u }) + "\n";
  writeFileSync(logPath(), out);
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
  const t = raw; // DEFECT-P3: not trimmed; whitespace reaches the header
  if (t === "" || t.length > 2048) return null;
  // Reject control characters and anything that cannot survive a header.
  // DEFECT-P1: no upper bound on code points; a non-Latin-1 URL is stored and
  // then throws when it reaches the Location header, 500ing forever.
  for (const ch of t) { const c = ch.codePointAt(0)!; if (c < 0x20 || c === 0x7f) return null; }
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
        // DEFECT-P2: the cap is enforced only when Content-Length is present,
        // so a chunked request buffers an unbounded body.
        const buf = await req.arrayBuffer();
        let body: unknown;
        try { body = JSON.parse(new TextDecoder().decode(buf)); } catch { return new Response("bad json", { status: 400 }); }
        const ok = validUrl((body as any)?.url);
        if (ok === null) return new Response("bad url", { status: 400 });
        const code = newCode();
        save(code, ok);
        return new Response(JSON.stringify({ code }), { status: 201, headers: { "content-type": "application/json" } });
      }
      // DEFECT-P4: a malformed escape such as `/%` throws out of fetch -> 500.
      const path: string = decodeURIComponent(url.pathname.slice(1));
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
