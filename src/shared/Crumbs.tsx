import { breadcrumbs } from "./format";

type Props = { path: string; onNavigate: (path: string) => void };

export function Crumbs({ path, onNavigate }: Props) {
  return (
    <nav className="crumbs" aria-label="Folder">
      {breadcrumbs(path).map((crumb, index) => (
        <span key={crumb.path} className="crumb">
          {index > 0 && <span className="sep">/</span>}
          {crumb.path === path ? (
            <span className="current">{crumb.label}</span>
          ) : (
            <button type="button" onClick={() => onNavigate(crumb.path)}>
              {crumb.label}
            </button>
          )}
        </span>
      ))}
    </nav>
  );
}
