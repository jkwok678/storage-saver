export function FolderIcon() {
  return (
    <svg className="icon folder" viewBox="0 0 16 16" aria-hidden="true">
      <path
        d="M1.4 3.9c0-.8.6-1.4 1.4-1.4h2.9l1.5 1.8h6.1c.8 0 1.4.6 1.4 1.4v6.4c0 .8-.6 1.4-1.4 1.4H2.8c-.8 0-1.4-.6-1.4-1.4z"
        fill="#4c9df2"
      />
      <path d="M1.4 5.4h13.2v1H1.4z" fill="#fff" fillOpacity=".35" />
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

export function SparklesIcon() {
  return (
    <svg className="icon sparkles" viewBox="0 0 16 16" fill="currentColor" aria-hidden="true">
      <path d="M8 1.5l1.3 3.2L12.5 6 9.3 7.3 8 10.5 6.7 7.3 3.5 6l3.2-1.3L8 1.5zm4.8 7.7l.8 1.8 1.9.8-1.9.8-.8 1.9-.8-1.9-1.9-.8 1.9-.8.8-1.8zM3.2 8.5l.7 1.5 1.6.7-1.6.7-.7 1.6-.7-1.6-1.5-.7 1.5-.7.7-1.5z" />
    </svg>
  );
}

export function TrashIcon() {
  return (
    <svg className="icon trash" viewBox="0 0 16 16" fill="currentColor" aria-hidden="true">
      <path d="M5.5 1.5h5a.5.5 0 0 1 .5.5v1h3a.5.5 0 0 1 0 1h-.8l-.8 10.1A1.5 1.5 0 0 1 11.9 15H4.1a1.5 1.5 0 0 1-1.5-1.4L1.8 4H1a.5.5 0 0 1 0-1h3v-1a.5.5 0 0 1 .5-.5zm.5 1.5h4v-.5H6V3zm-2.4 2l.7 9a.5.5 0 0 0 .5.5h6.4a.5.5 0 0 0 .5-.5l.7-9H3.6zM6 6.5a.5.5 0 0 1 .5.5v5a.5.5 0 0 1-1 0v-5a.5.5 0 0 1 .5-.5zm4 0a.5.5 0 0 1 .5.5v5a.5.5 0 0 1-1 0v-5a.5.5 0 0 1 .5-.5z" />
    </svg>
  );
}

export function CloseIcon() {
  return (
    <svg className="icon close" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" aria-hidden="true">
      <path d="M4 4l8 8M12 4l-8 8" />
    </svg>
  );
}

export function CheckIcon() {
  return (
    <svg className="icon check" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <path d="M3.5 8.5l3 3 6-7" />
    </svg>
  );
}

export function RevealIcon() {
  return (
    <svg className="icon reveal" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <path d="M6 3h7v7M13 3L6 10M3 7v6h6" />
    </svg>
  );
}
