// The app's single floating-surface primitive — every anchored menu or
// popup dialog (agent pickers, overflow menus, inline confirms) should
// render through this component rather than hand-rolling position math and
// dismiss handling per call site. Positions itself against an anchor
// element, flips to whichever side of the anchor has room, and portals into
// document.body so it is never clipped by an ancestor's overflow.
//
// Dismiss contract: Esc closes, a mousedown outside the panel (and outside
// the anchor) closes, and focus returns to the anchor element once the
// popover unmounts — the anchor is usually the button that opened it, so
// keyboard users land back where they started rather than at the top of
// the page.

import {
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type ReactNode,
  type RefObject,
} from "react";
import { createPortal } from "react-dom";

const MARGIN = 6;

export type PopoverPlacement = "bottom-start" | "bottom-end" | "top-start" | "top-end";

export type PopoverProps = {
  open: boolean;
  onClose: () => void;
  /// Element the popover is positioned against and returns focus to on close.
  anchorRef: RefObject<HTMLElement | null>;
  children: ReactNode;
  /// "menu" for an action list (SplitButton chevrons, overflow menus);
  /// "dialog" for anything with its own form controls or free-form content.
  role?: "menu" | "dialog";
  /// Preferred side/alignment; flips to the side with more room if the
  /// preferred one doesn't fit the panel.
  placement?: PopoverPlacement;
  "aria-label"?: string;
  className?: string;
};

type Resolved = {
  top: number;
  left: number;
  vertical: "top" | "bottom";
};

function resolvePosition(
  anchorRect: DOMRect,
  panelRect: DOMRect,
  placement: PopoverPlacement,
): Resolved {
  const vw = window.innerWidth;
  const vh = window.innerHeight;

  let vertical: "top" | "bottom" = placement.startsWith("top") ? "top" : "bottom";
  const spaceBelow = vh - anchorRect.bottom;
  const spaceAbove = anchorRect.top;
  if (vertical === "bottom" && spaceBelow < panelRect.height + MARGIN && spaceAbove > spaceBelow) {
    vertical = "top";
  } else if (vertical === "top" && spaceAbove < panelRect.height + MARGIN && spaceBelow > spaceAbove) {
    vertical = "bottom";
  }

  let align: "start" | "end" = placement.endsWith("end") ? "end" : "start";
  let left = align === "start" ? anchorRect.left : anchorRect.right - panelRect.width;
  if (align === "start" && left + panelRect.width > vw - MARGIN) {
    align = "end";
    left = anchorRect.right - panelRect.width;
  } else if (align === "end" && left < MARGIN) {
    align = "start";
    left = anchorRect.left;
  }
  left = Math.min(Math.max(left, MARGIN), Math.max(MARGIN, vw - panelRect.width - MARGIN));

  const top =
    vertical === "bottom" ? anchorRect.bottom + MARGIN : anchorRect.top - panelRect.height - MARGIN;
  const clampedTop = Math.min(Math.max(top, MARGIN), Math.max(MARGIN, vh - panelRect.height - MARGIN));

  return { top: clampedTop, left, vertical };
}

export function Popover({
  open,
  onClose,
  anchorRef,
  children,
  role = "menu",
  placement = "bottom-start",
  className,
  ...rest
}: PopoverProps) {
  const panelRef = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState<Resolved | null>(null);

  // Reposition against the anchor whenever the popover opens, the window
  // resizes, or the page scrolls underneath it. Uses `fixed` positioning
  // (viewport-relative rects), so no scroll-offset math is needed.
  useLayoutEffect(() => {
    if (!open) {
      setPos(null);
      return;
    }
    function reposition() {
      const anchor = anchorRef.current;
      const panel = panelRef.current;
      if (!anchor || !panel) return;
      setPos(resolvePosition(anchor.getBoundingClientRect(), panel.getBoundingClientRect(), placement));
    }
    reposition();
    window.addEventListener("resize", reposition);
    window.addEventListener("scroll", reposition, true);
    return () => {
      window.removeEventListener("resize", reposition);
      window.removeEventListener("scroll", reposition, true);
    };
  }, [open, anchorRef, placement]);

  // Esc and outside-click dismissal, scoped to while the popover is open.
  useEffect(() => {
    if (!open) return;
    function onKeyDown(e: KeyboardEvent) {
      if (e.key === "Escape") {
        e.preventDefault();
        onClose();
      }
    }
    function onPointerDown(e: MouseEvent) {
      const target = e.target as Node;
      if (panelRef.current?.contains(target)) return;
      if (anchorRef.current?.contains(target)) return;
      onClose();
    }
    document.addEventListener("keydown", onKeyDown);
    document.addEventListener("mousedown", onPointerDown);
    return () => {
      document.removeEventListener("keydown", onKeyDown);
      document.removeEventListener("mousedown", onPointerDown);
    };
  }, [open, onClose, anchorRef]);

  // Return focus to the anchor once the popover closes/unmounts, so
  // keyboard users land back on the control that opened it.
  useEffect(() => {
    if (!open) return;
    return () => {
      anchorRef.current?.focus();
    };
  }, [open, anchorRef]);

  if (!open) return null;

  return createPortal(
    <div
      ref={panelRef}
      className={className ? `popover ${className}` : "popover"}
      role={role}
      data-placement={pos?.vertical ?? "bottom"}
      style={{
        top: pos?.top ?? -9999,
        left: pos?.left ?? -9999,
        visibility: pos ? "visible" : "hidden",
      }}
      {...rest}
    >
      {children}
    </div>,
    document.body,
  );
}
