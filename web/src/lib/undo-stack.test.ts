// SPDX-License-Identifier: Apache-2.0
// Copyright NuriScada contributors

import { describe, expect, it } from "vitest";

import { UndoStack } from "./undo-stack";

describe("UndoStack", () => {
  it("returns undefined on empty undo and redo", () => {
    const h = new UndoStack<number[]>();
    expect(h.undo([1])).toBeUndefined();
    expect(h.redo([1])).toBeUndefined();
    expect(h.undoDepth).toBe(0);
    expect(h.redoDepth).toBe(0);
  });

  it("undoes and redoes a single change", () => {
    const h = new UndoStack<number[]>();
    h.commit([1]);
    expect(h.undo([2])).toEqual([1]);
    expect(h.undoDepth).toBe(0);
    expect(h.redoDepth).toBe(1);
    expect(h.redo([2])).toEqual([2]);
    expect(h.undoDepth).toBe(1);
    expect(h.redoDepth).toBe(0);
  });

  it("clears redo on a new commit", () => {
    const h = new UndoStack<number[]>();
    h.commit([1]);
    h.undo([2]);
    h.commit([3]);
    expect(h.redoDepth).toBe(0);
    expect(h.redo([3])).toBeUndefined();
  });

  it("evicts the oldest entry past the cap", () => {
    const h = new UndoStack<number[]>(2);
    h.commit([1]);
    h.commit([2]);
    h.commit([3]);
    expect(h.undoDepth).toBe(2);
    // [1]이 밀려나고, 뒤에서부터 [3]·[2] 순으로 되돌아간다.
    expect(h.undo([9])).toEqual([3]);
    expect(h.undo([9])).toEqual([2]);
    expect(h.undo([9])).toBeUndefined();
  });

  it("stages an edit only when committed", () => {
    const h = new UndoStack<number[]>();
    h.commitStaged();
    expect(h.undoDepth).toBe(0);
    h.stage([1]);
    h.commitStaged();
    expect(h.undoDepth).toBe(1);
    expect(h.undo([2])).toEqual([1]);
  });

  it("clears depths on clear", () => {
    const h = new UndoStack<number[]>();
    h.commit([1]);
    h.undo([2]);
    h.clear();
    expect(h.undoDepth).toBe(0);
    expect(h.redoDepth).toBe(0);
    expect(h.undo([2])).toBeUndefined();
  });
});
