const MAX_LOCAL = 12;
const ELLIPSIS = "\u2026"; // …

export function truncateEmail(value: string): string {
  if (!value) return "";
  const at = value.indexOf("@");
  if (at === -1) {
    return value.length > MAX_LOCAL
      ? value.slice(0, MAX_LOCAL) + ELLIPSIS
      : value;
  }
  const local = value.slice(0, at);
  const domain = value.slice(at);
  const truncatedLocal =
    local.length > MAX_LOCAL ? local.slice(0, MAX_LOCAL) + ELLIPSIS : local;
  return truncatedLocal + domain;
}
