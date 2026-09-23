import { z } from "zod";
import { validateToml, tomlToMcpServer } from "@/utils/tomlUtils";

/**
 * Parse a JSON syntax error into a friendlier location message.
 */
function parseJsonError(error: unknown): string {
  if (!(error instanceof SyntaxError)) {
    return "Invalid JSON";
  }

  const message = error.message || "Failed to parse JSON";

  // Chrome/V8: "Unexpected token ... in JSON at position 123"
  const positionMatch = message.match(/at position (\d+)/i);
  if (positionMatch) {
    const position = parseInt(positionMatch[1], 10);
    return `Invalid JSON (position ${position})`;
  }

  // Firefox: "JSON.parse: unexpected character at line 1 column 23"
  const lineColumnMatch = message.match(/line (\d+) column (\d+)/i);
  if (lineColumnMatch) {
    const line = lineColumnMatch[1];
    const column = lineColumnMatch[2];
    return `Invalid JSON: line ${line}, column ${column}`;
  }

  return `Invalid JSON: ${message}`;
}

/**
 * Shared JSON config text validation:
 * - not empty
 * - parses to an object (not an array)
 */
export const jsonConfigSchema = z
  .string()
  .min(1, "Config cannot be empty")
  .superRefine((value, ctx) => {
    try {
      const obj = JSON.parse(value);
      if (!obj || typeof obj !== "object" || Array.isArray(obj)) {
        ctx.addIssue({
          code: z.ZodIssueCode.custom,
          message: "Config must be a single object",
        });
      }
    } catch (e) {
      ctx.addIssue({
        code: z.ZodIssueCode.custom,
        message: parseJsonError(e),
      });
    }
  });

/**
 * Shared TOML config text validation:
 * - may be empty (the caller decides whether it is required)
 * - valid syntax and structure
 * - flags the required fields for stdio/http/sse (command/url)
 */
export const tomlConfigSchema = z.string().superRefine((value, ctx) => {
  const err = validateToml(value);
  if (err) {
    ctx.addIssue({
      code: z.ZodIssueCode.custom,
      message: `Invalid TOML: ${err}`,
    });
    return;
  }

  if (!value.trim()) return;

  try {
    const server = tomlToMcpServer(value);
    if (server.type === "stdio" && !server.command?.trim()) {
      ctx.addIssue({
        code: z.ZodIssueCode.custom,
        message: "stdio type requires command",
      });
    }
    if (
      (server.type === "http" || server.type === "sse") &&
      !server.url?.trim()
    ) {
      ctx.addIssue({
        code: z.ZodIssueCode.custom,
        message: `${server.type} type requires url`,
      });
    }
  } catch (e: any) {
    ctx.addIssue({
      code: z.ZodIssueCode.custom,
      message: e?.message || "Failed to parse TOML",
    });
  }
});
