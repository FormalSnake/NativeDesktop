/** @jsxImportSource @nativedesktop/react */

import type { ReactNode } from "react";

export interface FormProps {
  title?: string;
  description?: string;
  children: ReactNode;
  testID?: string;
}

export function Form(props: FormProps): ReactNode {
  const { title, description, children, testID } = props;
  return (
    <settingsgroup title={title} description={description} testID={testID}>
      {children}
    </settingsgroup>
  );
}

export interface FormFieldProps {
  label: string;
  error?: string;
  hint?: string;
  children: ReactNode;
  testID?: string;
}

/** The control renders into the row's default (suffix) slot, matching the
 * label-left/control-right shape every other settings row already uses. */
export function FormField(props: FormFieldProps): ReactNode {
  const { label, error, hint, children, testID } = props;
  return (
    <row title={label} subtitle={error ?? hint} cssClasses={error ? ["error"] : undefined} testID={testID}>
      {children}
    </row>
  );
}
