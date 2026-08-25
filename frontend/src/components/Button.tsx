// Two button families live here.
//
// `Button` is the panel-level action button (styles in panels.css: .btn,
// .btn--primary, .btn--secondary, .btn--quiet) — an explicit three-level
// hierarchy from filled-accent call to action, down through the hairline
// default, to the text-weight affordance that only picks up a border on
// hover for actions that ride beside a surface's real work.
//
// `IconButton` and `SplitButton` are toolbar-class primitives (styles in
// toolbar.css: .toolbar-btn, .toolbar-btn--icon, .toolbar-split) — a bare
// icon-only button for the trailing slot, and a split button whose chevron
// opens a separate menu from the main action.

import type { ButtonHTMLAttributes, JSX, ReactNode, RefObject } from "react";
import { ChevronDownIcon } from "./Icon";

type IconComponent = (props: { size?: number; className?: string }) => JSX.Element;

export type ButtonVariant = "primary" | "secondary" | "quiet";

type ButtonProps = ButtonHTMLAttributes<HTMLButtonElement> & {
  /// Leading 16px glyph, drawn before the label.
  icon?: IconComponent;
  /// Hierarchy level. Defaults to the hairline "secondary" look.
  variant?: ButtonVariant;
};

export function Button({ icon: Icon, variant = "secondary", className, children, ...rest }: ButtonProps) {
  const cls = className
    ? `btn btn--${variant} ${className}`
    : `btn btn--${variant}`;
  return (
    <button type="button" className={cls} {...rest}>
      {Icon && <Icon size={16} className="btn__icon" />}
      {children}
    </button>
  );
}

type IconButtonProps = ButtonHTMLAttributes<HTMLButtonElement> & {
  icon: IconComponent;
  /// Required: an icon-only button carries no visible label.
  "aria-label": string;
};

export function IconButton({ icon: Icon, className, ...rest }: IconButtonProps) {
  const cls = className
    ? `toolbar-btn toolbar-btn--icon ${className}`
    : "toolbar-btn toolbar-btn--icon";
  return (
    <button type="button" className={cls} {...rest}>
      <Icon size={16} className="icon" />
    </button>
  );
}

type SplitButtonProps = {
  /// Leading 16px glyph on the main segment.
  icon?: IconComponent;
  children?: ReactNode;
  onClick?: ButtonHTMLAttributes<HTMLButtonElement>["onClick"];
  onChevronClick?: ButtonHTMLAttributes<HTMLButtonElement>["onClick"];
  /// Label for the chevron segment, since it carries no visible text.
  chevronLabel?: string;
  disabled?: boolean;
  className?: string;
  /// Forwarded to the chevron segment so callers can anchor a Popover to it.
  chevronRef?: RefObject<HTMLButtonElement | null>;
};

export function SplitButton({
  icon: Icon,
  children,
  onClick,
  onChevronClick,
  chevronLabel = "More options",
  disabled,
  className,
  chevronRef,
}: SplitButtonProps) {
  const cls = className ? `toolbar-split ${className}` : "toolbar-split";
  return (
    <div className={cls}>
      <button
        type="button"
        className="toolbar-btn toolbar-split__main"
        onClick={onClick}
        disabled={disabled}
      >
        {Icon && <Icon size={16} className="icon" />}
        {children}
      </button>
      <button
        ref={chevronRef}
        type="button"
        className="toolbar-btn toolbar-btn--icon toolbar-split__chevron"
        onClick={onChevronClick}
        disabled={disabled}
        aria-label={chevronLabel}
      >
        <ChevronDownIcon size={16} className="icon" />
      </button>
    </div>
  );
}
