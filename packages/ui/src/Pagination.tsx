import { For, Show, createMemo } from "solid-js";
import type { JSX } from "@nativedesktop/react";
import { Spacing } from "@nativedesktop/react";
import { computePaginationRange } from "./pagination.ts";

export interface PaginationProps {
  page: number;
  pageCount: number;
  onPageChange: (page: number) => void;
  siblingCount?: number;
  testID?: string;
}

export function Pagination(props: PaginationProps): JSX.Element {
  const items = createMemo(() => computePaginationRange(props.page, props.pageCount, props.siblingCount ?? 1));
  const id = (suffix: string): string | undefined => (props.testID ? `${props.testID}-${suffix}` : undefined);

  return (
    <Show when={props.pageCount > 0}>
      <box orientation="horizontal" spacing={Spacing.xs} cssClasses={["linked"]} testID={props.testID}>
        <button
          label="First"
          onClick={() => props.onPageChange(1)}
          testID={id("first")}
          cssClasses={props.page === 1 ? ["flat"] : undefined}
        />
        <button
          label="Prev"
          onClick={() => props.onPageChange(Math.max(props.page - 1, 1))}
          testID={id("prev")}
          cssClasses={props.page === 1 ? ["flat"] : undefined}
        />
        <For each={items()}>
          {(item) =>
            item === "dots-start" || item === "dots-end" ? (
              <label text="…" style={{ valign: "center" }} />
            ) : (
              <button
                label={String(item)}
                prominent={item === props.page}
                onClick={() => props.onPageChange(item)}
                testID={id(`page-${item}`)}
              />
            )
          }
        </For>
        <button
          label="Next"
          onClick={() => props.onPageChange(Math.min(props.page + 1, props.pageCount))}
          testID={id("next")}
          cssClasses={props.page === props.pageCount ? ["flat"] : undefined}
        />
        <button
          label="Last"
          onClick={() => props.onPageChange(props.pageCount)}
          testID={id("last")}
          cssClasses={props.page === props.pageCount ? ["flat"] : undefined}
        />
      </box>
    </Show>
  );
}
