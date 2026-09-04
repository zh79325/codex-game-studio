import type { ReactNode } from "react";

export default function MarkdownDocument({ content }: { content: string }) {
  const lines = content.replaceAll("\r\n", "\n").split("\n");
  const blocks: ReactNode[] = [];
  let index = 0;

  while (index < lines.length) {
    const line = lines[index];
    if (!line.trim()) {
      index += 1;
      continue;
    }

    if (line.trimStart().startsWith("```")) {
      const language = line.trim().slice(3).trim();
      const code: string[] = [];
      index += 1;
      while (
        index < lines.length &&
        !lines[index].trimStart().startsWith("```")
      ) {
        code.push(lines[index]);
        index += 1;
      }
      index += index < lines.length ? 1 : 0;
      blocks.push(
        <pre className="markdown-code" key={`code-${index}`}>
          <code data-language={language || undefined}>{code.join("\n")}</code>
        </pre>,
      );
      continue;
    }

    const heading = /^(#{1,6})\s+(.+)$/.exec(line);
    if (heading) {
      const body = renderInline(heading[2], `heading-${index}`);
      const level = heading[1].length;
      blocks.push(
        level === 1 ? (
          <h1 key={`heading-${index}`}>{body}</h1>
        ) : level === 2 ? (
          <h2 key={`heading-${index}`}>{body}</h2>
        ) : level === 3 ? (
          <h3 key={`heading-${index}`}>{body}</h3>
        ) : level === 4 ? (
          <h4 key={`heading-${index}`}>{body}</h4>
        ) : level === 5 ? (
          <h5 key={`heading-${index}`}>{body}</h5>
        ) : (
          <h6 key={`heading-${index}`}>{body}</h6>
        ),
      );
      index += 1;
      continue;
    }

    if (/^\s*([-*_])(?:\s*\1){2,}\s*$/.test(line)) {
      blocks.push(<hr key={`rule-${index}`} />);
      index += 1;
      continue;
    }

    if (isTableHeader(lines, index)) {
      const header = splitTableRow(lines[index]);
      const rows: string[][] = [];
      index += 2;
      while (
        index < lines.length &&
        /^\s*\|?.+\|.+\|?\s*$/.test(lines[index])
      ) {
        rows.push(splitTableRow(lines[index]));
        index += 1;
      }
      blocks.push(
        <div className="markdown-table-wrap" key={`table-${index}`}>
          <table>
            <thead>
              <tr>
                {header.map((cell, cellIndex) => (
                  <th key={cellIndex}>
                    {renderInline(cell, `th-${cellIndex}`)}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {rows.map((row, rowIndex) => (
                <tr key={rowIndex}>
                  {header.map((_, cellIndex) => (
                    <td key={cellIndex}>
                      {renderInline(
                        row[cellIndex] ?? "",
                        `td-${rowIndex}-${cellIndex}`,
                      )}
                    </td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        </div>,
      );
      continue;
    }

    const unordered = /^\s*[-+*]\s+(.+)$/.exec(line);
    const ordered = /^\s*\d+[.)]\s+(.+)$/.exec(line);
    if (unordered || ordered) {
      const items: string[] = [];
      const pattern = unordered ? /^\s*[-+*]\s+(.+)$/ : /^\s*\d+[.)]\s+(.+)$/;
      while (index < lines.length) {
        const match = pattern.exec(lines[index]);
        if (!match) break;
        items.push(match[1]);
        index += 1;
      }
      const children = items.map((item, itemIndex) => (
        <li key={itemIndex}>{renderInline(item, `item-${itemIndex}`)}</li>
      ));
      blocks.push(
        unordered ? (
          <ul key={`list-${index}`}>{children}</ul>
        ) : (
          <ol key={`list-${index}`}>{children}</ol>
        ),
      );
      continue;
    }

    if (/^\s*>/.test(line)) {
      const quote: string[] = [];
      while (index < lines.length && /^\s*>/.test(lines[index])) {
        quote.push(lines[index].replace(/^\s*>\s?/, ""));
        index += 1;
      }
      blocks.push(
        <blockquote key={`quote-${index}`}>
          {renderInline(quote.join(" "), `quote-${index}`)}
        </blockquote>,
      );
      continue;
    }

    const paragraph: string[] = [];
    while (
      index < lines.length &&
      lines[index].trim() &&
      !startsBlock(lines, index)
    ) {
      paragraph.push(lines[index].trim());
      index += 1;
    }
    if (!paragraph.length) {
      paragraph.push(line.trim());
      index += 1;
    }
    blocks.push(
      <p key={`paragraph-${index}`}>
        {renderInline(paragraph.join(" "), `paragraph-${index}`)}
      </p>,
    );
  }

  return <article className="markdown-document">{blocks}</article>;
}

function startsBlock(lines: string[], index: number) {
  const line = lines[index];
  return (
    line.trimStart().startsWith("```") ||
    /^#{1,6}\s+/.test(line) ||
    /^\s*([-*_])(?:\s*\1){2,}\s*$/.test(line) ||
    /^\s*[-+*]\s+/.test(line) ||
    /^\s*\d+[.)]\s+/.test(line) ||
    /^\s*>/.test(line) ||
    isTableHeader(lines, index)
  );
}

function isTableHeader(lines: string[], index: number) {
  return (
    index + 1 < lines.length &&
    lines[index].includes("|") &&
    /^\s*\|?\s*:?-{3,}:?\s*(\|\s*:?-{3,}:?\s*)+\|?\s*$/.test(lines[index + 1])
  );
}

function splitTableRow(line: string) {
  return line
    .trim()
    .replace(/^\||\|$/g, "")
    .split("|")
    .map((cell) => cell.trim());
}

function renderInline(value: string, keyPrefix: string) {
  const tokenPattern =
    /(\*\*[^*]+\*\*|__[^_]+__|`[^`]+`|\[[^\]]+\]\([^)]+\)|\*[^*]+\*|_[^_]+_)/g;
  const nodes: ReactNode[] = [];
  let cursor = 0;
  for (const [tokenIndex, match] of [
    ...value.matchAll(tokenPattern),
  ].entries()) {
    const start = match.index;
    if (start > cursor) nodes.push(value.slice(cursor, start));
    const token = match[0];
    const key = `${keyPrefix}-${tokenIndex}`;
    if (
      (token.startsWith("**") && token.endsWith("**")) ||
      (token.startsWith("__") && token.endsWith("__"))
    ) {
      nodes.push(<strong key={key}>{token.slice(2, -2)}</strong>);
    } else if (token.startsWith("`") && token.endsWith("`")) {
      nodes.push(<code key={key}>{token.slice(1, -1)}</code>);
    } else if (token.startsWith("[")) {
      const link = /^\[([^\]]+)]\(([^)]+)\)$/.exec(token);
      const href = link?.[2].trim() ?? "";
      nodes.push(
        href.startsWith("https://") || href.startsWith("http://") ? (
          <a key={key} href={href} target="_blank" rel="noreferrer">
            {link?.[1]}
          </a>
        ) : (
          <span key={key}>{link?.[1]}</span>
        ),
      );
    } else {
      nodes.push(<em key={key}>{token.slice(1, -1)}</em>);
    }
    cursor = start + token.length;
  }
  if (cursor < value.length) nodes.push(value.slice(cursor));
  return nodes;
}
