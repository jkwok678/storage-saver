import type { ReactNode } from "react";

const BULLET = /^\s*[-*+]\s+(.*)$/;
const NUMBERED = /^\s*(\d+)[.)]\s+(.*)$/;
const HEADING = /^(#{1,6})\s+(.*)$/;
const RULE = /^(-{3,}|\*{3,}|_{3,})$/;
const TABLE_DIVIDER = /^\|?\s*:?-+:?\s*(\|\s*:?-+:?\s*)*\|?$/;
const FENCE = "```";

/** A piece of text keyed by where it starts, so React keys stay stable while a reply streams. */
type Line = { key: number; text: string };

type Row = { key: number; cells: Line[] };

type Block =
  | { kind: "code"; key: number; text: string }
  | { kind: "heading"; key: number; level: number; text: string }
  | { kind: "rule"; key: number }
  | { kind: "list"; key: number; start: number | null; items: Line[] }
  | { kind: "table"; key: number; header: Line[]; rows: Row[] }
  | { kind: "paragraph"; key: number; lines: Line[] };

/** A block read from `lines` at `at`, and the line after it. */
type Read = { block: Block; next: number };

function takeWhile(lines: string[], from: number, keep: (line: string) => boolean) {
  let next = from;
  while (next < lines.length && keep(lines[next])) next += 1;
  return next;
}

function readCode(lines: string[], at: number): Read | null {
  if (!lines[at].trim().startsWith(FENCE)) return null;
  const end = takeWhile(lines, at + 1, (line) => !line.trim().startsWith(FENCE));
  return {
    block: { kind: "code", key: at, text: lines.slice(at + 1, end).join("\n") },
    next: end + 1,
  };
}

function readHeading(lines: string[], at: number): Read | null {
  const match = HEADING.exec(lines[at].trim());
  if (!match) return null;
  return {
    block: { kind: "heading", key: at, level: match[1].length, text: match[2] },
    next: at + 1,
  };
}

function readRule(lines: string[], at: number): Read | null {
  if (!RULE.test(lines[at].trim())) return null;
  return { block: { kind: "rule", key: at }, next: at + 1 };
}

function listItems(lines: string[], at: number, end: number, pattern: RegExp, group: number) {
  return lines.slice(at, end).map((line, offset) => ({
    key: at + offset,
    text: pattern.exec(line)?.[group] ?? line,
  }));
}

function readBullets(lines: string[], at: number): Read | null {
  if (!BULLET.test(lines[at])) return null;
  const end = takeWhile(lines, at, (line) => BULLET.test(line));
  return {
    block: { kind: "list", key: at, start: null, items: listItems(lines, at, end, BULLET, 1) },
    next: end,
  };
}

function readNumbered(lines: string[], at: number): Read | null {
  const first = NUMBERED.exec(lines[at]);
  if (!first) return null;
  // Items split by blank lines or sub-bullets keep their own numbers.
  const end = takeWhile(lines, at, (line) => NUMBERED.test(line));
  const items = listItems(lines, at, end, NUMBERED, 2);
  return { block: { kind: "list", key: at, start: Number(first[1]), items }, next: end };
}

function cells(row: string): Line[] {
  return row
    .trim()
    .replace(/^\|/, "")
    .replace(/\|$/, "")
    .split("|")
    .map((cell, column) => ({ key: column, text: cell.trim() }));
}

function readTable(lines: string[], at: number): Read | null {
  const isTable =
    lines[at].trim().startsWith("|") &&
    at + 1 < lines.length &&
    TABLE_DIVIDER.test(lines[at + 1].trim());
  if (!isTable) return null;
  const end = takeWhile(lines, at + 2, (line) => line.trim().startsWith("|"));
  const rows = lines
    .slice(at + 2, end)
    .map((line, offset) => ({ key: at + 2 + offset, cells: cells(line) }));
  return { block: { kind: "table", key: at, header: cells(lines[at]), rows }, next: end };
}

const BLOCK_READERS = [readCode, readHeading, readRule, readBullets, readNumbered, readTable];

function startsBlock(line: string) {
  const trimmed = line.trim();
  return (
    trimmed.startsWith(FENCE) ||
    trimmed.startsWith("|") ||
    HEADING.test(trimmed) ||
    BULLET.test(line) ||
    NUMBERED.test(line)
  );
}

function readParagraph(lines: string[], at: number): Read {
  const end = takeWhile(lines, at + 1, (line) => line.trim() !== "" && !startsBlock(line));
  const paragraph = lines
    .slice(at, end)
    .map((line, offset) => ({ key: at + offset, text: line.trim() }));
  return { block: { kind: "paragraph", key: at, lines: paragraph }, next: end };
}

/** Read the markdown the AI writes line by line: lists need not follow a blank line. */
function parseMarkdown(text: string): Block[] {
  const lines = text.split("\n");
  const blocks: Block[] = [];
  let at = 0;
  while (at < lines.length) {
    if (!lines[at].trim()) {
      at += 1;
      continue;
    }
    const read =
      BLOCK_READERS.map((reader) => reader(lines, at)).find((found) => found !== null) ??
      readParagraph(lines, at);
    blocks.push(read.block);
    at = read.next;
  }
  return blocks;
}

function Heading({ level, text }: { level: number; text: string }) {
  const content = formatInline(text);
  if (level === 1) return <h2>{content}</h2>;
  if (level === 2) return <h3>{content}</h3>;
  return <h4>{content}</h4>;
}

function List({ start, items }: { start: number | null; items: Line[] }) {
  const children = items.map((item) => <li key={item.key}>{formatInline(item.text)}</li>);
  return start === null ? <ul>{children}</ul> : <ol start={start}>{children}</ol>;
}

function Table({ header, rows }: { header: Line[]; rows: Row[] }) {
  return (
    <div className="ai-table-wrap">
      <table>
        <thead>
          <tr>
            {header.map((cell) => (
              <th key={cell.key}>{formatInline(cell.text)}</th>
            ))}
          </tr>
        </thead>
        <tbody>
          {rows.map((row) => (
            <tr key={row.key}>
              {row.cells.map((cell) => (
                <td key={cell.key}>{formatInline(cell.text)}</td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function Paragraph({ lines }: { lines: Line[] }) {
  return (
    <p>
      {lines.map((line, index) => (
        <span key={line.key}>
          {index > 0 && <br />}
          {formatInline(line.text)}
        </span>
      ))}
    </p>
  );
}

function renderBlock(block: Block) {
  switch (block.kind) {
    case "code":
      return (
        <pre key={block.key}>
          <code>{block.text}</code>
        </pre>
      );
    case "heading":
      return <Heading key={block.key} level={block.level} text={block.text} />;
    case "rule":
      return <hr key={block.key} />;
    case "list":
      return <List key={block.key} start={block.start} items={block.items} />;
    case "table":
      return <Table key={block.key} header={block.header} rows={block.rows} />;
    case "paragraph":
      return <Paragraph key={block.key} lines={block.lines} />;
  }
}

export function renderMarkdown(text: string) {
  if (!text) return null;
  return <>{parseMarkdown(text).map(renderBlock)}</>;
}

function formatInline(text: string): ReactNode[] {
  let offset = 0;
  return text.split(/(`[^`]+`|\*\*[^*]+\*\*)/g).map((part) => {
    const key = offset;
    offset += part.length;
    if (part.startsWith("`") && part.endsWith("`")) {
      return <code key={key}>{part.slice(1, -1)}</code>;
    }
    if (part.startsWith("**") && part.endsWith("**")) {
      return <strong key={key}>{part.slice(2, -2)}</strong>;
    }
    return part;
  });
}
