// Themed replacement for a native <input type="number">: same underlying
// input (so browser numeric validation/IME/paste behavior is unchanged) but
// with the native up/down spinner hidden (number-field.css) and redrawn as a
// stacked +/- affordance in app tokens, matching the boxed-input look shared
// by Select.tsx and .field input (panels.css).

import {
  useEffect,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
} from "react";
import { PlusIcon, MinusIcon } from "./Icon";

export type NumberFieldProps = {
  value: number;
  onChange: (value: number) => void;
  min?: number;
  max?: number;
  step?: number;
  /// Trailing unit label drawn inside the field, before the stepper (e.g. "px", "%").
  unit?: string;
  disabled?: boolean;
  className?: string;
  id?: string;
  "aria-label"?: string;
};

function clamp(n: number, min?: number, max?: number): number {
  let v = n;
  if (min !== undefined) v = Math.max(min, v);
  if (max !== undefined) v = Math.min(max, v);
  return v;
}

// Rounds to the step's own decimal precision so repeated +/- presses don't
// accumulate floating-point noise (the classic 0.1 + 0.2 drift).
function roundToStep(n: number, step: number): number {
  const decimals = (step.toString().split(".")[1] ?? "").length;
  return Number(n.toFixed(decimals));
}

export function NumberField({
  value,
  onChange,
  min,
  max,
  step = 1,
  unit,
  disabled,
  className,
  id,
  ...rest
}: NumberFieldProps) {
  const [text, setText] = useState(() => String(value));
  const inputRef = useRef<HTMLInputElement>(null);

  // Sync from external changes (e.g. a reset button elsewhere), but never
  // stomp on what the user is mid-typing.
  useEffect(() => {
    setText(String(value));
  }, [value]);

  function commit(next: number) {
    const clamped = clamp(roundToStep(next, step), min, max);
    onChange(clamped);
    setText(String(clamped));
  }

  function nudge(delta: number) {
    if (disabled) return;
    const parsed = Number(text);
    const base = text.trim() !== "" && Number.isFinite(parsed) ? parsed : value;
    commit(base + delta);
  }

  function onKeyDown(e: ReactKeyboardEvent<HTMLInputElement>) {
    if (disabled) return;
    switch (e.key) {
      case "ArrowUp":
        e.preventDefault();
        nudge(step);
        return;
      case "ArrowDown":
        e.preventDefault();
        nudge(-step);
        return;
      case "Enter":
        inputRef.current?.blur();
        return;
    }
  }

  function onBlur() {
    const parsed = Number(text);
    if (text.trim() === "" || !Number.isFinite(parsed)) {
      setText(String(value));
      return;
    }
    commit(parsed);
  }

  const cls = className ? `number-field ${className}` : "number-field";
  const atMin = min !== undefined && value <= min;
  const atMax = max !== undefined && value >= max;

  return (
    <div className={disabled ? `${cls} number-field--disabled` : cls}>
      <input
        ref={inputRef}
        id={id}
        type="number"
        className="number-field__input"
        value={text}
        min={min}
        max={max}
        step={step}
        disabled={disabled}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={onKeyDown}
        onBlur={onBlur}
        {...rest}
      />
      {unit && <span className="number-field__unit">{unit}</span>}
      <div className="number-field__stepper">
        <button
          type="button"
          className="number-field__step number-field__step--up"
          tabIndex={-1}
          aria-label="Increment"
          disabled={disabled || atMax}
          onClick={() => nudge(step)}
        >
          <PlusIcon size={11} className="icon" />
        </button>
        <button
          type="button"
          className="number-field__step number-field__step--down"
          tabIndex={-1}
          aria-label="Decrement"
          disabled={disabled || atMin}
          onClick={() => nudge(-step)}
        >
          <MinusIcon size={11} className="icon" />
        </button>
      </div>
    </div>
  );
}
