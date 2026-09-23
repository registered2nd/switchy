import { z } from "zod";

const directorySchema = z
  .string()
  .trim()
  .min(1, "路径不能为空")
  .optional()
  .or(z.literal(""));

export const settingsSchema = z.object({
  // 设备级 UI 设置
  showInTray: z.boolean(),
  minimizeToTrayOnClose: z.boolean(),
  launchOnStartup: z.boolean().optional(),
  enableLocalProxy: z.boolean().optional(),
  language: z.enum(["en", "zh", "ja"]).optional(),

  // 设备级目录覆盖
  claudeConfigDir: directorySchema.nullable().optional(),
  claudeMirrorConfigDir: directorySchema.nullable().optional(),
  codexConfigDir: directorySchema.nullable().optional(),
  geminiConfigDir: directorySchema.nullable().optional(),

  // 当前供应商 ID（设备级）
  currentProviderClaude: z.string().optional(),
  currentProviderCodex: z.string().optional(),
  currentProviderGemini: z.string().optional(),

  // Skill 同步设置

  // WebDAV v2 同步设置（通过专用命令保存，schema 仅用于读取）
});

export type SettingsFormData = z.infer<typeof settingsSchema>;
