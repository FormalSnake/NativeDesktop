import { For, Show } from "solid-js";
import type { JSX } from "@nativedesktop/react";
import { Spacing } from "@nativedesktop/react";
import { stepState } from "./stepper.ts";

export interface StepperStep {
  id: string;
  title: string;
  description?: string;
}

export interface StepperProps {
  steps: StepperStep[];
  activeIndex: number;
  onStepClick?: (index: number) => void;
  testID?: string;
}

export function Stepper(props: StepperProps): JSX.Element {
  return (
    <box orientation="horizontal" spacing={Spacing.sm} testID={props.testID}>
      <For each={props.steps} keyed={(step) => step.id}>
        {(step, i) => {
          const state = () => stepState(i(), props.activeIndex);
          return (
            <>
              <Show when={i() > 0}>
                <separator
                  orientation="horizontal"
                  style={{ hexpand: true, valign: "center" }}
                  cssClasses={state() === "pending" ? ["dimmed"] : ["accent"]}
                />
              </Show>
              <box
                orientation="vertical"
                spacing={Spacing.xs}
                testID={props.testID ? `${props.testID}-step-${step().id}` : undefined}
              >
                <button
                  label={state() === "completed" ? undefined : String(i() + 1)}
                  iconName={state() === "completed" ? "emblem-ok" : undefined}
                  cssClasses={[
                    "circular",
                    state() === "active" ? "suggested-action" : state() === "completed" ? "success" : "flat",
                  ]}
                  onClick={props.onStepClick ? () => props.onStepClick?.(i()) : undefined}
                />
                <label
                  text={step().title}
                  cssClasses={state() === "pending" ? ["dimmed", "caption"] : ["caption-heading"]}
                />
                <Show when={step().description}>
                  {(description) => <label text={description()} cssClasses={["dimmed", "caption"]} />}
                </Show>
              </box>
            </>
          );
        }}
      </For>
    </box>
  );
}
