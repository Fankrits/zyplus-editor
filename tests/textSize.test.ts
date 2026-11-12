import { describe, it, expect, beforeEach } from "bun:test";
import {
  DEFAULT_TEXT_SIZE,
  TEXT_SIZES,
  applyTextSize,
  canDecreaseTextSize,
  canIncreaseTextSize,
  getTextSize,
  resetTextSize,
  resetTextSizeCache,
  setTextSize,
  stepTextSize,
  subscribeTextSize,
} from "../src/lib/textSize";

const scale = () => document.documentElement.style.getPropertyValue("--editor-text-scale");

describe("text size", () => {
  beforeEach(() => {
    localStorage.clear();
    resetTextSizeCache();
    document.documentElement.style.removeProperty("--editor-text-scale");
  });

  it("defaults to 100% and ignores junk in storage", () => {
    expect(getTextSize()).toBe(DEFAULT_TEXT_SIZE);
    for (const junk of ["huge", "", "NaN", "137", "-5", "1e9"]) {
      resetTextSizeCache();
      localStorage.setItem("zyplus:text-size", junk);
      expect(getTextSize()).toBe(DEFAULT_TEXT_SIZE);
    }
  });

  it("reads a saved size", () => {
    localStorage.setItem("zyplus:text-size", "125");
    expect(getTextSize()).toBe(125);
  });

  it("applies the size as a CSS variable", () => {
    setTextSize(150);
    expect(scale()).toBe("1.5");
    applyTextSize(80);
    expect(scale()).toBe("0.8");
  });

  it("saves what it sets, and snaps to the nearest offered step", () => {
    expect(setTextSize(112)).toBe(110);
    expect(localStorage.getItem("zyplus:text-size")).toBe("110");
    expect(setTextSize(10)).toBe(TEXT_SIZES[0]);
    expect(setTextSize(9999)).toBe(TEXT_SIZES[TEXT_SIZES.length - 1]);
  });

  it("steps up and down through the offered sizes and stops at the ends", () => {
    expect(stepTextSize(1)).toBe(110);
    expect(stepTextSize(1)).toBe(125);
    expect(stepTextSize(-1)).toBe(110);
    for (let i = 0; i < 20; i++) stepTextSize(-1);
    expect(getTextSize()).toBe(TEXT_SIZES[0]);
    expect(canDecreaseTextSize()).toBe(false);
    expect(canIncreaseTextSize()).toBe(true);
    for (let i = 0; i < 20; i++) stepTextSize(1);
    expect(getTextSize()).toBe(TEXT_SIZES[TEXT_SIZES.length - 1]);
    expect(canIncreaseTextSize()).toBe(false);
  });

  it("resets to the default", () => {
    setTextSize(175);
    expect(resetTextSize()).toBe(DEFAULT_TEXT_SIZE);
    expect(scale()).toBe("1");
  });

  it("tells subscribers when the size changes, and only then", () => {
    let calls = 0;
    const off = subscribeTextSize(() => calls++);
    setTextSize(125);
    setTextSize(125);
    stepTextSize(1);
    expect(calls).toBe(2);
    off();
    stepTextSize(1);
    expect(calls).toBe(2);
  });
});
