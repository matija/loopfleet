// The app's single icon module: hand-rolled stroke SVGs on a 24-unit
// viewBox, sharing a common visual language (currentColor stroke, round
// caps/joins) so glyphs read as one family regardless of where they're used.

export type IconProps = {
  size?: number;
  className?: string;
};

const defaultProps = {
  viewBox: "0 0 24 24",
  fill: "none",
  stroke: "currentColor",
  strokeWidth: "1.5",
  strokeLinecap: "round" as const,
  strokeLinejoin: "round" as const,
};

export function FolderIcon({ size = 16, className }: IconProps) {
  return (
    <svg width={size} height={size} className={className} {...defaultProps}>
      <path d="M3 6a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v9a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z" />
    </svg>
  );
}

export function FolderPlusIcon({ size = 16, className }: IconProps) {
  return (
    <svg width={size} height={size} className={className} {...defaultProps}>
      <path d="M3 6a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v9a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z" />
      <path d="M12 10.5v5M9.5 13h5" />
    </svg>
  );
}

export function SearchIcon({ size = 16, className }: IconProps) {
  return (
    <svg width={size} height={size} className={className} {...defaultProps}>
      <circle cx="11" cy="11" r="7" />
      <path d="M21 21l-4.3-4.3" />
    </svg>
  );
}

export function ComposeIcon({ size = 16, className }: IconProps) {
  return (
    <svg width={size} height={size} className={className} {...defaultProps}>
      <path d="M12 20h9" />
      <path d="M16.5 3.5a2.1 2.1 0 0 1 3 3L7 19l-4 1 1-4z" />
    </svg>
  );
}

export function ChevronDownIcon({ size = 16, className }: IconProps) {
  return (
    <svg width={size} height={size} className={className} {...defaultProps}>
      <path d="M6 9l6 6 6-6" />
    </svg>
  );
}

export function ChevronRightIcon({ size = 16, className }: IconProps) {
  return (
    <svg width={size} height={size} className={className} {...defaultProps}>
      <path d="M9 6l6 6-6 6" />
    </svg>
  );
}

export function GitBranchIcon({ size = 16, className }: IconProps) {
  return (
    <svg width={size} height={size} className={className} {...defaultProps}>
      <circle cx="6" cy="5" r="2.5" />
      <circle cx="6" cy="19" r="2.5" />
      <circle cx="18" cy="8" r="2.5" />
      <path d="M6 7.5v9" />
      <path d="M6 12c0 3 3 4 6 4h3.5" />
      <path d="M18 10.5V15" />
    </svg>
  );
}

export function BoxIcon({ size = 16, className }: IconProps) {
  return (
    <svg width={size} height={size} className={className} {...defaultProps}>
      <path d="M3.5 7.5l8.5-4.5 8.5 4.5-8.5 4.5-8.5-4.5z" />
      <path d="M3.5 7.5v9l8.5 4.5 8.5-4.5v-9" />
      <path d="M12 12v9" />
    </svg>
  );
}

export function PlayIcon({ size = 16, className }: IconProps) {
  return (
    <svg width={size} height={size} className={className} {...defaultProps}>
      <path d="M6 4.5v15l13-7.5z" />
    </svg>
  );
}

export function SquareIcon({ size = 16, className }: IconProps) {
  return (
    <svg width={size} height={size} className={className} {...defaultProps}>
      <rect x="5" y="5" width="14" height="14" rx="1.5" />
    </svg>
  );
}

export function DiffIcon({ size = 16, className }: IconProps) {
  return (
    <svg width={size} height={size} className={className} {...defaultProps}>
      <path d="M9 3v6M6 6h6" />
      <path d="M15 15v6M12 18h6" />
      <path d="M4 12h16" />
    </svg>
  );
}

export function TerminalIcon({ size = 16, className }: IconProps) {
  return (
    <svg width={size} height={size} className={className} {...defaultProps}>
      <rect x="3" y="4" width="18" height="16" rx="2" />
      <path d="M7 9l4 3-4 3" />
      <path d="M13 15h4" />
    </svg>
  );
}

export function FilesIcon({ size = 16, className }: IconProps) {
  return (
    <svg width={size} height={size} className={className} {...defaultProps}>
      <path d="M7 3h8l4 4v13a1 1 0 0 1-1 1H7a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1z" />
      <path d="M15 3v4h4" />
      <path d="M4 8v12a1 1 0 0 0 1 1h9" />
    </svg>
  );
}

export function AgentIcon({ size = 16, className }: IconProps) {
  return (
    <svg width={size} height={size} className={className} {...defaultProps}>
      <rect x="4" y="8" width="16" height="11" rx="2" />
      <path d="M12 8V4" />
      <circle cx="12" cy="3" r="1" />
      <circle cx="9" cy="13.5" r="1.25" />
      <circle cx="15" cy="13.5" r="1.25" />
      <path d="M9 17h6" />
    </svg>
  );
}

