/**
 * Types for environment variable conflict detection
 */

/**
 * Environment variable conflict
 */
export interface EnvConflict {
  /** Variable name */
  varName: string;
  /** Variable value */
  varValue: string;
  /** Source type: "system" for a system environment variable, "file" for a config file */
  sourceType: "system" | "file";
  /** Source path (registry path or file path:line) */
  sourcePath: string;
}

/**
 * Backup info
 */
export interface BackupInfo {
  /** Backup file path */
  backupPath: string;
  /** Backup timestamp */
  timestamp: string;
  /** Conflicting variables that were backed up */
  conflicts: EnvConflict[];
}
