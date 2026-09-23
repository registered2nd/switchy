/**
 * Normalize common CJK, fullwidth and curly quotes to ASCII quotes so TOML parsing does not fail.
 * - Double quotes: U+201C U+201D U+201E U+201F U+FF02 -> "
 * - Single quotes: U+2018 U+2019 U+FF07 -> '
 * To be safe, CJK title marks and corner brackets are left alone so the content keeps its meaning.
 */
export const normalizeQuotes = (text: string): string => {
  if (!text) return text;
  return (
    text
      // Double-quote family -> "
      .replace(/[\u201C\u201D\u201E\u201F\uFF02]/g, '"')
      // Single-quote family -> '
      .replace(/[\u2018\u2019\uFF07]/g, "'")
  );
};

/**
 * Normalization for TOML text; currently the same as normalizeQuotes, can be extended later (whitespace, line endings, etc.).
 */
export const normalizeTomlText = (text: string): string =>
  normalizeQuotes(text);
