import { cleanup, render, waitFor } from '@solidjs/testing-library'
import { invoke } from '@tauri-apps/api/core'
import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest'
import type { NewsItem } from '../ipc/bindings/NewsItem'

vi.mock('@tauri-apps/api/core', async () => ({
  invoke: vi.fn(),
  convertFileSrc: (path: string, protocol: string) =>
    `http://${protocol}.localhost/${encodeURIComponent(path)}`,
}))

const asked = vi.mocked(invoke)

function story(over: Partial<NewsItem> = {}): NewsItem {
  return {
    id: 'https://www.beyondallreason.info/news/hooded-horse',
    title: 'A new chapter',
    summary: 'BAR is going professional.',
    link: 'https://www.beyondallreason.info/news/hooded-horse',
    publishedAt: 1782334310,
    image: 'https://cdn.example/thumb.webp',
    ...over,
  }
}

function serve(items: NewsItem[], unread = 0) {
  asked.mockImplementation(async (command: string) => {
    switch (command) {
      case 'news':
        return { items, unread }
      case 'mark_news_read':
        return undefined
      default:
        throw new Error(`unexpected ${command}`)
    }
  })
}

/**
 * The store behind the page holds one feed per page, so the view is imported
 * per test rather than once: a module kept between them would answer the next
 * test with the last one's news.
 */
async function fresh() {
  vi.resetModules()
  return (await import('./News')).News
}

beforeEach(() => {
  asked.mockReset()
  serve([story()])
})

afterEach(cleanup)

describe('the news page', () => {
  test('a story shows its headline, its date and its banner', async () => {
    const News = await fresh()
    const { container } = render(() => <News />)

    await waitFor(() =>
      expect(container.querySelector('.news-card')).not.toBeNull(),
    )
    expect(container.querySelector('.news-text h2')?.textContent).toBe(
      'A new chapter',
    )
    expect(container.querySelector('.news-when')?.textContent).not.toBe('')
    // The banner is asked of the app, never of the CDN the picture is on.
    expect(container.querySelector('.news-banner')?.getAttribute('src')).toBe(
      'http://thumb.localhost/news%2F320x180%2Fhttps%3A%2F%2Fwww.beyondallreason.info%2Fnews%2Fhooded-horse',
    )
  })

  test('a story with no picture still shows the rest of itself', async () => {
    serve([story({ image: null })])
    const News = await fresh()
    const { container } = render(() => <News />)

    await waitFor(() =>
      expect(container.querySelector('.news-card')).not.toBeNull(),
    )
    expect(container.querySelector('.news-banner')).toBeNull()
    expect(container.querySelector('.news-text h2')?.textContent).toBe(
      'A new chapter',
    )
  })

  test('a story with no readable date shows the rest of itself', async () => {
    serve([story({ publishedAt: null })])
    const News = await fresh()
    const { container } = render(() => <News />)

    await waitFor(() =>
      expect(container.querySelector('.news-card')).not.toBeNull(),
    )
    expect(container.querySelector('.news-when')).toBeNull()
  })

  test('an empty feed says so rather than showing nothing', async () => {
    serve([])
    const News = await fresh()
    const { container } = render(() => <News />)

    await waitFor(() =>
      expect(container.querySelector('.muted')?.textContent).toContain(
        'No news right now',
      ),
    )
  })

  test('opening the page is what clears the count', async () => {
    serve([story()], 3)
    const News = await fresh()
    render(() => <News />)

    await waitFor(() => expect(asked).toHaveBeenCalledWith('mark_news_read'))
  })
})
