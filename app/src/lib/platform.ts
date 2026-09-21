/**
 * Whether this is a Mac, where Cmd does what Ctrl does elsewhere. Every
 * binding in the app takes either (`ctrlKey || metaKey`, Monaco's `CtrlCmd`);
 * this is for saying so the way the platform writes it.
 */
export const IS_MAC = /Macintosh|Mac OS X/.test(navigator.userAgent)

/** The key a shortcut is held with, for prose: `⌘` on a Mac, `Ctrl` elsewhere. */
export const MOD = IS_MAC ? '⌘' : 'Ctrl'

/**
 * A shortcut as this platform writes it: `shortcut('Shift+F')` is
 * `Ctrl+Shift+F`, or `⇧⌘F` on a Mac, with its modifiers in Apple's order.
 */
export function shortcut(keys: string, mac = IS_MAC): string {
	if (!mac) return `Ctrl+${keys}`
	const parts = keys.split('+')
	const key = parts.pop() ?? ''
	const shift = parts.includes('Shift') ? '⇧' : ''
	return `${shift}⌘${key === 'Enter' ? '↩' : key}`
}
