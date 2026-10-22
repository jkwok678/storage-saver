import { useState } from "react";
import { breadcrumbs, formatBytes } from "../shared/format";
import { arcHsl } from "./color";
import { annularPath } from "./geometry";
import { CENTER, type LayoutNode, percentOfParent, segmentAriaLabel, VIEWBOX } from "./layout";

type SunburstChartProps = {
  rootPath: string;
  nodes: LayoutNode[];
  totalBytes: number;
  scanning: boolean;
  onZoom: (path: string) => void;
  onReveal: (path: string) => void;
};

type TooltipPos = { x: number; y: number };

export function SunburstChart({
  rootPath,
  nodes,
  totalBytes,
  scanning,
  onZoom,
  onReveal,
}: SunburstChartProps) {
  const [hovered, setHovered] = useState<LayoutNode | null>(null);
  const [tooltipPos, setTooltipPos] = useState<TooltipPos | null>(null);

  const crumbs = breadcrumbs(rootPath);
  const centreName = crumbs[crumbs.length - 1]?.label ?? rootPath;
  const centreSize = scanning && totalBytes === 0 ? "Measuring…" : formatBytes(totalBytes);
  const summary = `Disk map of ${centreName}, ${centreSize}`;

  return (
    <figure className="sunburst-wrap" aria-label={summary}>
      <svg className="sunburst" viewBox={`0 0 ${VIEWBOX} ${VIEWBOX}`}>
        <title>{summary}</title>
        <defs>
          <pattern id="pending-hatch" patternUnits="userSpaceOnUse" width="3" height="3">
            <path d="M0 3L3 0" stroke="currentColor" strokeWidth="0.5" />
          </pattern>
        </defs>
        {nodes.map((node) => (
          <ArcPath
            key={node.id}
            node={node}
            onZoom={onZoom}
            onReveal={onReveal}
            onHover={(next, pos) => {
              setHovered(next);
              setTooltipPos(pos);
            }}
          />
        ))}
      </svg>

      <div className="sunburst-centre" aria-hidden="true">
        <div className="centre-name">{centreName}</div>
        <div className="centre-size">{centreSize}</div>
      </div>

      {hovered && tooltipPos && <ArcTooltip node={hovered} pos={tooltipPos} />}
    </figure>
  );
}

type ArcPathProps = {
  node: LayoutNode;
  onZoom: (path: string) => void;
  onReveal: (path: string) => void;
  onHover: (node: LayoutNode | null, pos: TooltipPos | null) => void;
};

function ArcPath({ node, onZoom, onReveal, onHover }: ArcPathProps) {
  const d = annularPath(
    CENTER,
    CENTER,
    node.innerRadius,
    node.outerRadius,
    node.startAngle,
    node.endAngle,
  );
  if (!d) return null;

  const fill = node.pending || node.hue === null ? undefined : arcHsl(node.hue, node.depth);

  return (
    // biome-ignore lint/a11y/noStaticElementInteractions: every slice has a hover tooltip
    <path
      d={d}
      className={`arc arc-${node.kind}${node.pending ? " arc-pending" : ""}`}
      fill={fill}
      tabIndex={node.clickable ? 0 : undefined}
      role={node.clickable ? "button" : "img"}
      aria-label={segmentAriaLabel(node)}
      onClick={() => {
        if (node.clickable && node.path) onZoom(node.path);
      }}
      onKeyDown={(event) => {
        if ((event.key === "Enter" || event.key === " ") && node.clickable && node.path) {
          event.preventDefault();
          onZoom(node.path);
        }
      }}
      onContextMenu={(event) => {
        if (node.kind === "folder" && node.path) {
          event.preventDefault();
          onReveal(node.path);
        }
      }}
      onMouseEnter={(event) => onHover(node, { x: event.clientX, y: event.clientY })}
      onMouseMove={(event) => onHover(node, { x: event.clientX, y: event.clientY })}
      onMouseLeave={() => onHover(null, null)}
    />
  );
}

function ArcTooltip({ node, pos }: { node: LayoutNode; pos: TooltipPos }) {
  const bytes = node.bytes;
  const pending = node.pending || bytes === null;

  return (
    <div className="sunburst-tooltip" style={{ left: pos.x + 14, top: pos.y - 8 }}>
      <div className="tooltip-name">{node.name}</div>
      {pending ? (
        <div className="tooltip-size">Measuring…</div>
      ) : (
        <>
          <div className="tooltip-size">{formatBytes(bytes)}</div>
          <div className="tooltip-pct">{percentOfParent(bytes, node.parentBytes)}</div>
        </>
      )}
    </div>
  );
}
