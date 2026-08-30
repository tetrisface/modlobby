#!/usr/bin/env bun
/**
 * Drives the running app's webview over the Chrome DevTools Protocol.
 *
 * A lobby is only honest against a real server: a room's occupants, a vote
 * arriving, what a channel with seventeen hundred people in it does to the
 * layout. None of that is reachable from a test, and all of it has produced
 * bugs that the tests were happy with.
 *
 * Start the app with the debugger open, then talk to it:
 *
 *   WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222 bun run dev
 *   bun scripts/webview.ts eval "document.querySelectorAll('.battle-row').length"
 *   bun scripts/webview.ts shot /tmp/battles.png
 *
 * On Windows the variable is read by WebView2; on Linux and macOS the WebKit
 * inspector is opened by `--devtools` instead and this script does not apply.
 */

const PORT = process.env.MODLOBBY_DEBUG_PORT ?? '9222'
const TIMEOUT_MS = Number(process.env.MODLOBBY_DEBUG_TIMEOUT ?? 10_000)

type Target = { type: string; webSocketDebuggerUrl: string }

const targets: Target[] = await (
  await fetch(`http://127.0.0.1:${PORT}/json/list`)
).json()
const page = targets.find((target) => target.type === 'page')
if (!page) {
  console.error(
    `no page on port ${PORT}; is the app running with the debugger?`,
  )
  process.exit(1)
}

const socket = new WebSocket(page.webSocketDebuggerUrl)
const waiting = new Map<number, (message: Message) => void>()
let nextId = 0

type Message = { id?: number; result?: Record<string, unknown> }

socket.onmessage = (event) => {
  const message: Message = JSON.parse(String(event.data))
  const resolve =
    message.id === undefined ? undefined : waiting.get(message.id)
  if (message.id !== undefined && resolve) {
    waiting.delete(message.id)
    resolve(message)
  }
}
await new Promise((resolve) => (socket.onopen = resolve))

/**
 * One request, with a deadline.
 *
 * A navigation destroys the page's execution context, and an `evaluate` that
 * was in flight when that happens is never answered. Without a deadline this
 * script waits for that answer forever, is killed from outside, and leaves the
 * debugger session attached — after which every later run appears to hang and
 * the app looks wedged when it is perfectly idle.
 */
function send(method: string, params: unknown = {}): Promise<Message> {
  const id = (nextId += 1)
  return new Promise((resolve, reject) => {
    const giveUp = setTimeout(() => {
      waiting.delete(id)
      reject(new Error(`${method} went unanswered after ${TIMEOUT_MS}ms`))
    }, TIMEOUT_MS)
    waiting.set(id, (message) => {
      clearTimeout(giveUp)
      resolve(message)
    })
    socket.send(JSON.stringify({ id, method, params }))
  })
}

const [, , command, argument] = Bun.argv

// Whatever happens, hand the session back: an attached debugger that nobody
// is talking to is what makes the next run look like a hung app.
process.on('exit', () => socket.close())

try {
  if (command === 'eval') {
    const answer = await send('Runtime.evaluate', {
      expression: argument,
      returnByValue: true,
      // So an expression can be an async function call and still be waited for.
      awaitPromise: true,
    })
    const result = answer.result?.result as
      { value?: unknown; description?: string } | undefined
    console.log(
      JSON.stringify(result?.value ?? result?.description ?? null, null, 1),
    )
  } else if (command === 'shot') {
    const answer = await send('Page.captureScreenshot', { format: 'png' })
    const data = (answer.result?.data as string) ?? ''
    await Bun.write(argument!, Buffer.from(data, 'base64'))
    console.log(`wrote ${argument}`)
  } else {
    console.error('usage: webview.ts eval <expression> | shot <file.png>')
    process.exit(1)
  }
} catch (error) {
  console.error(String(error))
  process.exit(1)
}

socket.close()
