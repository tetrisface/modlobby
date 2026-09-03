import type { DownloadStatus } from '../ipc/bindings/DownloadStatus'

/**
 * How far along a download is, 0..1, or null when there is nothing to say:
 * no download running, or one whose size pr-downloader has not stated yet
 * (the runtime reports `0/0` until it does).
 */
export function downloadFraction(status: DownloadStatus): number | null {
  if (status.state !== 'running' || status.total <= 0) return null
  return Math.min(status.current / status.total, 1)
}
