import { Show } from 'solid-js'
import { build } from '../store/build'
import { openExternal } from './Linkify'
import { Row } from './SettingRow'

/**
 * What this build is and where it comes from: the rows of Settings' About
 * section.
 *
 * The build is written out first, as version plus commit, since it is the one
 * fact a bug report is missing. The support row is drawn only when there is a
 * page to send to: a button that goes nowhere is noise.
 */
export function AboutRows(props: { repo: string; support: string | null }) {
	return (
		<>
			<Row>
				<Show when={build()}>
					{(found) => (
						<p class='build-stamp'>
							modlobby {found().version}+{found().commit}
						</p>
					)}
				</Show>
				<p class='muted'>
					The version, and the commit it was built from. A bug report wants
					both.
				</p>
			</Row>
			<Row>
				<p class='muted'>
					Free software under the MIT licence. The source, the releases and the
					issues live on GitHub.{' '}
					<button type='button' onClick={() => void openExternal(props.repo)}>
						Open on GitHub
					</button>
				</p>
			</Row>
			<Show when={props.support}>
				{(page) => (
					<Row>
						<p class='muted'>
							If modlobby is useful to you, a donation is welcome.{' '}
							<button type='button' onClick={() => void openExternal(page())}>
								Support modlobby
							</button>
						</p>
					</Row>
				)}
			</Show>
		</>
	)
}
