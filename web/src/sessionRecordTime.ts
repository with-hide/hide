/** Calendar dates in the viewer's timezone for request headings and stamps. */
export function recordDate(at: number): string {
  const date = new Date(at);
  if (!Number.isFinite(date.getTime())) throw new Error("invalid_record_date");
  return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, "0")}-${String(date.getDate()).padStart(2, "0")}`;
}
