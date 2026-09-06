import { beforeEach, describe, expect, test, vi } from 'vitest'
import type { NewsFeed } from '../ipc/bindings/NewsFeed'

const { news, markNewsRead } = vi.hoisted(() => ({
  news: vi.fn(),
  markNewsRead: vi.fn(),
}))
vi.mock('../ipc/client', () => ({
  api: {
    news: () => news(),
    markNewsRead: () => markNewsRead(),
  },
}))
// What Tauri's own helper does on Windows; the Rust side reads the same path
// whichever way the scheme is spelled.
vi.mock('@tauri-apps/api/core', () => ({
  convertFileSrc: (path: string, protocol: string) =>
    `http://${protocol}.localhost/${encodeURIComponent(path)}`,
}))

/** The module holds one feed per page; each test wants a page of its own. */
async function fresh() {
  vi.resetModules()
  return import('./news')
}

function feed(unread: number): NewsFeed {
  return {
    unread,
    items: [
      {
        id: 'https://www.beyondallreason.info/news/hooded-horse',
        title: 'A new chapter',
        summary: 'BAR is going professional.',
        link: 'https://www.beyondallreason.info/news/hooded-horse',
        publishedAt: 1782334310,
        image: 'https://cdn.example/thumb.webp',
      },
    ],
  }
}

beforeEach(() => {
  news.mockReset()
  news.mockResolvedValue(feed(2))
  markNewsRead.mockReset()
  markNewsRead.mockResolvedValue(undefined)
})

describe('the news the nav counts', () => {
  test('asks Rust once however many callers arrive together', async () => {
    const store = await fresh()

    await Promise.all([store.loadNews(), store.loadNews(), store.loadNews()])

    expect(news).toHaveBeenCalledTimes(1)
    expect(store.unreadNews()).toBe(2)
  })

  test('stays quiet and empty when Rust cannot be reached', async () => {
    news.mockRejectedValue(new Error('no'))
    const store = await fresh()

    await store.loadNews()

    expect(store.news()).toBeNull()
    expect(store.unreadNews()).toBe(0)
    // The next caller asks again rather than replaying the failure.
    news.mockResolvedValue(feed(1))
    await store.loadNews()
    expect(store.unreadNews()).toBe(1)
  })

  test('marks read only after the list has loaded', async () => {
    const store = await fresh()

    await store.markNewsRead()

    expect(news).toHaveBeenCalled()
    expect(markNewsRead).toHaveBeenCalledTimes(1)
    expect(store.unreadNews()).toBe(0)
  })

  test('with nothing unread there is nothing to write', async () => {
    news.mockResolvedValue(feed(0))
    const store = await fresh()

    await store.markNewsRead()

    expect(markNewsRead).not.toHaveBeenCalled()
  })

  test('the badge stays cleared when the feed is asked for again', async () => {
    const store = await fresh()
    await store.markNewsRead()

    // Rust has the marks now, so it answers with nothing unread; the held
    // promise from before the mark must not put the old count back.
    news.mockResolvedValue(feed(0))
    await store.loadNews()

    expect(store.unreadNews()).toBe(0)
  })

  test('a banner is asked of Rust by the story that published it', async () => {
    const store = await fresh()
    const src = store.newsBanner('https://x/news/a')

    // The whole permalink, encoded, behind the size the card draws.
    expect(decodeURIComponent(src!)).toBe(
      'http://thumb.localhost/news/320x180/https://x/news/a',
    )
    expect(store.newsBanner('')).toBeNull()
  })
})
