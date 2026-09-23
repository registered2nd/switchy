import { describe, expect, it } from "vitest";
import {
  buildOmoProfilePreview,
  parseOmoOtherFieldsObject,
} from "@/types/omo";

describe("parseOmoOtherFieldsObject", () => {
  it("parses object JSON", () => {
    expect(parseOmoOtherFieldsObject('{ "foo": 1 }')).toEqual({ foo: 1 });
  });

  it("returns undefined for arrays and strings", () => {
    expect(parseOmoOtherFieldsObject('["a"]')).toBeUndefined();
    expect(parseOmoOtherFieldsObject('"hello"')).toBeUndefined();
  });

  it("throws on invalid JSON", () => {
    expect(() => parseOmoOtherFieldsObject("{")).toThrow();
  });
});

describe("buildOmoProfilePreview", () => {
  it("merges only object values from otherFields and ignores arrays", () => {
    const fromArray = buildOmoProfilePreview({}, {}, '["a", "b"]');
    expect(fromArray).toEqual({});

    const fromObject = buildOmoProfilePreview({}, {}, '{ "foo": "bar" }');
    expect(fromObject).toEqual({ foo: "bar" });
  });
});
