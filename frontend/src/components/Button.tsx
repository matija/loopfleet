// Toolbar-class button primitives (styles in toolbar.css: .toolbar-btn,
// .toolbar-btn--icon, .toolbar-split). Three shapes sharing one hairline
// vocabulary: a labeled pill, a bare icon-only button for the trailing
// slot, and a split button whose chevron opens a separate menu from the
// main action.

import type { ButtonHTMLAttributes, ReactNode } from "react";
import { ChevronDownIcon } from "./Icon";

type IconComponent = (props: { size?: number; className?: string }) => JSX.Element;

type ButtonProps = ButtonHTMLAttributes<HTMLButtonElement> & {
  /// Leading 16px glyph, drawn before the label.
  icon?: IconComponent;
};

export function Button({ icon: Icon, className, children, ...rest }: ButtonProps) {
  const cls = className ? `toolbar-btn ${className}` : "toolbar-btn";
  return (
    <button type="button" className={cls} {...rest}>
      {Icon && <Icon size={16} className="icon" />}
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
};

export function SplitButton({
  icon: Icon,
  children,
  onClick,
  onChevronClick,
  chevronLabel = "More options",
  disabled,
  className,
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
