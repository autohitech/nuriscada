// SPDX-License-Identifier: Apache-2.0
// Copyright NuriScada contributors

/** 고정 행 높이 목록의 보이는 범위. 가상 스크롤이 이 범위의 행만 그린다. */
export function visibleRange(
  scrollTop: number,
  viewportHeight: number,
  rowHeight: number,
  total: number,
  overscan = 8,
): { start: number; end: number } {
  if (total <= 0 || rowHeight <= 0) return { start: 0, end: 0 };
  const first = Math.floor(Math.max(0, scrollTop) / rowHeight);
  const last = Math.ceil((Math.max(0, scrollTop) + Math.max(0, viewportHeight)) / rowHeight);
  const end = Math.min(total, last + overscan);
  // 스크롤 위치가 목록 끝을 넘어서면(검색으로 목록이 줄어든 직후) 빈 범위를 돌려준다.
  return { start: Math.min(end, Math.max(0, first - overscan)), end };
}
