export function FolderIcon() {
  return (
    <svg className="icon folder" viewBox="0 0 16 16" aria-hidden="true">
      <path
        d="M1.4 3.9c0-.8.6-1.4 1.4-1.4h2.9l1.5 1.8h6.1c.8 0 1.4.6 1.4 1.4v6.4c0 .8-.6 1.4-1.4 1.4H2.8c-.8 0-1.4-.6-1.4-1.4z"
        fill="#4c9df2"
      />
      <path
        d="M1.4 5.4h13.2v1H1.4z"
        fill="#fff"
        fillOpacity=".35"
      />
    </svg>
  );
}

export function FileIcon() {
  return (
    <svg className="icon file" viewBox="0 0 16 16" aria-hidden="true">
      <path
        d="M3.5 1.5h5.7l3.3 3.3v9.2c0 .3-.2.5-.5.5H3.5a.5.5 0 0 1-.5-.5V2c0-.3.2-.5.5-.5z"
        fill="currentColor"
      />
      <path d="M9.2 1.5v3.3h3.3" fill="#fff" fillOpacity=".55" />
    </svg>
  );
}
