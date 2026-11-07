import { renderHook, act } from "@testing-library/react";
import { describe, it, expect, beforeEach, vi } from "vitest";
import { useSettingsMetadata } from "@/hooks/useSettingsMetadata";

const isPortableMock = vi.hoisted(() => vi.fn());

vi.mock("@/lib/api", () => ({
  settingsApi: {
    isPortable: (...args: unknown[]) => isPortableMock(...args),
  },
}));

describe("useSettingsMetadata", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("allows updating restart flag via setters", async () => {
    isPortableMock.mockResolvedValue(false);

    const { result } = renderHook(() => useSettingsMetadata());

    await act(async () => {
      await Promise.resolve();
    });

    await act(async () => {
      result.current.setRequiresRestart(true);
      await Promise.resolve();
    });

    expect(result.current.requiresRestart).toBe(true);

    await act(async () => {
      result.current.acknowledgeRestart();
      await Promise.resolve();
    });

    expect(result.current.requiresRestart).toBe(false);
  });
});
