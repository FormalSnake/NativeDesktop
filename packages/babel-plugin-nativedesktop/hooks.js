// The React hook subset that `@nativedesktop/react` pins across `bun --hot`
// re-evals (packages/react/src/dev-react.ts). A hook imported from raw "react"
// resolves to a re-evaluated react instance whose dispatcher no reconciler
// ever attached to -> "Invalid hook call". These names, and only these, get
// their `from "react"` import redirected to `@nativedesktop/react`.
module.exports.PINNED_HOOKS = [
  "useState",
  "useEffect",
  "useLayoutEffect",
  "useMemo",
  "useCallback",
  "useRef",
  "useContext",
  "useReducer",
  "useTransition",
  "useDeferredValue",
  "useSyncExternalStore",
  "useId",
  "use",
  "startTransition",
];
