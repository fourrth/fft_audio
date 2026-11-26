/**
 * Data-table extensions read by the shipping desktop and headless binaries.
 *
 * Keep this in the same order as `fft_core::table::TABLE_EXTENSIONS`. The
 * browser client cannot query the compiled feature set, and both shipping
 * binaries enable `tables-text`, `tables-parquet`, and `tables-arrow`.
 */
export const TABLE_EXTENSIONS = [
  "csv",
  "tsv",
  "tab",
  "txt",
  "dat",
  "parquet",
  "pq",
  "json",
  "ndjson",
  "jsonl",
  "arrow",
  "ipc",
  "feather",
] as const;

const AUDIO_EXTENSIONS = [
  "wav",
  "flac",
  "mp3",
  "ogg",
  "aac",
  "m4a",
  "opus",
  "caf",
] as const;

/** True when a path names a supported data-table file. */
export function isTablePath(path: string): boolean {
  const extension = path.split(".").pop()?.toLowerCase() ?? "";
  return TABLE_EXTENSIONS.some((value) => value === extension);
}

/** `accept` attribute for the browser audio/table upload picker. */
export const UPLOAD_ACCEPT = [
  "audio/*",
  ...AUDIO_EXTENSIONS.map((extension) => `.${extension}`),
  ...TABLE_EXTENSIONS.map((extension) => `.${extension}`),
].join(",");
