// A minimal, dependency-free Markdown renderer for the frozen PRD document view.
// The project holds a deliberate no-new-dependency line (see fuzzy.ts: "PRD
// non-goal: no new dependency"), so rather than pull a Markdown library we render
// only the constructs a PRD actually uses: ATX headings, paragraphs, bullet /
// ordered / task lists (nested by indent), fenced code blocks, blockquotes, and
// horizontal rules — with inline code, bold, italic, and links.
//
// Output is built as React elements, never `dangerouslySetInnerHTML`, so authored
// document text can never inject markup: an `<script>` in the source renders as
// the literal characters, not an element.

import type { ReactNode } from "react";

const HEADING = /^(#{1,6})\s+(.*)$/;
const HR = /^\s*(?:-{3,}|\*{3,}|_{3,})\s*$/;
const LIST_ITEM = /^(\s*)([-*+]|\d+\.)\s+(.*)$/;
const TASK = /^\[([ xX])\]\s+(.*)$/;

type ListItem = {
  indent: number;
  ordered: boolean;
  checked: boolean | null;
  text: string;
};

/// Render a whole Markdown document to a flat array of block elements.
export function renderMarkdown(source: string): ReactNode[] {
  const lines = source.replace(/\r\n?/g, "\n").split("\n");
  const blocks: ReactNode[] = [];
  let i = 0;
  let key = 0;

  while (i < lines.length) {
    const line = lines[i];

    // Blank line — no output, just a separator between blocks.
    if (line.trim() === "") {
      i++;
      continue;
    }

    // Fenced code block: ```lang … ``` (verbatim, no inline parsing inside).
    const fence = line.match(/^\s*```(.*)$/);
    if (fence) {
      const body: string[] = [];
      i++;
      while (i < lines.length && !/^\s*```\s*$/.test(lines[i])) {
        body.push(lines[i]);
        i++;
      }
      i++; // consume the closing fence (or run off the end)
      blocks.push(
        <pre className="md-pre" key={key++}>
          <code>{body.join("\n")}</code>
        </pre>,
      );
      continue;
    }

    // ATX heading.
    const heading = line.match(HEADING);
    if (heading) {
      const level = heading[1].length;
      const Tag = `h${Math.min(level + 1, 6)}` as "h2";
      blocks.push(
        <Tag className={`md-h md-h${level}`} key={key++}>
          {renderInline(heading[2])}
        </Tag>,
      );
      i++;
      continue;
    }

    // Horizontal rule.
    if (HR.test(line)) {
      blocks.push(<hr className="md-hr" key={key++} />);
      i++;
      continue;
    }

    // Blockquote: consecutive `>`-prefixed lines, rendered as one quote.
    if (/^\s*>/.test(line)) {
      const quoted: string[] = [];
      while (i < lines.length && /^\s*>/.test(lines[i])) {
        quoted.push(lines[i].replace(/^\s*>\s?/, ""));
        i++;
      }
      blocks.push(
        <blockquote className="md-quote" key={key++}>
          {renderInline(quoted.join(" "))}
        </blockquote>,
      );
      continue;
    }

    // List: a contiguous run of list-item lines (any marker / indent).
    if (LIST_ITEM.test(line)) {
      const items: ListItem[] = [];
      while (i < lines.length && LIST_ITEM.test(lines[i])) {
        const m = lines[i].match(LIST_ITEM)!;
        const rest = m[3];
        const task = rest.match(TASK);
        items.push({
          indent: m[1].length,
          ordered: /\d/.test(m[2]),
          checked: task ? task[1].toLowerCase() === "x" : null,
          text: task ? task[2] : rest,
        });
        i++;
      }
      blocks.push(renderList(items, `${key++}`));
      continue;
    }

    // Otherwise a paragraph: gather until a blank line or a block starter.
    const para: string[] = [];
    while (
      i < lines.length &&
      lines[i].trim() !== "" &&
      !HEADING.test(lines[i]) &&
      !HR.test(lines[i]) &&
      !LIST_ITEM.test(lines[i]) &&
      !/^\s*>/.test(lines[i]) &&
      !/^\s*```/.test(lines[i])
    ) {
      para.push(lines[i]);
      i++;
    }
    blocks.push(
      <p className="md-p" key={key++}>
        {renderInline(para.join(" "))}
      </p>,
    );
  }

  return blocks;
}

// Build a (possibly nested) list from a flat run of items grouped by indent:
// lines more indented than an item belong to that item's sublist.
function renderList(items: ListItem[], keyPrefix: string): ReactNode {
  const base = Math.min(...items.map((it) => it.indent));
  const ordered = items.find((it) => it.indent === base)?.ordered ?? false;
  const children: ReactNode[] = [];
  let i = 0;
  while (i < items.length) {
    const item = items[i];
    let j = i + 1;
    while (j < items.length && items[j].indent > item.indent) j++;
    const sub = items.slice(i + 1, j);
    children.push(
      <li className="md-li" key={`${keyPrefix}-${i}`}>
        {item.checked !== null && (
          <input
            type="checkbox"
            className="md-check"
            checked={item.checked}
            disabled
            aria-hidden
          />
        )}
        {renderInline(item.text)}
        {sub.length > 0 && renderList(sub, `${keyPrefix}-${i}s`)}
      </li>,
    );
    i = j;
  }
  return ordered ? (
    <ol className="md-ol" key={keyPrefix}>
      {children}
    </ol>
  ) : (
    <ul className="md-ul" key={keyPrefix}>
      {children}
    </ul>
  );
}

// Inline: `code`, **bold**, *italic* / _italic_, and [label](url). Scanned by a
// single alternation that yields the earliest match; text between matches is
// emitted verbatim. Bold/italic recurse so nesting (**a *b***) resolves.
const INLINE =
  /`([^`]+)`|\*\*([^*]+?)\*\*|__([^_]+?)__|\*([^*]+?)\*|(?<![\w*])_([^_]+?)_(?![\w])|\[([^\]]+)\]\(([^)\s]+)\)/;

function renderInline(text: string): ReactNode[] {
  const out: ReactNode[] = [];
  let rest = text;
  let key = 0;
  while (rest.length > 0) {
    const m = rest.match(INLINE);
    if (!m || m.index === undefined) {
      out.push(rest);
      break;
    }
    if (m.index > 0) out.push(rest.slice(0, m.index));
    if (m[1] !== undefined) {
      out.push(
        <code className="md-code" key={key++}>
          {m[1]}
        </code>,
      );
    } else if (m[2] !== undefined || m[3] !== undefined) {
      out.push(<strong key={key++}>{renderInline(m[2] ?? m[3])}</strong>);
    } else if (m[4] !== undefined || m[5] !== undefined) {
      out.push(<em key={key++}>{renderInline(m[4] ?? m[5])}</em>);
    } else {
      // Link. External URLs open in a new tab; the frozen document is read-only.
      out.push(
        <a
          className="md-link"
          href={m[7]}
          target="_blank"
          rel="noreferrer noopener"
          key={key++}
        >
          {m[6]}
        </a>,
      );
    }
    rest = rest.slice(m.index + m[0].length);
  }
  return out;
}
