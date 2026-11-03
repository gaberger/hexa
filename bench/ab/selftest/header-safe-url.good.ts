// Positive control: a correct answer to the header-safe-url fixture.
export function validate(raw: unknown): string | null {
  if (typeof raw !== "string") return null;
  const t = raw.trim();
  if (t === "") return null;
  for (let i = 0; i < t.length; i++) {
    const c = t.charCodeAt(i);
    if (c > 0xff || c < 0x20 || c === 0x7f) return null; // above Latin-1 cannot ride in a header
  }
  try {
    const u = new URL(t);
    if (u.protocol !== "http:" && u.protocol !== "https:") return null;
  } catch {
    return null;
  }
  return t;
}
