// SPDX-License-Identifier: Apache-2.0
// Copyright NuriScada contributors

/** 실행 취소·다시 실행 스택. 스냅샷은 불투명 값으로 다루며, 호출 측이 찍어 준다. */
export class UndoStack<T> {
  private past: T[] = [];
  private future: T[] = [];
  private staged: T | null = null;

  constructor(private readonly max = 50) {}

  get undoDepth(): number {
    return this.past.length;
  }

  get redoDepth(): number {
    return this.future.length;
  }

  clear(): void {
    this.past = [];
    this.future = [];
    this.staged = null;
  }

  /** 변경 직전 상태를 쌓는다. 같은 제스처에서는 한 번만 호출하도록 호출 측에서 조절한다. */
  commit(snapshot: T): void {
    this.past.push(snapshot);
    if (this.past.length > this.max) this.past.shift();
    this.future = [];
    this.staged = null;
  }

  /** 입력 칸에 들어갈 때 변경 전 상태를 잡아 둔다. */
  stage(snapshot: T): void {
    this.staged = snapshot;
  }

  /** 입력 칸의 값이 확정되면 잡아 둔 상태를 쌓는다. */
  commitStaged(): void {
    if (this.staged === null) return;
    this.commit(this.staged);
  }

  /** 되돌릴 이전 상태를 내놓는다. 현재 상태는 다시 실행 쪽에 쌓는다. */
  undo(current: T): T | undefined {
    const prev = this.past.pop();
    if (prev === undefined) return undefined;
    this.future.push(current);
    this.staged = null;
    return prev;
  }

  /** 다시 실행할 다음 상태를 내놓는다. 현재 상태는 실행 취소 쪽에 쌓는다. */
  redo(current: T): T | undefined {
    const next = this.future.pop();
    if (next === undefined) return undefined;
    this.past.push(current);
    if (this.past.length > this.max) this.past.shift();
    this.staged = null;
    return next;
  }
}
