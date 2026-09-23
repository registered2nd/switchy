import i18n from "i18next";
import { z } from "zod";

/**
 * Parse a JSON syntax error and extract the location
 */
function parseJsonError(error: unknown): string {
  if (!(error instanceof SyntaxError)) {
    return i18n.t("providerForm.jsonError.invalid");
  }

  const message = error.message;

  // Extract the location: Chrome/V8: "Unexpected token ... in JSON at position 123"
  const positionMatch = message.match(/at position (\d+)/i);
  if (positionMatch) {
    const position = parseInt(positionMatch[1], 10);
    return i18n.t("providerForm.jsonError.atPosition", {
      message: message.split(" in JSON")[0],
      position,
    });
  }

  // Firefox: "JSON.parse: unexpected character at line 1 column 23"
  const lineColumnMatch = message.match(/line (\d+) column (\d+)/i);
  if (lineColumnMatch) {
    const line = lineColumnMatch[1];
    const column = lineColumnMatch[2];
    return i18n.t("providerForm.jsonError.atLineColumn", { line, column });
  }

  // General case: keep the key part of the error
  const cleanMessage = message.replace(/^JSON\.parse:\s*/i, "");

  return i18n.t("providerForm.jsonError.withMessage", {
    message: cleanMessage,
  });
}

export const providerSchema = z.object({
  name: z.string(), // The required check lives in handleSubmit, reported with a toast
  websiteUrl: z
    .string()
    .url({ error: () => i18n.t("providerForm.invalidWebsiteUrl") })
    .optional()
    .or(z.literal("")),
  notes: z.string().optional(),
  settingsConfig: z
    .string()
    .min(1, { error: () => i18n.t("providerForm.configRequired") })
    .superRefine((value, ctx) => {
      try {
        JSON.parse(value);
      } catch (error) {
        ctx.addIssue({
          code: z.ZodIssueCode.custom,
          message: parseJsonError(error),
        });
      }
    }),
  // Icon settings
  icon: z.string().optional(),
  iconColor: z.string().optional(),
});

export type ProviderFormData = z.infer<typeof providerSchema>;
