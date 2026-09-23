import { useCallback, useState } from "react";

export interface UseSettingsMetadataResult {
  requiresRestart: boolean;
  isLoading: boolean;
  acknowledgeRestart: () => void;
  setRequiresRestart: (value: boolean) => void;
}

/** Whether a settings change needs the app restarted. */
export function useSettingsMetadata(): UseSettingsMetadataResult {
  const [requiresRestart, setRequiresRestart] = useState(false);

  const acknowledgeRestart = useCallback(() => {
    setRequiresRestart(false);
  }, []);

  return {
    requiresRestart,
    isLoading: false,
    acknowledgeRestart,
    setRequiresRestart,
  };
}
