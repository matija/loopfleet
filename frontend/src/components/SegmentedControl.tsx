// Themed control for 2-4 mutually exclusive options laid out as adjoining
// segments — the visual alternative to a small `Select` when every option
// should be visible at once (e.g. view-mode toggles). Follows the ARIA
// "radiogroup" pattern: the group carries `role="radiogroup"`, each segment
// is a `role="radio"` button, and only the checked segment (or the first
// segment, before any selection) is in the tab order — arrow keys move
// both focus and the roving tabindex across the rest, exactly like a
// native radio button group.

import { useRef, type KeyboardEvent as ReactKeyboardEvent, type ReactNode } from "react";

export type SegmentedControlOption<T extends string = string> = {
  value: T;
  label: ReactNode;
  disabled?: boolean;
};

export type SegmentedControlProps<T extends string = string> = {
  value: T;
  options: SegmentedControlOption<T>[];
  onChange: (value: T) => void;
  disabled?: boolean;
  className?: string;
  "aria-label"?: string;
};

export function SegmentedControl<T extends string = string>({
  value,
  options,
  onChange,
  disabled,
  className,
  ...rest
}: SegmentedControlProps<T>) {
  if (options.length < 2 || options.length > 4) {
    throw new Error("SegmentedControl supports 2-4 options, got " + options.length);
  }

  const rootRef = useRef<HTMLDivElement>(null);

  const enabledIndices = options
    .map((o, i) => (o.disabled ? -1 : i))
    .filter((i) => i >= 0);

  function focusSegment(index: number) {
    const root = rootRef.current;
    if (!root) return;
    const el = root.querySelectorAll<HTMLButtonElement>('[role="radio"]')[index];
    el?.focus();
  }

  function moveFocus(from: number, delta: number) {
    if (enabledIndices.length === 0) return;
    const pos = enabledIndices.indexOf(from);
    const basePos = pos === -1 ? 0 : pos;
    const len = enabledIndices.length;
    const nextPos = ((basePos + delta) % len + len) % len;
    focusSegment(enabledIndices[nextPos]);
  }

  function onKeyDown(e: ReactKeyboardEvent<HTMLButtonElement>, index: number) {
    if (disabled) return;
    switch (e.key) {
      case "ArrowRight":
      case "ArrowDown":
        e.preventDefault();
        moveFocus(index, 1);
        return;
      case "ArrowLeft":
      case "ArrowUp":
        e.preventDefault();
        moveFocus(index, -1);
        return;
      case "Home":
        e.preventDefault();
        if (enabledIndices.length) focusSegment(enabledIndices[0]);
        return;
      case "End":
        e.preventDefault();
        if (enabledIndices.length) focusSegment(enabledIndices[enabledIndices.length - 1]);
        return;
    }
  }

  const selectedIndex = options.findIndex((o) => o.value === value);
  const cls = className ? `segmented-control ${className}` : "segmented-control";

  return (
    <div ref={rootRef} className={cls} role="radiogroup" aria-label={rest["aria-label"]}>
      {options.map((opt, i) => {
        const checked = opt.value === value;
        // Roving tabindex: the checked segment is the tab stop, or the first
        // enabled segment when nothing is selected yet.
        const isTabStop = selectedIndex === -1 ? i === enabledIndices[0] : checked;
        return (
          <button
            key={opt.value}
            type="button"
            role="radio"
            aria-checked={checked}
            disabled={disabled || opt.disabled}
            tabIndex={isTabStop ? 0 : -1}
            className={`segmented-control__segment${checked ? " segmented-control__segment--checked" : ""}`}
            onClick={() => !opt.disabled && onChange(opt.value)}
            onKeyDown={(e) => onKeyDown(e, i)}
          >
            {opt.label}
          </button>
        );
      })}
    </div>
  );
}
