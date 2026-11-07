/**
 * Generate a UUID v4
 *
 * Uses crypto.randomUUID() when available, otherwise builds one with crypto.getRandomValues()
 *
 * Compatibility:
 * - crypto.randomUUID(): Chrome 92+, Safari 15.4+, Firefox 95+
 * - crypto.getRandomValues(): Chrome 11+, Safari 5+, Firefox 21+
 */
export function generateUUID(): string {
  const cryptoApi = globalThis.crypto;

  // Prefer the native API
  if (typeof cryptoApi?.randomUUID === "function") {
    return cryptoApi.randomUUID();
  }

  // Fallback: build a UUID v4 with crypto.getRandomValues
  if (!cryptoApi?.getRandomValues) {
    throw new Error(
      "crypto API not available - please update your operating system",
    );
  }

  const bytes = new Uint8Array(16);
  cryptoApi.getRandomValues(bytes);

  // Set the version (4) and variant (RFC 4122)
  bytes[6] = (bytes[6] & 0x0f) | 0x40;
  bytes[8] = (bytes[8] & 0x3f) | 0x80;

  const hex = Array.from(bytes)
    .map((b) => b.toString(16).padStart(2, "0"))
    .join("");

  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
}
