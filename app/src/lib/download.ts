import type { DownloadStatus } from '../ipc/bindings/DownloadStatus'
import type { EngineProgress } from '../ipc/bindings/EngineProgress'

/** `part` of `whole` as 0..1, or null for a whole not known yet (`0`). */
function fractionOf(part: number, whole: number): number | null {
	return whole > 0 ? Math.min(part / whole, 1) : null
}

/**
 * How far along a download is, 0..1, or null when there is nothing to say:
 * no download running, or one whose size pr-downloader has not stated yet
 * (the runtime reports `0/0` until it does).
 */
export function downloadFraction(status: DownloadStatus): number | null {
	if (status.state !== 'running') return null
	return fractionOf(status.current, status.total)
}

/**
 * How far the engine's download or unpacking is, 0..1, or null for a step
 * with no size: looking it up, or the moment before the archive says how
 * much it holds.
 */
export function engineFraction(progress: EngineProgress | null): number | null {
	switch (progress?.phase) {
		case 'downloading':
			return fractionOf(progress.got, progress.total)
		case 'extracting':
			return fractionOf(progress.done, progress.total)
		default:
			return null
	}
}

export const percent = (fraction: number) => `${Math.round(fraction * 100)}%`

/** Bytes as something a person reads, which for a download is whole MB. */
export function mb(bytes: number): string {
	return `${Math.round(bytes / 1_000_000)} MB`
}
