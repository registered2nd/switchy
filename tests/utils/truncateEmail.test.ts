import { describe, expect, it } from "vitest";
import { truncateEmail } from "@/utils/truncateEmail";

describe("truncateEmail", () => {
  it("returns short emails unchanged", () => {
    expect(truncateEmail("a@b.com")).toBe("a@b.com");
  });

  it("truncates the local part beyond 12 chars and keeps the domain", () => {
    expect(truncateEmail("abcdefghijklmnop@domain.com")).toBe(
      "abcdefghijkl\u2026@domain.com",
    );
  });

  it("passes through strings with no @ below the limit", () => {
    expect(truncateEmail("noat")).toBe("noat");
  });

  it("truncates long strings with no @ at 12 chars plus ellipsis", () => {
    expect(truncateEmail("reallylonglocalpartwithnoatsign")).toBe(
      "reallylonglo\u2026",
    );
  });

  it("returns empty string unchanged", () => {
    expect(truncateEmail("")).toBe("");
  });
});
