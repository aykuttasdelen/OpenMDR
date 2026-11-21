import type { HistorySettings } from '../types/electron';

const MIN_MAX_VERSIONS = 5;
const MAX_MAX_VERSIONS = 200;
const DEFAULT_MAX_VERSIONS = 50;
const MIN_AUTOSAVE_INTERVAL_MINUTES = 1;
const MAX_AUTOSAVE_INTERVAL_MINUTES = 60;
const DEFAULT_AUTOSAVE_INTERVAL_MINUTES = 3;

/**
 * Formats a byte count into a human-readable string (B, KB, MB, GB).
 */
export function formatByteSize(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) {
    return '0 B';
  }
  if (bytes < 1024) {
    return `${Math.round(bytes)} B`;
  }
  const kb = bytes / 1024;
  if (kb < 1024) {
    return `${kb.toFixed(1)} KB`;
  }
  const mb = kb / 1024;
  if (mb < 1024) {
    return `${mb.toFixed(1)} MB`;
  }
  const gb = mb / 1024;
  return `${gb.toFixed(1)} GB`;
}

/**
 * Formats an epoch millisecond timestamp as a relative label or localized date/time.
 */
export function formatRelativeTimestamp(
  epochMs: number,
  lang: 'tr' | 'en',
  nowMs: number = Date.now()
): string {
  if (!Number.isFinite(epochMs)) {
    return '';
  }
  const safeNow = Number.isFinite(nowMs) ? nowMs : Date.now();
  const diffMs = Math.max(0, safeNow - epochMs);

  if (diffMs < 60_000) {
    return lang === 'tr' ? 'Az önce' : 'Just now';
  }
  if (diffMs < 3_600_000) {
    const minutes = Math.floor(diffMs / 60_000);
    return lang === 'tr' ? `${minutes} dk önce` : `${minutes} min ago`;
  }
  if (diffMs < 86_400_000) {
    const hours = Math.floor(diffMs / 3_600_000);
    return lang === 'tr' ? `${hours} sa önce` : `${hours}h ago`;
  }

  const date = new Date(epochMs);
  const nowDate = new Date(safeNow);
  const options: Intl.DateTimeFormatOptions = {
    day: '2-digit',
    month: 'short',
    hour: '2-digit',
    minute: '2-digit',
  };
  if (date.getFullYear() !== nowDate.getFullYear()) {
    options.year = 'numeric';
  }
  return date.toLocaleString(lang === 'tr' ? 'tr-TR' : 'en-US', options);
}

/**
 * Builds a tab title for opening a snapshot copy while preserving the file extension.
 */
export function buildCopyTabTitle(
  fileName: string,
  createdAtMs: number,
  lang: 'tr' | 'en'
): string {
  const baseName = fileName.trim() || (lang === 'tr' ? 'İsimsiz.md' : 'Untitled.md');
  const date = new Date(createdAtMs);
  const hh = String(date.getHours()).padStart(2, '0');
  const mm = String(date.getMinutes()).padStart(2, '0');
  const suffix = lang === 'tr' ? 'kopya' : 'copy';

  const lastDot = baseName.lastIndexOf('.');
  if (lastDot > 0) {
    const stem = baseName.slice(0, lastDot);
    const ext = baseName.slice(lastDot);
    return `${stem} (${hh}-${mm} ${suffix})${ext}`;
  }
  return `${baseName} (${hh}-${mm} ${suffix})`;
}

/**
 * Clamps and rounds version history settings to valid backend bounds.
 */
export function clampHistorySettings(input: HistorySettings): HistorySettings {
  const rawMax = Number.isFinite(input.maxVersionsPerFile)
    ? Math.round(input.maxVersionsPerFile)
    : DEFAULT_MAX_VERSIONS;
  const maxVersionsPerFile = Math.min(MAX_MAX_VERSIONS, Math.max(MIN_MAX_VERSIONS, rawMax));

  const rawInterval = Number.isFinite(input.autosaveIntervalMinutes)
    ? Math.round(input.autosaveIntervalMinutes)
    : DEFAULT_AUTOSAVE_INTERVAL_MINUTES;
  const autosaveIntervalMinutes = Math.min(
    MAX_AUTOSAVE_INTERVAL_MINUTES,
    Math.max(MIN_AUTOSAVE_INTERVAL_MINUTES, rawInterval)
  );

  return {
    enabled: Boolean(input.enabled),
    maxVersionsPerFile,
    autosaveIntervalMinutes,
  };
}
