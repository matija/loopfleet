// Surface cards: a responsive grid of entry-point tiles, each pairing a glyph
// with a title and a one-line description. Cards are buttons when actionable;
// a disabled card stays in the grid as an inert tile that says *why* it's
// unavailable, so a gated surface reads as "not yet" rather than vanishing.

import type { ReactNode } from "react";

export function SurfaceCardGrid({ children }: { children: ReactNode }) {
  return <div className="surface-grid">{children}</div>;
}

export function SurfaceCard({
  icon,
  title,
  description,
  onClick,
  disabled = false,
  disabledReason,
}: {
  icon: ReactNode;
  title: string;
  description: string;
  onClick?: () => void;
  disabled?: boolean;
  /** Shown in place of the action when the card is disabled. */
  disabledReason?: string;
}) {
  const body = (
    <>
      <span className="surface-card__icon" aria-hidden="true">
        {icon}
      </span>
      <span className="surface-card__title">{title}</span>
      <span className="surface-card__desc">{description}</span>
      {disabled && disabledReason && (
        <span className="surface-card__reason">{disabledReason}</span>
      )}
    </>
  );

  if (disabled || !onClick) {
    return (
      <div
        className={`surface-card${disabled ? " surface-card--disabled" : ""}`}
        aria-disabled={disabled || undefined}
      >
        {body}
      </div>
    );
  }

  return (
    <button type="button" className="surface-card" onClick={onClick}>
      {body}
    </button>
  );
}
