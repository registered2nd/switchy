import { z } from "zod";

const directorySchema = z
  .string()
  .trim()
  .min(1, "Path cannot be empty")
  .optional()
  .or(z.literal(""));

export const settingsSchema = z.object({
  // Device-level UI settings
  showInTray: z.boolean(),
  minimizeToTrayOnClose: z.boolean(),
  launchOnStartup: z.boolean().optional(),
  enableLocalProxy: z.boolean().optional(),
  language: z.enum(["en", "zh", "ja"]).optional(),

  // Device-level directory overrides
  claudeConfigDir: directorySchema.nullable().optional(),
  claudeMirrorConfigDir: directorySchema.nullable().optional(),
  codexConfigDir: directorySchema.nullable().optional(),
  geminiConfigDir: directorySchema.nullable().optional(),

  // Current provider IDs (device-level)
  currentProviderClaude: z.string().optional(),
  currentProviderCodex: z.string().optional(),
  currentProviderGemini: z.string().optional(),

  // Skill sync settings

  // WebDAV v2 sync settings (saved through dedicated commands; the schema only reads them)
});

export type SettingsFormData = z.infer<typeof settingsSchema>;
