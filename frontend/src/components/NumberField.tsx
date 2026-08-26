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
import {
  atMax,
  atMin,
  normalizeValue,
  parseFieldValue,
  stepValue,
} from "../numberSteps";

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

  const bounds = { min, max, step };

  function commit(next: number) {
    onChange(next);
    setText(String(next));
  }

  function nudge(delta: number) {
    if (disabled) return;
    // A half-typed field steps from the last committed value, not from 0.
    const base = parseFieldValue(text) ?? value;
    commit(stepValue(base, delta, bounds));
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
    const parsed = parseFieldValue(text);
    if (parsed === null) {
      setText(String(value));
      return;
    }
    commit(normalizeValue(parsed, bounds));
  }

  const cls = className ? `number-field ${className}` : "number-field";
  const lowered = atMin(value, min);
  const raised = atMax(value, max);

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
          disabled={disabled || raised}
          onClick={() => nudge(step)}
        >
          <PlusIcon size={11} className="icon" />
        </button>
        <button
          type="button"
          className="number-field__step number-field__step--down"
          tabIndex={-1}
          aria-label="Decrement"
          disabled={disabled || lowered}
          onClick={() => nudge(-step)}
        >
          <MinusIcon size={11} className="icon" />
        </button>
      </div>
    </div>
  );
}
