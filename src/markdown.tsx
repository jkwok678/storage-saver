import type { ReactNode } from "react";

const BULLET = /^\s*[-*+]\s+(.*)$/;
const NUMBERED = /^\s*(\d+)[.)]\s+(.*)$/;
const HEADING = /^(#{1,6})\s+(.*)$/;
const TABLE_DIVIDER = /^\|?\s*:?-+:?\s*(\|\s*:?-+:?\s*)*\|?$/;

/** Read the markdown the AI writes line by line: lists need not follow a blank line. */
export function renderMarkdown(text: string) {
  if (!text) return null;
  const lines = text.split("\n");
  const blocks: ReactNode[] = [];
  let i = 0;
  const startsBlock = (line: string) =>
    line.trim().startsWith("```") ||
    HEADING.test(line.trim()) ||
    BULLET.test(line) ||
    NUMBERED.test(line) ||
    line.trim().startsWith("|");

  while (i < lines.length) {
    const line = lines[i];
    const trimmed = line.trim();
    const key = blocks.length;
    if (!trimmed) {
      i += 1;
      continue;
    }
    if (trimmed.startsWith("```")) {
      const code: string[] = [];
      i += 1;
      while (i < lines.length && !lines[i].trim().startsWith("```")) {
        code.push(lines[i]);
        i += 1;
      }
      i += 1;
      blocks.push(
        <pre key={key}>
          <code>{code.join("\n")}</code>
        </pre>,
      );
      continue;
    }
    const heading = HEADING.exec(trimmed);
    if (heading) {
      const content = formatInline(heading[2]);
      const level = heading[1].length;
      blocks.push(
        level === 1 ? (
          <h2 key={key}>{content}</h2>
        ) : level === 2 ? (
          <h3 key={key}>{content}</h3>
        ) : (
          <h4 key={key}>{content}</h4>
        ),
      );
      i += 1;
      continue;
    }
    if (/^(-{3,}|\*{3,}|_{3,})$/.test(trimmed)) {
      blocks.push(<hr key={key} />);
      i += 1;
      continue;
    }
    if (BULLET.test(line)) {
      const items: string[] = [];
      while (i < lines.length && BULLET.test(lines[i])) {
        items.push(BULLET.exec(lines[i])![1]);
        i += 1;
      }
      blocks.push(
        <ul key={key}>
          {items.map((item, itemIdx) => (
            <li key={itemIdx}>{formatInline(item)}</li>
          ))}
        </ul>,
      );
      continue;
    }
    if (NUMBERED.test(line)) {
      // Items split by blank lines or sub-bullets keep their own numbers.
      const start = Number(NUMBERED.exec(line)![1]);
      const items: string[] = [];
      while (i < lines.length && NUMBERED.test(lines[i])) {
        items.push(NUMBERED.exec(lines[i])![2]);
        i += 1;
      }
      blocks.push(
        <ol key={key} start={start}>
          {items.map((item, itemIdx) => (
            <li key={itemIdx}>{formatInline(item)}</li>
          ))}
        </ol>,
      );
      continue;
    }
    if (trimmed.startsWith("|") && i + 1 < lines.length && TABLE_DIVIDER.test(lines[i + 1].trim())) {
      const cells = (row: string) =>
        row
          .trim()
          .replace(/^\|/, "")
          .replace(/\|$/, "")
          .split("|")
          .map((cell) => cell.trim());
      const header = cells(line);
      const rows: string[][] = [];
      i += 2;
      while (i < lines.length && lines[i].trim().startsWith("|")) {
        rows.push(cells(lines[i]));
        i += 1;
      }
      blocks.push(
        <div key={key} className="ai-table-wrap">
          <table>
            <thead>
              <tr>
                {header.map((cell, cellIdx) => (
                  <th key={cellIdx}>{formatInline(cell)}</th>
                ))}
              </tr>
            </thead>
            <tbody>
              {rows.map((row, rowIdx) => (
                <tr key={rowIdx}>
                  {row.map((cell, cellIdx) => (
                    <td key={cellIdx}>{formatInline(cell)}</td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        </div>,
      );
      continue;
    }
    const paragraph: string[] = [trimmed];
    i += 1;
    while (i < lines.length && lines[i].trim() && !startsBlock(lines[i])) {
      paragraph.push(lines[i].trim());
      i += 1;
    }
    blocks.push(
      <p key={key}>
        {paragraph.map((part, partIdx) => (
          <span key={partIdx}>
            {partIdx > 0 && <br />}
            {formatInline(part)}
          </span>
        ))}
      </p>,
    );
  }
  return <>{blocks}</>;
}

function formatInline(text: string) {
  const parts = text.split(/(`[^`]+`|\*\*[^*]+\*\*)/g);
  return parts.map((part, index) => {
    if (part.startsWith("`") && part.endsWith("`")) {
      return <code key={index}>{part.slice(1, -1)}</code>;
    }
    if (part.startsWith("**") && part.endsWith("**")) {
      return <strong key={index}>{part.slice(2, -2)}</strong>;
    }
    return part;
  });
}