export function SettingsIcon({ size = 16, className }: IconProps) {
  return (
    <svg width={size} height={size} className={className} {...defaultProps}>
      <circle cx="12" cy="12" r="3" />
      <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 1 1-4 0v-.09a1.65 1.65 0 0 0-1-1.51 1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 1 1 0-4h.09a1.65 1.65 0 0 0 1.51-1 1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 1 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 1 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" />
    </svg>
  );
}

export function CheckIcon({ size = 16, className }: IconProps) {
  return (
    <svg width={size} height={size} className={className} {...defaultProps}>
      <path d="M5 12l5 5L20 6" />
    </svg>
  );
}

export function ClockIcon({ size = 16, className }: IconProps) {
  return (
    <svg width={size} height={size} className={className} {...defaultProps}>
      <circle cx="12" cy="12" r="9" />
      <path d="M12 7v5l3.5 2" />
    </svg>
  );
}

export function AlertIcon({ size = 16, className }: IconProps) {
  return (
    <svg width={size} height={size} className={className} {...defaultProps}>
      <path d="M12 3l10 18H2z" />
      <path d="M12 10v4" />
      <path d="M12 17.5v.01" />
    </svg>
  );
}

export function InfoIcon({ size = 16, className }: IconProps) {
  return (
    <svg width={size} height={size} className={className} {...defaultProps}>
      <circle cx="12" cy="12" r="9" />
      <path d="M12 11v5" />
      <path d="M12 7.5v.01" />
    </svg>
  );
}

export function DotIcon({ size = 16, className }: IconProps) {
  return (
    <svg width={size} height={size} className={className} {...defaultProps}>
      <circle cx="12" cy="12" r="3" fill="currentColor" stroke="none" />
    </svg>
  );
}

export function DownloadIcon({ size = 16, className }: IconProps) {
  return (
    <svg width={size} height={size} className={className} {...defaultProps}>
      <path d="M12 3v12" />
      <path d="M7 10l5 5 5-5" />
      <path d="M4 19.5h16" />
    </svg>
  );
}

export function XIcon({ size = 16, className }: IconProps) {
  return (
    <svg width={size} height={size} className={className} {...defaultProps}>
      <path d="M6 6l12 12M18 6L6 18" />
    </svg>
  );
}

export function PanelLeftIcon({ size = 16, className }: IconProps) {
  return (
    <svg width={size} height={size} className={className} {...defaultProps}>
      <rect x="3" y="4" width="18" height="16" rx="2" />
      <path d="M9 4v16" />
    </svg>
  );
}

export function PanelBottomIcon({ size = 16, className }: IconProps) {
  return (
    <svg width={size} height={size} className={className} {...defaultProps}>
      <rect x="3" y="4" width="18" height="16" rx="2" />
      <path d="M3 15h18" />
    </svg>
  );
}

export function MaximizeIcon({ size = 16, className }: IconProps) {
  return (
    <svg width={size} height={size} className={className} {...defaultProps}>
      <path d="M9 3H5a2 2 0 0 0-2 2v4" />
      <path d="M15 3h4a2 2 0 0 1 2 2v4" />
      <path d="M9 21H5a2 2 0 0 1-2-2v-4" />
      <path d="M15 21h4a2 2 0 0 0 2-2v-4" />
    </svg>
  );
}

// A short checklist glyph: rows of tasks, one already ticked.
export function ChecklistIcon({ size = 16, className }: IconProps) {
  return (
    <svg width={size} height={size} className={className} {...defaultProps}>
      <path d="M4.5 6l1.5 1.5L9 4.5" />
      <path d="M12 6h7.5" />
      <path d="M5 12h.01M5 18h.01" />
      <path d="M12 12h7.5M12 18h7.5" />
    </svg>
  );
}

export function PlusIcon({ size = 16, className }: IconProps) {
  return (
    <svg width={size} height={size} className={className} {...defaultProps}>
      <path d="M12 5v14M5 12h14" />
    </svg>
  );
}

export function MinusIcon({ size = 16, className }: IconProps) {
  return (
    <svg width={size} height={size} className={className} {...defaultProps}>
      <path d="M5 12h14" />
    </svg>
  );
}

// A document-with-checklist glyph: a plan file waiting to be added.
export function DocPlusIcon({ size = 26, className }: IconProps) {
  return (
    <svg width={size} height={size} className={className} {...defaultProps}>
      <path d="M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8z" />
      <path d="M14 3v5h5" />
      <path d="M8.5 13h7M8.5 16.5h4.5" />
    </svg>
  );
}
