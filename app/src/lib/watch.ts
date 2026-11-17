import type { DownloadStatus } from '../ipc/bindings/DownloadStatus'
import type { EngineProgress } from '../ipc/bindings/EngineProgress'
import { downloadFraction, engineFraction, mb, percent } from './download'

/** One step of a Watch, as its chip shows it. */
export interface WatchStep {
	label: string
	/** How full the chip is, 0..1, or `null` for a step with no size. */
	fraction: number | null
	/** The step in full, for the tooltip. */
	title: string
}

/**
 * Where a Watch on a replay has got: its engine, then the game and map
 * pr-downloader fetches, then the start. `engine` is the engine download's
 * latest word, `null` until one starts; an engine already here only ever
 * says `done`.
 */
export function watchStep(
	version: string,
	engine: EngineProgress | null,
	download: DownloadStatus,
): WatchStep {
	switch (engine?.phase) {
		case 'finding':
			return {
				label: 'Engine',
				fraction: null,
				title: `Looking up engine ${version}`,
			}
		case 'downloading': {
			const fraction = engineFraction(engine)
			return {
				label: fraction === null ? 'Engine' : `Engine ${percent(fraction)}`,
				fraction,
				title: `Engine ${version}: ${mb(engine.got)} of ${mb(engine.total)}`,
			}
		}
		case 'extracting': {
			const fraction = engineFraction(engine)
			return {
				label:
					fraction === null ? 'Unpacking' : `Unpacking ${percent(fraction)}`,
				fraction,
				title: `Unpacking engine ${version}`,
			}
		}
	}
	if (download.state === 'running') {
		const fraction = downloadFraction(download)
		return {
			label: fraction === null ? 'Content' : `Content ${percent(fraction)}`,
			fraction,
			title: `Downloading ${download.what}`,
		}
	}
	return { label: 'Starting', fraction: null, title: 'Starting the replay' }
}
