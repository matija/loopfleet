// Themed replacement for a native <select>: a button trigger plus a
// listbox popup positioned with the app's one floating-surface primitive
// (Popover.tsx), so it sits in the same z-index/portal/dismiss world as
// every other menu rather than inheriting the browser's unthemeable native
// dropdown chrome.
//
// Follows the ARIA "listbox popup" pattern (a native <select>'s actual
// keyboard contract, not a combobox's): the trigger itself owns focus and
// all key handling, `aria-activedescendant` tracks the highlighted option,
// and the listbox is presentational rather than focusable. That keeps
// arrow/type-ahead behavior working identically whether the popup is open
// or closed, exactly like the control it replaces.

import {
  useId,
  useRef,
  useState,
  type JSX,
  type KeyboardEvent as ReactKeyboardEvent,
} from "react";
import { Popover } from "./Popover";
import { ChevronDownIcon } from "./Icon";
import { clampToEnabled, enabledIndices, moveIndex } from "../optionIndex";
import { emptyTypeahead, extendTypeahead, matchTypeahead } from "../typeahead";

type IconComponent = (props: { size?: number; className?: string }) => JSX.Element;

export type SelectOption<T extends string = string> = {
  value: T;
  label: string;
  /// Leading 16px glyph drawn before the option's label.
  icon?: IconComponent;
  disabled?: boolean;
};

export type SelectProps<T extends string = string> = {
  value: T;
  options: SelectOption<T>[];
  onChange: (value: T) => void;
  disabled?: boolean;
  placeholder?: string;
  className?: string;
  "aria-label"?: string;
};

export function Select<T extends string = string>({
  value,
  options,
  onChange,
  disabled,
  placeholder = "Select…",
  className,
  ...rest
}: SelectProps<T>) {
  const [open, setOpen] = useState(false);
  const [activeIndex, setActiveIndex] = useState(() =>
    Math.max(
      0,
      options.findIndex((o) => o.value === value),
    ),
  );
  const buttonRef = useRef<HTMLButtonElement>(null);
  const typeaheadRef = useRef(emptyTypeahead);
  const listboxId = useId();
  const optionIdPrefix = useId();

  const selected = options.find((o) => o.value === value);
  const enabled = enabledIndices(options);

  function openAt(index: number) {
    setActiveIndex(clampToEnabled(enabled, index));
    setOpen(true);
  }

  function commit(index: number) {
    const opt = options[index];
    if (!opt || opt.disabled) return;
    onChange(opt.value);
    setOpen(false);
  }

  function moveActive(delta: number) {
    if (enabled.length === 0) return;
    setActiveIndex((current) => moveIndex(enabled, current, delta, "clamp"));
  }

  function typeahead(char: string) {
    const state = extendTypeahead(typeaheadRef.current, char, Date.now());
    typeaheadRef.current = state;

    const match = matchTypeahead(
      options.map((o) => o.label),
      enabled,
      activeIndex,
      state.text,
    );
    if (match !== undefined) {
      if (open) setActiveIndex(match);
      else commit(match);
    }
  }

  function onKeyDown(e: ReactKeyboardEvent<HTMLButtonElement>) {
    if (disabled) return;
    switch (e.key) {
      case "ArrowDown":
        e.preventDefault();
        if (!open) openAt(activeIndex);
        else moveActive(1);
        return;
      case "ArrowUp":
        e.preventDefault();
        if (!open) openAt(activeIndex);
        else moveActive(-1);
        return;
      case "Home":
        e.preventDefault();
        if (enabled.length) setActiveIndex(enabled[0]);
        if (!open) commit(enabled[0]);
        return;
      case "End":
        e.preventDefault();
        if (enabled.length) {
          const last = enabled[enabled.length - 1];
          setActiveIndex(last);
          if (!open) commit(last);
        }
        return;
      case "Enter":
      case " ":
        e.preventDefault();
        if (open) commit(activeIndex);
        else openAt(activeIndex);
        return;
      case "Escape":
        // Popover's own document-level listener also handles Escape (and
        // refocuses the trigger); nothing else to do here.
        return;
      case "Tab":
        setOpen(false);
        return;
      default:
        if (e.key.length === 1 && !e.metaKey && !e.ctrlKey && !e.altKey) {
          e.preventDefault();
          typeahead(e.key);
        }
    }
  }

  const cls = className ? `select-trigger ${className}` : "select-trigger";
  const SelectedIcon = selected?.icon;

  return (
    <>
      <button
        ref={buttonRef}
        type="button"
        className={cls}
        disabled={disabled}
        role="combobox"
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-controls={listboxId}
        aria-activedescendant={open ? `${optionIdPrefix}-${activeIndex}` : undefined}
        onClick={() => (open ? setOpen(false) : openAt(activeIndex))}
        onKeyDown={onKeyDown}
        {...rest}
      >
        {SelectedIcon && <SelectedIcon size={16} className="icon" />}
        <span className="select-trigger__label">{selected?.label ?? placeholder}</span>
        <ChevronDownIcon size={16} className="icon select-trigger__chevron" />
      </button>
      <Popover
        open={open}
        onClose={() => setOpen(false)}
        anchorRef={buttonRef}
        role="dialog"
        className="select-popover"
      >
        <ul id={listboxId} role="listbox" className="select-listbox" aria-label={rest["aria-label"]}>
          {options.map((opt, i) => {
            const active = i === activeIndex;
            const isSelected = opt.value === value;
            const OptIcon = opt.icon;
            return (
              <li
                key={opt.value}
                id={`${optionIdPrefix}-${i}`}
                role="option"
                aria-selected={isSelected}
                aria-disabled={opt.disabled || undefined}
                className={`select-option${active ? " select-option--active" : ""}${
                  opt.disabled ? " select-option--disabled" : ""
                }`}
                onMouseEnter={() => !opt.disabled && setActiveIndex(i)}
                onMouseDown={(e) => {
                  // Prevent the trigger button from losing focus before the
                  // click fires, so keyboard state (active index) survives a
                  // mouse pick.
                  e.preventDefault();
                }}
                onClick={() => !opt.disabled && commit(i)}
              >
                {OptIcon && <OptIcon size={16} className="icon select-option__icon" />}
                <span className="select-option__label">{opt.label}</span>
              </li>
            );
          })}
        </ul>
      </Popover>
    </>
  );
}
