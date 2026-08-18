export type StepState = "completed" | "active" | "pending";

export function stepState(index: number, activeIndex: number): StepState {
  if (index < activeIndex) return "completed";
  if (index === activeIndex) return "active";
  return "pending";
}
