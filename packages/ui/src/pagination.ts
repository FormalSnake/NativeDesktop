// Pure page-range computation for Pagination: which numbered buttons show,
// and where the ellipsis gaps sit. Mirrors the standard sibling-count
// windowing (first, last, current +/- siblings, collapse the rest to dots).

export type PaginationItem = number | "dots-start" | "dots-end";

function range(start: number, end: number): number[] {
  if (end < start) return [];
  return Array.from({ length: end - start + 1 }, (_, i) => start + i);
}

export function computePaginationRange(page: number, pageCount: number, siblingCount = 1): PaginationItem[] {
  if (pageCount <= 0) return [];
  const clampedPage = Math.min(Math.max(page, 1), pageCount);
  const clampedSiblings = Math.max(siblingCount, 0);

  // first + last + current + two sibling runs + slack for the two dots.
  const totalVisible = clampedSiblings * 2 + 5;
  if (pageCount <= totalVisible) return range(1, pageCount);

  const leftSibling = Math.max(clampedPage - clampedSiblings, 1);
  const rightSibling = Math.min(clampedPage + clampedSiblings, pageCount);
  const showLeftDots = leftSibling > 2;
  const showRightDots = rightSibling < pageCount - 1;

  if (!showLeftDots && showRightDots) {
    const leftCount = 3 + clampedSiblings * 2;
    return [...range(1, leftCount), "dots-end", pageCount];
  }
  if (showLeftDots && !showRightDots) {
    const rightCount = 3 + clampedSiblings * 2;
    return [1, "dots-start", ...range(pageCount - rightCount + 1, pageCount)];
  }
  return [1, "dots-start", ...range(leftSibling, rightSibling), "dots-end", pageCount];
}
