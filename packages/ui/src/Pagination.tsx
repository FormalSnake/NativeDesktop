/** @jsxImportSource @nativedesktop/react */

import type { ReactNode } from "react";
import { Spacing } from "@nativedesktop/react";
import { computePaginationRange } from "./pagination.ts";

export interface PaginationProps {
  page: number;
  pageCount: number;
  onPageChange: (page: number) => void;
  siblingCount?: number;
  testID?: string;
}

export function Pagination(props: PaginationProps): ReactNode {
  const { page, pageCount, onPageChange, siblingCount = 1, testID } = props;
  if (pageCount <= 0) return null;
  const items = computePaginationRange(page, pageCount, siblingCount);

  return (
    <box orientation="horizontal" spacing={Spacing.xs} cssClasses={["linked"]} testID={testID}>
      <button
        label="First"
        onClick={() => onPageChange(1)}
        testID={testID ? `${testID}-first` : undefined}
        cssClasses={page === 1 ? ["flat"] : undefined}
      />
      <button
        label="Prev"
        onClick={() => onPageChange(Math.max(page - 1, 1))}
        testID={testID ? `${testID}-prev` : undefined}
        cssClasses={page === 1 ? ["flat"] : undefined}
      />
      {items.map((item, i) =>
        item === "dots-start" || item === "dots-end" ? (
          <label key={item} text="…" style={{ valign: "center" }} />
        ) : (
          <button
            key={item}
            label={String(item)}
            prominent={item === page}
            onClick={() => onPageChange(item)}
            testID={testID ? `${testID}-page-${item}` : undefined}
          />
        ),
      )}
      <button
        label="Next"
        onClick={() => onPageChange(Math.min(page + 1, pageCount))}
        testID={testID ? `${testID}-next` : undefined}
        cssClasses={page === pageCount ? ["flat"] : undefined}
      />
      <button
        label="Last"
        onClick={() => onPageChange(pageCount)}
        testID={testID ? `${testID}-last` : undefined}
        cssClasses={page === pageCount ? ["flat"] : undefined}
      />
    </box>
  );
}
