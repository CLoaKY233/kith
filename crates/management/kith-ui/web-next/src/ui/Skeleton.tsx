export function Skeleton({ height = 16, width = "100%", className = "" }: { height?: number; width?: number | string; className?: string }) {
  return <span aria-hidden="true" className={`block max-w-full bg-ink-2 ${className}`} style={{ height, width }} />;
}
