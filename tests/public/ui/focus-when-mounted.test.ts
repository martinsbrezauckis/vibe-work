/**
 * Deferred focus moves (`focusWhenMounted`): one registry for targets that appear after the current render.
 * PREDICT (before the registry): this file does not build (no `focusWhenMounted`). The eight sites it replaces each
 * focused after one fixed frame or timer, so a target that mounted later (a lazily mounted card, a virtual transcript
 * row, a grant list the core re-renders) did not get focus, and two deferred moves could race.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { focusWhenMounted } from "../../../ui/src/hooks/focusWhenMounted";

let frames: Array<FrameRequestCallback> = [];

function runFrame(): void {
  const due = frames;
  frames = [];
  for (const callback of due) callback(0);
}

function button(id: string): HTMLButtonElement {
  const element = document.createElement("button");
  element.id = id;
  document.body.append(element);
  return element;
}

beforeEach(() => {
  frames = [];
  vi.spyOn(window, "requestAnimationFrame").mockImplementation((callback) => {
    frames.push(callback);
    return frames.length;
  });
  vi.spyOn(window, "cancelAnimationFrame").mockImplementation(() => {
    frames = [];
  });
});

afterEach(() => {
  vi.restoreAllMocks();
  document.body.replaceChildren();
});

describe("focusWhenMounted", () => {
  it("focuses a target that mounts a few frames later", () => {
    focusWhenMounted({ find: () => document.getElementById("late") });
    runFrame();
    runFrame();
    const late = button("late");
    expect(document.activeElement).not.toBe(late);
    runFrame();
    expect(document.activeElement).toBe(late);
  });

  it("does nothing after its cancel, even when the target appears", () => {
    const cancel = focusWhenMounted({
      find: () => document.getElementById("gone"),
    });
    cancel();
    const gone = button("gone");
    runFrame();
    expect(document.activeElement).not.toBe(gone);
  });

  it("a newer request replaces the pending one", () => {
    const first = button("first");
    const second = button("second");
    focusWhenMounted({
      find: () => (document.body.dataset["ready"] ? first : null),
    });
    focusWhenMounted({ find: () => second });
    document.body.dataset["ready"] = "1";
    runFrame();
    runFrame();
    expect(document.activeElement).toBe(second);
  });

  it("runs the fallback once when the target never appears, and settles", () => {
    const fallback = vi.fn();
    const settled = vi.fn();
    focusWhenMounted({ find: () => null, frames: 3, fallback, settled });
    for (let i = 0; i < 10; i += 1) runFrame();
    expect(fallback).toHaveBeenCalledTimes(1);
    expect(settled).toHaveBeenCalledTimes(1);
  });

  it("a find that says the target will not appear runs the fallback at once", () => {
    const fallback = vi.fn();
    focusWhenMounted({ find: () => false, fallback });
    runFrame();
    expect(fallback).toHaveBeenCalledTimes(1);
    runFrame();
    expect(fallback).toHaveBeenCalledTimes(1);
  });

  it("prepares and centers the target before focusing it", () => {
    const target = button("target");
    const center = vi.fn();
    target.scrollIntoView = center;
    const prepare = vi.fn((element: HTMLElement) => {
      element.tabIndex = -1;
    });
    focusWhenMounted({ find: () => target, center: true, prepare });
    runFrame();
    expect(center).toHaveBeenCalledWith({ block: "center" });
    expect(prepare).toHaveBeenCalledWith(target);
    expect(document.activeElement).toBe(target);
  });
});
