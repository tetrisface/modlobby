import { For, useContext } from 'solid-js'
import { pieces } from '../lib/search'
import { Query } from './SettingRow'

/** Text with what the search around it found marked; all of it plain with no search. */
export function Marked(props: { text: string }) {
	const query = useContext(Query)
	return (
		<For each={pieces(props.text, query())}>
			{(piece) => (piece.hit ? <mark>{piece.text}</mark> : piece.text)}
		</For>
	)
}
