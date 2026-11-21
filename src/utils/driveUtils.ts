import type {
  DriveRemoteFile,
  DriveSettings,
  DriveSyncState,
} from '../types/electron';

export type DriveStatusTone = 'muted' | 'warning' | 'success' | 'info' | 'danger';

const MIN_SYNC_INTERVAL_MINUTES = 1;
const MAX_SYNC_INTERVAL_MINUTES = 60;
const DEFAULT_SYNC_INTERVAL_MINUTES = 5;

const MIN_IDLE_DELAY_SECONDS = 10;
const MAX_IDLE_DELAY_SECONDS = 300;
const DEFAULT_IDLE_DELAY_SECONDS = 30;

const DRIVE_ERROR_CODE_KEYS: ReadonlyArray<readonly [string, string]> = [
  ['AUTH_CANCELLED', 'driveError.authCancelled'],
  ['AUTH_TIMEOUT', 'driveError.authTimeout'],
  ['AUTH_DENIED', 'driveError.authDenied'],
  ['AUTH_STATE_MISMATCH', 'driveError.authStateMismatch'],
  ['OAUTH_CLIENT_NOT_CONFIGURED', 'driveError.oauthNotConfigured'],
];

const DRIVE_STATUS_TONES: Record<DriveSyncState, DriveStatusTone> = {
  disconnected: 'muted',
  paused: 'warning',
  idle: 'success',
  syncing: 'info',
  pending: 'warning',
  offline: 'warning',
  auth_required: 'danger',
  error: 'danger',
};

function clampRoundedInt(value: number, min: number, max: number, fallback: number): number {
  const rounded = Number.isFinite(value) ? Math.round(value) : fallback;
  return Math.min(max, Math.max(min, rounded));
}

function matchesSearchTerm(
  haystack: string,
  queryLower: string,
  queryTrLower: string
): boolean {
  return (
    haystack.toLowerCase().includes(queryLower) ||
    haystack.toLocaleLowerCase('tr-TR').includes(queryTrLower)
  );
}

function extractErrorMessage(error: unknown): string {
  if (typeof error === 'string') {
    return error.trim();
  }
  if (error instanceof Error) {
    return error.message.trim();
  }
  if (
    error &&
    typeof error === 'object' &&
    'message' in error &&
    typeof (error as { message: unknown }).message === 'string'
  ) {
    return (error as { message: string }).message.trim();
  }
  if (error == null) {
    return '';
  }
  return String(error).trim();
}

/**
 * Clamps and rounds Google Drive sync settings to valid backend bounds.
 */
export function clampDriveSettings(input: DriveSettings): DriveSettings {
  return {
    enabled: Boolean(input.enabled),
    syncIntervalMinutes: clampRoundedInt(
      input.syncIntervalMinutes,
      MIN_SYNC_INTERVAL_MINUTES,
      MAX_SYNC_INTERVAL_MINUTES,
      DEFAULT_SYNC_INTERVAL_MINUTES
    ),
    idleDelaySeconds: clampRoundedInt(
      input.idleDelaySeconds,
      MIN_IDLE_DELAY_SECONDS,
      MAX_IDLE_DELAY_SECONDS,
      DEFAULT_IDLE_DELAY_SECONDS
    ),
  };
}

/**
 * Filters remote Google Drive files case-insensitively by filename or remote path while preserving order.
 */
export function filterRemoteFiles(
  files: DriveRemoteFile[],
  query: string
): DriveRemoteFile[] {
  const trimmed = query.trim();
  if (!trimmed) {
    return [...files];
  }

  const queryLower = trimmed.toLowerCase();
  const queryTrLower = trimmed.toLocaleLowerCase('tr-TR');

  return files.filter(
    (file) =>
      matchesSearchTerm(file.name, queryLower, queryTrLower) ||
      matchesSearchTerm(file.remotePath, queryLower, queryTrLower)
  );
}

/**
 * Maps stable backend Drive IPC error codes into localized UI error messages.
 */
export function mapDriveErrorCode(
  error: unknown,
  t: (key: string, params?: Record<string, string | number>) => string
): string {
  const message = extractErrorMessage(error);
  for (const [code, translationKey] of DRIVE_ERROR_CODE_KEYS) {
    if (message.includes(code)) {
      return t(translationKey);
    }
  }
  return message;
}

/**
 * Maps a Drive sync state to its semantic UI badge tone.
 */
export function getDriveStatusTone(state: DriveSyncState): DriveStatusTone {
  return DRIVE_STATUS_TONES[state] ?? 'muted';
}
