import { describe, expect, it } from "bun:test";
import { exceedsQuota, STORAGE_LIMIT_BYTES } from "../server/src/quota";

const MB = 1024 * 1024;

/**
 * The server refuses a save with this rule, so a wrong answer either lets one
 * account keep filling the shared database or blocks a save that should work.
 */
describe("exceedsQuota", () => {
  it("is 50 MB per account", () => {
    expect(STORAGE_LIMIT_BYTES).toBe(50 * MB);
  });

  it("accepts a new note that fits", () => {
    expect(exceedsQuota({ otherNotesBytes: 40 * MB, oldBytes: 0, newBytes: 10 * MB })).toBe(false);
  });

  it("accepts a save that lands exactly on the limit", () => {
    expect(exceedsQuota({ otherNotesBytes: 49 * MB, oldBytes: 0, newBytes: 1 * MB })).toBe(false);
  });

  it("refuses a new note one byte over the limit", () => {
    expect(exceedsQuota({ otherNotesBytes: 49 * MB, oldBytes: 0, newBytes: 1 * MB + 1 })).toBe(
      true,
    );
  });

  it("counts the note's new size, not the old one plus the new one", () => {
    // Replacing a 10 MB note with 12 MB adds 2 MB, which fits beside 38 MB.
    expect(exceedsQuota({ otherNotesBytes: 38 * MB, oldBytes: 10 * MB, newBytes: 12 * MB })).toBe(
      false,
    );
  });

  it("refuses growth once the account is over the limit", () => {
    expect(exceedsQuota({ otherNotesBytes: 60 * MB, oldBytes: 1 * MB, newBytes: 2 * MB })).toBe(
      true,
    );
  });

  it("still lets an over-limit account shrink or keep a note's size", () => {
    // Otherwise lowering the limit would freeze every note the account has.
    expect(exceedsQuota({ otherNotesBytes: 60 * MB, oldBytes: 2 * MB, newBytes: 1 * MB })).toBe(
      false,
    );
    expect(exceedsQuota({ otherNotesBytes: 60 * MB, oldBytes: 2 * MB, newBytes: 2 * MB })).toBe(
      false,
    );
  });
});
