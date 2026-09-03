// The breadcrumb's project crumb, rendered as a menu button: clicking it
// lists every registered project and switches to the one picked. This is the
// only way to reach another project while the sidebar is hidden (⌘B), so the
// crumb — the one place the current project's name is always on screen — is
// what carries it; with the sidebar open it's a shortcut for the same jump.
//
// Picking the project that's already open is allowed rather than inert: it
// lands on that project's plan, which is what the plain (menu-less) crumb
// does when clicked, so the behavior the crumb had before the menu is still
// one click away inside it.

import { useRef, useState, type ComponentType } from "react";
import { Popover } from "./Popover";
import { ChevronDownIcon, type IconProps } from "./Icon";

export type CrumbMenuOption = {
  id: string;
  label: string;
  /// Dimmed trailing detail — the project's parent path, which is what tells
  /// two same-named repos apart (the sidebar row shows it for the same reason).
  hint?: string;
};

export function CrumbMenu({
  label,
  icon: Icon,
  options,
  currentId,
  onSelect,
  "aria-label": ariaLabel,
}: {
  label: string;
  icon?: ComponentType<IconProps>;
  options: CrumbMenuOption[];
  currentId?: string;
  onSelect: (id: string) => void;
  "aria-label"?: string;
}) {
  const [open, setOpen] = useState(false);
  const buttonRef = useRef<HTMLButtonElement>(null);
  const listRef = useRef<HTMLDivElement>(null);

  // Which row takes focus when the menu opens: the current project, or the
  // first row when the crumb names a project that isn't in the list (a run's
  // remembered project, say, whose entry has since been removed). Something
  // must take it, or the arrow keys below would have nothing to move.
  const focusId = options.some((o) => o.id === currentId)
    ? currentId
    : options[0]?.id;

  // Arrow keys walk the rendered option buttons themselves (roving focus)
  // rather than a tracked index: the list is short, flat, and never disabled,
  // so the DOM order is the whole model. Esc and outside clicks are Popover's.
  function moveFocus(delta: number) {
    const items = listRef.current?.querySelectorAll<HTMLButtonElement>(
      ".crumb-menu__option",
    );
    if (!items || items.length === 0) return;
    const current = document.activeElement;
    const at = Array.from(items).indexOf(current as HTMLButtonElement);
    const next = at < 0 ? 0 : (at + delta + items.length) % items.length;
    items[next].focus();
  }

  return (
    <>
      <button
        type="button"
        ref={buttonRef}
        className="toolbar__crumb toolbar__crumb--menu"
        aria-haspopup="menu"
        aria-expanded={open}
        aria-label={ariaLabel}
        onClick={() => setOpen((v) => !v)}
        onKeyDown={(e) => {
          if (e.key === "ArrowDown" || e.key === "ArrowUp") {
            e.preventDefault();
            setOpen(true);
          }
        }}
      >
        {Icon && <Icon size={13} className="toolbar__crumb-icon" />}
        {label}
        <ChevronDownIcon size={12} className="toolbar__crumb-chevron" />
      </button>
      <Popover
        open={open}
        onClose={() => setOpen(false)}
        anchorRef={buttonRef}
        role="menu"
        placement="bottom-start"
        className="crumb-menu"
        aria-label="Switch project"
      >
        <div
          className="crumb-menu__list"
          ref={listRef}
          onKeyDown={(e) => {
            if (e.key === "ArrowDown") {
              e.preventDefault();
              moveFocus(1);
            } else if (e.key === "ArrowUp") {
              e.preventDefault();
              moveFocus(-1);
            }
          }}
        >
          {options.map((o) => (
            <button
              key={o.id}
              type="button"
              role="menuitem"
              className="crumb-menu__option"
              aria-current={o.id === currentId}
              autoFocus={o.id === focusId}
              onClick={() => {
                setOpen(false);
                onSelect(o.id);
              }}
            >
              <span className="crumb-menu__label">{o.label}</span>
              {o.hint && <span className="crumb-menu__hint">{o.hint}</span>}
            </button>
          ))}
        </div>
      </Popover>
    </>
  );
}
