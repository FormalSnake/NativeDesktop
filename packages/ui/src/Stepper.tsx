/** @jsxImportSource @nativedesktop/react */

import type { ReactNode } from "react";
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

export function Stepper(props: StepperProps): ReactNode {
  const { steps, activeIndex, onStepClick, testID } = props;
  const nodes: ReactNode[] = [];

  steps.forEach((step, i) => {
    const state = stepState(i, activeIndex);
    if (i > 0) {
      nodes.push(
        <separator
          key={`sep-${step.id}`}
          orientation="horizontal"
          style={{ hexpand: true, valign: "center" }}
          cssClasses={state === "pending" ? ["dimmed"] : ["accent"]}
        />,
      );
    }
    nodes.push(
      <box
        key={step.id}
        orientation="vertical"
        spacing={Spacing.xs}
        testID={testID ? `${testID}-step-${step.id}` : undefined}
      >
        <button
          label={state === "completed" ? undefined : String(i + 1)}
          iconName={state === "completed" ? "emblem-ok" : undefined}
          cssClasses={["circular", state === "active" ? "suggested-action" : state === "completed" ? "success" : "flat"]}
          onClick={onStepClick ? () => onStepClick(i) : undefined}
        />
        <label text={step.title} cssClasses={state === "pending" ? ["dimmed", "caption"] : ["caption-heading"]} />
        {step.description && <label text={step.description} cssClasses={["dimmed", "caption"]} />}
      </box>,
    );
  });

  return (
    <box orientation="horizontal" spacing={Spacing.sm} testID={testID}>
      {nodes}
    </box>
  );
}
