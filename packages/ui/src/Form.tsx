import type { JSX } from "@nativedesktop/solid";

export interface FormProps {
  title?: string;
  description?: string;
  children: JSX.Element;
  testID?: string;
}

export function Form(props: FormProps): JSX.Element {
  return (
    <settingsgroup title={props.title} description={props.description} testID={props.testID}>
      {props.children}
    </settingsgroup>
  );
}

export interface FormFieldProps {
  label: string;
  error?: string;
  hint?: string;
  children: JSX.Element;
  testID?: string;
}

/** The control renders into the row's default (suffix) slot, matching the
 * label-left/control-right shape every other settings row already uses. */
export function FormField(props: FormFieldProps): JSX.Element {
  return (
    <row
      title={props.label}
      subtitle={props.error ?? props.hint}
      cssClasses={props.error ? ["error"] : undefined}
      testID={props.testID}
    >
      {props.children}
    </row>
  );
}
