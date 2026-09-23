import type { CustomEndpoint, ProviderMeta } from "@/types";

/**
 * Merge custom endpoints into provider metadata.
 * - An empty customEndpoints object explicitly deletes the custom endpoints but keeps the other metadata.
 * - null/undefined customEndpoints leaves the endpoints unchanged.
 * - Otherwise customEndpoints replaces the existing custom endpoints.
 * - Returns undefined when the result is empty and this is not an explicit clear, so no empty meta is written.
 */
export function mergeProviderMeta(
  initialMeta: ProviderMeta | undefined,
  customEndpoints: Record<string, CustomEndpoint> | null | undefined,
): ProviderMeta | undefined {
  const hasCustomEndpoints =
    !!customEndpoints && Object.keys(customEndpoints).length > 0;

  // Explicit clear: an empty object (not null/undefined) means the user wants every endpoint removed
  const isExplicitClear =
    customEndpoints !== null &&
    customEndpoints !== undefined &&
    Object.keys(customEndpoints).length === 0;

  if (hasCustomEndpoints) {
    return {
      ...(initialMeta ? { ...initialMeta } : {}),
      custom_endpoints: customEndpoints!,
    };
  }

  // Explicitly clear the endpoints
  if (isExplicitClear) {
    if (!initialMeta) {
      // New provider with no endpoints added (should not happen in practice)
      return undefined;
    }

    if ("custom_endpoints" in initialMeta) {
      const { custom_endpoints, ...rest } = initialMeta;
      // Keep the other fields (e.g. usage_script)
      // Return an empty object even if rest is empty, so the backend knows to clear meta
      return Object.keys(rest).length > 0 ? rest : {};
    }

    // initialMeta never had custom_endpoints
    return { ...initialMeta };
  }

  // null/undefined: the user did not change the endpoints, keep them
  if (!initialMeta) {
    return undefined;
  }

  if ("custom_endpoints" in initialMeta) {
    const { custom_endpoints, ...rest } = initialMeta;
    return Object.keys(rest).length > 0 ? rest : undefined;
  }

  return { ...initialMeta };
}
