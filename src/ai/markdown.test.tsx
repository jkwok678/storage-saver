import { render } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { renderMarkdown } from "./markdown";

function html(text: string) {
  return render(<div>{renderMarkdown(text)}</div>).container.firstElementChild?.innerHTML;
}

describe("renderMarkdown", () => {
  it("renders nothing for an empty reply", () => {
    expect(renderMarkdown("")).toBeNull();
  });

  it("shifts headings down a level under the panel title", () => {
    expect(html("# One\n## Two\n### Three")).toBe("<h2>One</h2><h3>Two</h3><h4>Three</h4>");
  });

  it("keeps fenced code as written", () => {
    expect(html("```\nrm -rf **x**\n```")).toBe("<pre><code>rm -rf **x**</code></pre>");
  });

  it("reads lists that do not follow a blank line", () => {
    expect(html("Clean up:\n- `node_modules`\n* **caches**")).toBe(
      "<p><span>Clean up:</span></p>" +
        "<ul><li><code>node_modules</code></li><li><strong>caches</strong></li></ul>",
    );
  });

  it("keeps the starting number of an ordered list", () => {
    expect(html("3. Three\n4) Four")).toBe('<ol start="3"><li>Three</li><li>Four</li></ol>');
  });

  it("renders tables and rules", () => {
    expect(html("| Path | Size |\n| --- | ---: |\n| /a | 1 GB |\n---")).toBe(
      '<div class="ai-table-wrap"><table>' +
        "<thead><tr><th>Path</th><th>Size</th></tr></thead>" +
        "<tbody><tr><td>/a</td><td>1 GB</td></tr></tbody></table></div><hr>",
    );
  });

  it("treats a lone pipe line as a paragraph", () => {
    expect(html("| not a table")).toBe("<p><span>| not a table</span></p>");
  });

  it("joins wrapped lines into one paragraph until the next block", () => {
    expect(html("first\nsecond\n\nthird\n# Next")).toBe(
      "<p><span>first</span><span><br>second</span></p><p><span>third</span></p><h2>Next</h2>",
    );
  });
});
