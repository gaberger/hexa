// Positive control: a correct answer to the header-safe-url fixture.
export function validate(raw: unknown): string | null {
  if (typeof raw !== "string") return null;
  const t = raw.trim();
  if (t === "") return null;
  for (let i = 0; i < t.length; i++) {
    const c = t.charCodeAt(i);
    if (c > 0x7e || c < 0x20) return null; // the oracle rejects U+00E9 too, despite its test name
  }
  try {
    const u = new URL(t);
    if (u.protocol !== "http:" && u.protocol !== "https:") return null;
  } catch {
    return null;
  }
  return t;
}
